//! Image API/state integration using disposable in-memory fixtures and test tokens.
use crate::{
    auth::BearerToken,
    clipboard::{ClipboardState, ClipboardStore, Origin, ReadValue, UnavailableReason, Update},
    http::{self, ApiState},
    image::{self, Budget, Options},
    writes::{WriteClient, WriteError, WriteReceipt},
};
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
    response::Response,
};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
fn options() -> Options {
    Options {
        enabled: true,
        max_mib: 2,
    }
}
fn fixture() -> Arc<image::ClipboardImage> {
    image::encode_png(&image::fixture(), options(), &Budget::new()).unwrap()
}
fn large_fixture() -> Arc<image::ClipboardImage> {
    let mut seed = 1u32;
    let rgba = (0..256 * 256 * 4)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        })
        .collect();
    image::encode_png(
        &image::Decoded {
            width: 256,
            height: 256,
            rgba,
        },
        options(),
        &Budget::new(),
    )
    .unwrap()
}

#[tokio::test]
async fn pause_releases_stalled_download_capacity_before_deadline_and_discards_queued_chunk() {
    let sharing = crate::sharing::SharingControl::default();
    let store = ClipboardStore::with_images(options());
    publish(&store, ReadValue::Image(large_fixture()));
    let app = http::controlled_router(state(store.clone(), None), None, sharing.clone());
    let response = request(app, "GET", "/v1/clipboard/image", true, "", "").await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    sharing.set_paused(true);
    tokio::time::timeout(std::time::Duration::from_millis(200), async {
        while store.image_downloads.available_permits() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Pause should release a stalled producer immediately, not wait ten seconds");
    assert!(
        to_bytes(response.into_body(), options().max_bytes())
            .await
            .is_err(),
        "Queued chunks must not be delivered after pause"
    );
}
fn state(store: ClipboardStore, writes: Option<WriteClient>) -> ApiState {
    ApiState {
        store,
        token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
        sequence: || Some(7),
        writes,
    }
}
fn publish(store: &ClipboardStore, value: ReadValue) -> Update {
    let mut observation = store.begin_observation(Some(7));
    store.complete(&mut observation, value).unwrap()
}

#[tokio::test]
async fn network_loss_cancels_admitted_upload_and_cannot_revive_after_reconnect() {
    let gate = crate::tailnet::Gate::default();
    gate.set_for_test(true);
    let store = ClipboardStore::with_images(options());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    let app = http::tailnet_router(
        state(store, Some(WriteClient::new(sender, || true))),
        gate.clone(),
    );
    let task = tokio::spawn(request(
        app,
        "POST",
        "/v1/clipboard/image",
        true,
        "image/png",
        fixture().png(),
    ));
    let command = receiver.recv().await.unwrap();
    gate.close();
    gate.set_for_test(true);
    assert!(
        command.expired(),
        "A queued write must synchronously observe lost readiness, even before the HTTP task wakes"
    );
    let response = tokio::time::timeout(std::time::Duration::from_millis(200), task)
        .await
        .expect("Lost readiness must promptly cancel the request")
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn pause_resume_invalidates_native_command_before_http_task_is_rescheduled() {
    let sharing = crate::sharing::SharingControl::default();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    let app = http::controlled_router(
        state(
            ClipboardStore::with_images(options()),
            Some(WriteClient::new(sender, || true)),
        ),
        None,
        sharing.clone(),
    );
    let task = tokio::spawn(request(
        app,
        "POST",
        "/v1/clipboard/image",
        true,
        "image/png",
        fixture().png(),
    ));
    let command = receiver.recv().await.unwrap();
    sharing.set_paused(true);
    sharing.set_paused(false);
    assert!(
        command.expired(),
        "Pause must invalidate native commands without waiting for the async handler to run"
    );
    assert_eq!(
        task.await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[test]
fn rapid_interleaved_observations_never_publish_superseded_text_or_images() {
    let store = ClipboardStore::with_images(options());
    let image = fixture();
    for seq in 0..256u32 {
        let native = Some(u32::MAX.wrapping_sub(seq));
        let mut old = store.begin_observation(native);
        let mut latest = store.begin_observation(native.map(|s| s.wrapping_add(1)));
        let image_turn = seq % 2 == 0;
        store
            .complete(
                &mut latest,
                if image_turn {
                    ReadValue::Image(image.clone())
                } else {
                    ReadValue::Text("Unicode 😀 café\r\n".into())
                },
            )
            .unwrap();
        let version = store.snapshot().version;
        for value in [
            ReadValue::Text("superseded fixture".into()),
            ReadValue::Image(image.clone()),
            ReadValue::Unavailable(UnavailableReason::RetriesExhausted),
            ReadValue::Empty,
        ] {
            assert_eq!(store.complete(&mut old, value).unwrap(), Update::Superseded);
            assert_eq!(store.snapshot().version, version);
        }
        assert_eq!(
            matches!(store.snapshot().state, ClipboardState::Image(_)),
            image_turn
        );
    }
}

#[tokio::test]
async fn interrupted_body_and_network_stream_release_capacity_and_require_fresh_request() {
    let gate = crate::tailnet::Gate::default();
    gate.set_for_test(true);
    let store = ClipboardStore::with_images(options());
    publish(&store, ReadValue::Image(large_fixture()));
    let app = http::tailnet_router(state(store.clone(), None), gate.clone());
    let old = request(app.clone(), "GET", "/v1/clipboard/image", true, "", "").await;
    tokio::task::yield_now().await;
    gate.close();
    gate.set_for_test(true);
    assert!(
        to_bytes(old.into_body(), options().max_bytes())
            .await
            .is_err()
    );
    tokio::task::yield_now().await;
    assert_eq!(store.image_downloads.available_permits(), 2);
    let fresh = request(app.clone(), "GET", "/v1/clipboard/image", true, "", "").await;
    assert_eq!(fresh.status(), StatusCode::OK);
    assert!(
        to_bytes(fresh.into_body(), options().max_bytes())
            .await
            .is_ok()
    );
    let abandoned = request(app, "GET", "/v1/clipboard/image", true, "", "").await;
    drop(abandoned);
    tokio::task::yield_now().await;
    assert_eq!(store.image_downloads.available_permits(), 2);
}
async fn request(
    app: Router,
    method: &str,
    path: &str,
    auth: bool,
    content_type: &str,
    body: impl Into<Body>,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
        .header(header::CONTENT_TYPE, content_type);
    if auth {
        request = request.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    }
    app.oneshot(request.body(body.into()).unwrap())
        .await
        .unwrap()
}
#[test]
fn image_state_dedup_transient_failure_remote_origin_and_cross_type_invalidation() {
    let store = ClipboardStore::with_images(options());
    let image = fixture();
    assert_eq!(
        publish(&store, ReadValue::Image(image.clone())),
        Update::Changed
    );
    let mut observation = store.begin_observation(Some(7));
    for _ in 0..3 {
        store
            .complete(
                &mut observation,
                ReadValue::Unavailable(UnavailableReason::TemporaryFailure),
            )
            .unwrap();
        assert!(matches!(
            store.snapshot().state,
            ClipboardState::Unavailable(_)
        ));
    }
    assert_eq!(
        store
            .complete(&mut observation, ReadValue::Image(image.clone()))
            .unwrap(),
        Update::Duplicate
    );
    let mut stale = store.begin_observation(Some(7));
    publish(&store, ReadValue::Text("Unicode 😀 café\r\n".into()));
    assert_eq!(
        store
            .complete(&mut stale, ReadValue::Image(image.clone()))
            .unwrap(),
        Update::Superseded
    );
    assert!(matches!(
        store.snapshot().state,
        ClipboardState::Available(_)
    ));
    assert_eq!(
        publish(&store, ReadValue::Image(image.clone())),
        Update::Changed
    );
    publish(&store, ReadValue::Empty);
    assert_eq!(
        publish(&store, ReadValue::Image(image.clone())),
        Update::Changed
    );
    for reason in [
        UnavailableReason::MalformedData,
        UnavailableReason::Oversized,
        UnavailableReason::PrivateContent,
        UnavailableReason::RetriesExhausted,
    ] {
        publish(&store, ReadValue::Unavailable(reason));
        assert_eq!(
            publish(&store, ReadValue::Image(image.clone())),
            Update::Changed
        );
    }
    let mut remote = store.begin_remote_observation(Some(7));
    assert_eq!(
        store
            .complete(&mut remote, ReadValue::Image(image.clone()))
            .unwrap(),
        Update::Remote
    );
    assert_eq!(store.snapshot().origin, Origin::Iphone);
    assert!(store.is_remote_sequence(Some(7)));
    assert_eq!(publish(&store, ReadValue::Image(image)), Update::Changed);
}
#[tokio::test]
async fn image_get_auth_states_sequence_pause_and_text_contract() {
    let store = ClipboardStore::with_images(options());
    let image = fixture();
    publish(&store, ReadValue::Image(image.clone()));
    let sharing = crate::sharing::SharingControl::default();
    let gate = crate::tailnet::Gate::default();
    gate.set_for_test(true);
    let app = http::controlled_router(
        state(store.clone(), None),
        Some(gate.clone()),
        sharing.clone(),
    );
    for method in ["GET", "POST"] {
        assert_eq!(
            request(
                app.clone(),
                method,
                "/v1/clipboard/image",
                false,
                "image/png",
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let response = request(app.clone(), "GET", "/v1/clipboard/image", true, "", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        to_bytes(response.into_body(), image::MAX_PNG_BYTES)
            .await
            .unwrap(),
        image.png()
    );
    assert_eq!(
        request(app.clone(), "GET", "/v1/clipboard", true, "", "")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    sharing.set_paused(true);
    for method in ["GET", "POST"] {
        assert_eq!(
            request(
                app.clone(),
                method,
                "/v1/clipboard/image",
                true,
                "image/png",
                ""
            )
            .await
            .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            request(
                app.clone(),
                method,
                "/v1/clipboard/image",
                false,
                "image/png",
                ""
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    sharing.set_paused(false);
    gate.close();
    assert_eq!(
        request(app.clone(), "GET", "/v1/clipboard/image", true, "", "")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    gate.set_for_test(true);
    let mut stale = state(store.clone(), None);
    stale.sequence = || Some(8);
    assert_eq!(
        request(
            http::router(stale),
            "GET",
            "/v1/clipboard/image",
            true,
            "",
            ""
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let mut unknown = state(store.clone(), None);
    unknown.sequence = || None;
    assert_eq!(
        request(
            http::router(unknown),
            "GET",
            "/v1/clipboard/image",
            true,
            "",
            ""
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    publish(&store, ReadValue::Empty);
    assert_eq!(
        request(app.clone(), "GET", "/v1/clipboard/image", true, "", "")
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    publish(&store, ReadValue::Text("Unicode 😀 café\r\n".into()));
    assert_eq!(
        request(app.clone(), "GET", "/v1/clipboard/image", true, "", "")
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        to_bytes(
            request(app.clone(), "GET", "/v1/clipboard", true, "", "")
                .await
                .into_body(),
            1024
        )
        .await
        .unwrap(),
        "Unicode 😀 café\r\n"
    );
    for (reason, status) in [
        (UnavailableReason::Pending, 503),
        (UnavailableReason::PrivateContent, 503),
        (UnavailableReason::Oversized, 413),
        (UnavailableReason::MalformedData, 503),
    ] {
        publish(&store, ReadValue::Unavailable(reason));
        assert_eq!(
            request(app.clone(), "GET", "/v1/clipboard/image", true, "", "")
                .await
                .status()
                .as_u16(),
            status
        );
    }
    let disabled = http::router(state(ClipboardStore::default(), None));
    assert_eq!(
        request(disabled, "GET", "/v1/clipboard/image", true, "", "")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn image_post_validation_write_permission_and_acknowledgement() {
    let store = ClipboardStore::with_images(options());
    let image = fixture();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(crate::writes::QUEUE_CAPACITY);
    let app = http::router(state(
        store.clone(),
        Some(WriteClient::new(sender, || true)),
    ));
    for (content_type, body, status) in [
        ("application/json", Bytes::from_static(b"{}"), 415),
        ("image/jpeg", image.png(), 415),
        ("image/png", Bytes::from_static(b"malformed PNG"), 422),
        (
            "image/png",
            Bytes::from(vec![0; options().max_bytes() + 1]),
            413,
        ),
    ] {
        assert_eq!(
            request(
                app.clone(),
                "POST",
                "/v1/clipboard/image",
                true,
                content_type,
                body
            )
            .await
            .status()
            .as_u16(),
            status
        );
    }
    assert!(receiver.try_recv().is_err());
    assert_eq!(
        request(
            http::router(state(store.clone(), None)),
            "POST",
            "/v1/clipboard/image",
            true,
            "image/png",
            image.png()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let task = tokio::spawn(request(
        app.clone(),
        "POST",
        "/v1/clipboard/image",
        true,
        "image/png",
        image.png(),
    ));
    let command = receiver.recv().await.unwrap();
    assert!(!task.is_finished());
    let prepared = command.image.as_ref().unwrap();
    assert_eq!(
        image::decode_dib(&prepared.dib_v5, &Budget::new())
            .unwrap()
            .rgba,
        image::fixture().rgba
    );
    command
        .ack
        .send(Ok(WriteReceipt {
            operation: 42,
            sequence: Some(7),
        }))
        .unwrap();
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let receipt = to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(std::str::from_utf8(&receipt).unwrap().contains("42"));
}
#[tokio::test]
async fn image_post_failures_timeout_queue_bound_and_pause_cancellation() {
    let store = ClipboardStore::with_images(options());
    let image = fixture();
    let sharing = crate::sharing::SharingControl::default();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    let app = http::controlled_router(
        state(store.clone(), Some(WriteClient::new(sender, || true))),
        None,
        sharing.clone(),
    );
    let first = tokio::spawn(request(
        app.clone(),
        "POST",
        "/v1/clipboard/image",
        true,
        "image/png",
        image.png(),
    ));
    let command = receiver.recv().await.unwrap();
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/v1/clipboard/image",
            true,
            "image/png",
            image.png()
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    sharing.set_paused(true);
    assert_eq!(
        first.await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(command.expired());
    drop(command);
    sharing.set_paused(false);
    for (error, status) in [
        (WriteError::Busy, 503),
        (WriteError::Failed, 500),
        (WriteError::Unavailable, 503),
        (WriteError::Timeout, 504),
    ] {
        let task = tokio::spawn(request(
            app.clone(),
            "POST",
            "/v1/clipboard/image",
            true,
            "image/png",
            image.png(),
        ));
        let command = receiver.recv().await.unwrap();
        command.ack.send(Err(error)).unwrap();
        assert_eq!(task.await.unwrap().status().as_u16(), status);
    }
    let task = tokio::spawn(request(
        app,
        "POST",
        "/v1/clipboard/image",
        true,
        "image/png",
        image.png(),
    ));
    let command = receiver.recv().await.unwrap();
    assert_eq!(task.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(command.expired());
}
#[tokio::test]
async fn image_download_capacity_and_real_peer_enforcement() {
    let store = ClipboardStore::with_images(options());
    publish(&store, ReadValue::Image(fixture()));
    let permits = store
        .image_downloads
        .clone()
        .acquire_many_owned(2)
        .await
        .unwrap();
    let app = http::router(state(store, None));
    assert_eq!(
        request(app.clone(), "GET", "/v1/clipboard/image", true, "", "")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(permits);
    let request = Request::builder()
        .uri("/v1/clipboard/image")
        .extension(ConnectInfo("8.8.8.8:1".parse::<SocketAddr>().unwrap()))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header("x-forwarded-for", "127.0.0.1")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn image_body_timeout_and_disconnected_upload_release_capacity() {
    struct PendingBody;
    impl futures_core::Stream for PendingBody {
        type Item = Result<Bytes, std::io::Error>;
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            std::task::Poll::Pending
        }
    }
    let store = ClipboardStore::with_images(options());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    let app = http::router(state(
        store.clone(),
        Some(WriteClient::new(sender, || true)),
    ));
    let task = tokio::spawn(request(
        app.clone(),
        "POST",
        "/v1/clipboard/image",
        true,
        "image/png",
        Body::from_stream(PendingBody),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while store.image_uploads.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    assert_eq!(store.image_uploads.available_permits(), 1);
    let response = request(
        app,
        "POST",
        "/v1/clipboard/image",
        true,
        "image/png",
        Body::from_stream(PendingBody),
    )
    .await;
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(store.image_uploads.available_permits(), 1);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn changing_clipboard_cancels_a_backpressured_image_download() {
    let store = ClipboardStore::with_images(options());
    let mut seed = 1u32;
    let rgba = (0..256 * 256 * 4)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        })
        .collect();
    let image = image::encode_png(
        &image::Decoded {
            width: 256,
            height: 256,
            rgba,
        },
        options(),
        &Budget::new(),
    )
    .unwrap();
    let expected = image.len();
    assert!(expected > 3 * 64 * 1024);
    publish(&store, ReadValue::Image(image));
    let response = request(
        http::router(state(store.clone(), None)),
        "GET",
        "/v1/clipboard/image",
        true,
        "",
        "",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    publish(&store, ReadValue::Empty);
    assert!(
        to_bytes(response.into_body(), options().max_bytes())
            .await
            .is_err(),
        "A changed observation must abort the old body, including queued chunks"
    );
    tokio::task::yield_now().await;
    assert_eq!(store.image_downloads.available_permits(), 2);
}
