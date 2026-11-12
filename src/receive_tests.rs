//! Disposable memory fixtures only; no actual clipboard/credential access.
use crate::{
    auth::BearerToken,
    clipboard::{ClipboardStore, ReadValue, UnavailableReason},
    http::{self, ApiState},
    image::{self, Budget, Options},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
    response::Response,
};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const VERSION: &str = "x-winios-clipboard-version";
fn store() -> ClipboardStore {
    ClipboardStore::with_images(Options {
        enabled: true,
        ..Default::default()
    })
}
fn state(store: ClipboardStore) -> ApiState {
    ApiState {
        store,
        token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
        sequence: || Some(7),
        writes: None,
    }
}
fn publish(store: &ClipboardStore, value: ReadValue) {
    let mut observation = store.begin_observation(Some(7));
    store.complete(&mut observation, value).unwrap();
}
fn image() -> Arc<image::ClipboardImage> {
    image::encode_png(&image::fixture(), Options::default(), &Budget::new()).unwrap()
}
async fn call(app: &Router, path: &str, authenticated: bool, version: Option<&str>) -> Response {
    let mut request = Request::builder()
        .uri(path)
        .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()));
    if authenticated {
        request = request.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    }
    if let Some(version) = version {
        request = request.header(VERSION, version);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    response
}
async fn metadata(app: &Router, kind: &str) -> String {
    let response = call(app, "/v1/clipboard/type", true, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let bytes = to_bytes(response.into_body(), 256).await.unwrap();
    let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(data.as_object().unwrap().len(), 2);
    assert_eq!(data["kind"], kind);
    assert!(!String::from_utf8_lossy(&bytes).contains(TOKEN));
    let version = data["version"].as_str().unwrap();
    assert_eq!(version.len(), 52);
    version.to_owned()
}

#[tokio::test]
async fn smart_receive_text_image_empty_and_unsupported_are_content_free_and_bound() {
    let store = store();
    let app = http::router(state(store.clone()));
    let text = "DO NOT INCLUDE THIS TEXT IN METADATA 😀 café\r\n";
    publish(&store, ReadValue::Text(text.into()));
    let text_version = metadata(&app, "text").await;
    assert_eq!(
        call(&app, "/v1/clipboard", false, Some(&text_version))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = call(&app, "/v1/clipboard", true, Some(&text_version)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), 1024).await.unwrap()[..],
        text.as_bytes()
    );
    let png = image();
    publish(&store, ReadValue::Image(png.clone()));
    let image_version = metadata(&app, "image").await;
    assert_ne!(text_version, image_version);
    assert_eq!(
        call(&app, "/v1/clipboard", true, Some(&text_version))
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    let response = call(&app, "/v1/clipboard/image", true, Some(&image_version)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), 1024).await.unwrap()[..],
        &png.png()[..]
    );
    publish(&store, ReadValue::Text("new text".into()));
    assert_eq!(
        call(&app, "/v1/clipboard/image", true, Some(&image_version))
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    // Legacy requests retain the previous contracts, without implicit negotiation.
    assert_eq!(
        call(&app, "/v1/clipboard/image", true, None).await.status(),
        StatusCode::NO_CONTENT
    );
    publish(&store, ReadValue::Empty);
    let empty = metadata(&app, "empty").await;
    assert_eq!(
        call(&app, "/v1/clipboard", true, Some(&empty))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    publish(
        &store,
        ReadValue::Unavailable(UnavailableReason::UnsupportedFormat),
    );
    metadata(&app, "empty").await;
}

#[tokio::test]
async fn smart_receive_auth_pause_readiness_and_peer_boundary_match_clipboard_routes() {
    let store = store();
    publish(&store, ReadValue::Text("fixture".into()));
    let sharing = crate::sharing::SharingControl::default();
    let gate = crate::tailnet::Gate::default();
    gate.set_for_test(true);
    let app = http::controlled_router(state(store), Some(gate.clone()), sharing.clone());
    for paused in [false, true] {
        sharing.set_paused(paused);
        for ready in [false, true] {
            gate.set_for_test(ready);
            assert_eq!(
                call(&app, "/v1/clipboard/type", false, None).await.status(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                call(&app, "/v1/clipboard/type", true, None).await.status(),
                if paused || !ready {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::OK
                }
            );
        }
    }
    for peer in ["8.8.8.8:1", "192.168.1.7:1"] {
        let request = Request::builder()
            .uri("/v1/clipboard/type")
            .extension(ConnectInfo(peer.parse::<SocketAddr>().unwrap()))
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("x-forwarded-for", "127.0.0.1")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let request = Request::builder()
        .uri("/v1/clipboard/type")
        .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn smart_receive_pending_invalidated_disabled_and_unknown_sequence_fail_closed() {
    let store = store();
    let app = http::router(state(store.clone()));
    assert_eq!(
        call(&app, "/v1/clipboard/type", true, None).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    publish(&store, ReadValue::Text("old fixture".into()));
    let version = metadata(&app, "text").await;
    for reason in [
        UnavailableReason::Pending,
        UnavailableReason::TemporaryFailure,
        UnavailableReason::MalformedData,
        UnavailableReason::Oversized,
        UnavailableReason::PrivateContent,
        UnavailableReason::RetriesExhausted,
        UnavailableReason::WatcherStopped,
    ] {
        publish(&store, ReadValue::Unavailable(reason));
        assert_eq!(
            call(&app, "/v1/clipboard/type", true, None).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        for path in ["/v1/clipboard", "/v1/clipboard/image"] {
            let response = call(&app, path, true, Some(&version)).await;
            assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }
    publish(&store, ReadValue::Text("current fixture".into()));
    for sequence in [|| None, || Some(0), || Some(8)] {
        let app = http::router(ApiState {
            sequence,
            ..state(store.clone())
        });
        assert_eq!(
            call(&app, "/v1/clipboard/type", true, None).await.status(),
            if sequence() == Some(8) {
                StatusCode::CONFLICT
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            }
        );
    }
    let disabled = ClipboardStore::default();
    publish(&disabled, ReadValue::Image(image()));
    let app = http::router(state(disabled));
    let version = metadata(&app, "empty").await;
    assert_eq!(
        call(&app, "/v1/clipboard/image", true, Some(&version))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn smart_receive_rapid_changes_identical_recopies_and_server_restart_cannot_reuse_version() {
    let store = store();
    let app = http::router(state(store.clone()));
    let png = image();
    for turn in 0..64 {
        publish(&store, ReadValue::Text("same fixture".into()));
        let old = metadata(&app, "text").await;
        publish(
            &store,
            if turn % 2 == 0 {
                ReadValue::Image(png.clone())
            } else {
                ReadValue::Text("same fixture".into())
            },
        );
        assert_eq!(
            call(&app, "/v1/clipboard", true, Some(&old)).await.status(),
            StatusCode::PRECONDITION_FAILED
        );
    }
    publish(&store, ReadValue::Text("same fixture".into()));
    let before = metadata(&app, "text").await;
    let restarted = http::router(state(store));
    let after = metadata(&restarted, "text").await;
    assert_ne!(before, after);
    assert_eq!(
        call(&restarted, "/v1/clipboard", true, Some(&before))
            .await
            .status(),
        StatusCode::PRECONDITION_FAILED
    );
    for invalid in [
        "",
        "*",
        "w1-short",
        "w1-00000000000000000000000000000000-000000000000000g",
    ] {
        assert_eq!(
            call(&restarted, "/v1/clipboard", true, Some(invalid))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let request = Request::builder()
        .uri("/v1/clipboard")
        .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(VERSION, &after)
        .header(VERSION, &after)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        restarted.oneshot(request).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn smart_receive_valid_ticket_still_refuses_native_change_before_watcher_observation() {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQUENCE: AtomicU32 = AtomicU32::new(7);
    fn sequence() -> Option<u32> {
        Some(SEQUENCE.load(Ordering::SeqCst))
    }
    for image_turn in [false, true] {
        SEQUENCE.store(7, Ordering::SeqCst);
        let store = store();
        publish(
            &store,
            if image_turn {
                ReadValue::Image(image())
            } else {
                ReadValue::Text("disposable native-lag fixture".into())
            },
        );
        let app = http::router(ApiState {
            sequence,
            ..state(store.clone())
        });
        let version = metadata(&app, if image_turn { "image" } else { "text" }).await;
        for native in [8, 0] {
            SEQUENCE.store(native, Ordering::SeqCst);
            let response = call(
                &app,
                if image_turn {
                    "/v1/clipboard/image"
                } else {
                    "/v1/clipboard"
                },
                true,
                Some(&version),
            )
            .await;
            assert_eq!(
                response.status(),
                if native == 0 {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::CONFLICT
                }
            );
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(store.image_downloads.available_permits(), 2);
    }
}
