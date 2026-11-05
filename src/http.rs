use crate::{
    auth::BearerToken,
    clipboard::{ClipboardState, ClipboardStore, MAX_TEXT_BYTES},
    writes::{MAX_JSON_BYTES, WriteClient, WriteError, WriteReceipt},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, FromRequest, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

#[derive(Clone)]
pub struct ApiState {
    pub store: ClipboardStore,
    pub token: Arc<BearerToken>,
    /// Production uses GetClipboardSequenceNumber; tests inject a deterministic source.
    pub sequence: fn() -> Option<u32>,
    pub writes: Option<WriteClient>,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route(
            "/v1/clipboard",
            get(clipboard)
                .post(write_clipboard)
                .layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .with_state(state)
        .layer(middleware::from_fn(local_only))
}

fn trusted_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        _ => false,
    }
}

async fn local_only(request: Request, next: Next) -> Response {
    let allowed = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| trusted_peer(peer.0.ip()));
    let mut response = if allowed {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn health() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"service":"winios-agent","status":"ok"}"#,
    )
}

async fn clipboard(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if !state.token.authorizes(&headers) {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"winios\""),
        );
        return response;
    }
    let snapshot = state.store.snapshot();
    let version = snapshot.version;
    let expected = snapshot.sequence;
    let response = match snapshot.state {
        ClipboardState::Available(text) if text.text.len() <= MAX_TEXT_BYTES => Response::builder()
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(text.text))
            .expect("static response headers"),
        ClipboardState::Available(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        ClipboardState::Empty => StatusCode::NO_CONTENT.into_response(),
        ClipboardState::Unavailable(crate::clipboard::UnavailableReason::Oversized) => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        ClipboardState::Unavailable(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    // Build the body first, then validate store revision and the native sequence
    // immediately before handing it to Axum. Never serve the private baseline.
    if !state.store.is_current(version) {
        return StatusCode::CONFLICT.into_response();
    }
    match (expected, (state.sequence)()) {
        (Some(expected), Some(current)) if expected == current && current != 0 => response,
        (None | Some(0), _) | (_, None | Some(0)) => {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        _ => StatusCode::CONFLICT.into_response(),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WritePayload {
    text: String,
}

#[derive(serde::Serialize)]
struct WriteResponse {
    status: &'static str,
    #[serde(flatten)]
    receipt: WriteReceipt,
}

async fn write_clipboard(State(state): State<ApiState>, request: Request) -> Response {
    // Authenticate before reading/deserializing the body. Rejection messages
    // are deliberately discarded: serde diagnostics can include user input.
    if !state.token.authorizes(request.headers()) {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"winios\""),
        );
        return response;
    }
    let Some(writer) = &state.writes else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let payload = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        Json::<WritePayload>::from_request(request, &state),
    )
    .await
    {
        Ok(Ok(Json(payload))) => payload,
        Ok(Err(rejection)) => return rejection.status().into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    match writer.write(payload.text).await {
        Ok(receipt) => Json(WriteResponse {
            status: "ok",
            receipt,
        })
        .into_response(),
        Err(error) => match error {
            WriteError::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
            WriteError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            WriteError::Busy | WriteError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            WriteError::Failed => StatusCode::INTERNAL_SERVER_ERROR,
            WriteError::Timeout => StatusCode::GATEWAY_TIMEOUT,
        }
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{ReadValue, UnavailableReason};
    use axum::{body::to_bytes, http::Request};
    use tower::ServiceExt;

    fn write_app(
        enabled: bool,
    ) -> (
        Router,
        tokio::sync::mpsc::Receiver<crate::writes::WriteCommand>,
    ) {
        let (sender, receiver) = tokio::sync::mpsc::channel(crate::writes::QUEUE_CAPACITY);
        (
            router(ApiState {
                store: ClipboardStore::default(),
                token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
                sequence: || Some(7),
                writes: enabled.then(|| WriteClient::new(sender, || true)),
            }),
            receiver,
        )
    }

    async fn post(
        app: Router,
        auth: Option<&str>,
        content_type: Option<&str>,
        body: String,
    ) -> Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/clipboard")
            .extension(ConnectInfo(
                "192.168.1.25:12345".parse::<SocketAddr>().unwrap(),
            ));
        if let Some(auth) = auth {
            request = request.header(header::AUTHORIZATION, auth);
        }
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        app.oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn post_auth_and_feature_gate_precede_body_parsing() {
        let (app, mut receiver) = write_app(true);
        for auth in [None, Some("Bearer wrong")] {
            assert_eq!(
                post(app.clone(), auth, None, "not JSON".into())
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert!(receiver.try_recv().is_err());
        let (disabled, _) = write_app(false);
        assert_eq!(
            post(
                disabled,
                Some(&bearer()),
                Some("application/json"),
                r#"{"text":"disposable"}"#.into()
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn post_validates_json_content_type_text_and_bounded_body() {
        let (app, mut receiver) = write_app(true);
        for (ct, body, expected) in [
            (
                None,
                r#"{"text":"disposable"}"#.into(),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
            (
                Some("text/plain"),
                "disposable".into(),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
            (
                Some("application/json"),
                "{".into(),
                StatusCode::BAD_REQUEST,
            ),
            (
                Some("application/json"),
                r#"{"text":7}"#.into(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Some("application/json"),
                r#"{}"#.into(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Some("application/json"),
                r#"{"text":"a\u0000b"}"#.into(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Some("application/json"),
                r#"{"text":"x","extra":true}"#.into(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Some("application/json"),
                format!("{{\"text\":\"{}\"}}", "a".repeat(MAX_TEXT_BYTES + 1)),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                Some("application/json"),
                " ".repeat(MAX_JSON_BYTES + 1),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ] {
            let response = post(app.clone(), Some(&bearer()), ct, body).await;
            assert_eq!(response.status(), expected);
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn post_only_succeeds_after_ack_and_maps_worker_errors() {
        let (app, mut receiver) = write_app(true);
        let request_app = app.clone();
        let request = tokio::spawn(async move {
            post(
                request_app,
                Some(&bearer()),
                Some("application/json"),
                r#"{"text":"disposable 😀"}"#.into(),
            )
            .await
        });
        let command = receiver.recv().await.unwrap();
        assert_eq!(command.text, "disposable 😀");
        assert!(!request.is_finished());
        command
            .ack
            .send(Ok(WriteReceipt {
                operation: 12,
                sequence: Some(8),
            }))
            .ok();
        let response = request.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            br#"{"status":"ok","operation":12,"sequence":8}"#
        );
        for (error, status) in [
            (WriteError::Busy, StatusCode::SERVICE_UNAVAILABLE),
            (WriteError::Failed, StatusCode::INTERNAL_SERVER_ERROR),
            (WriteError::Timeout, StatusCode::GATEWAY_TIMEOUT),
            (WriteError::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let app = app.clone();
            let request = tokio::spawn(async move {
                post(
                    app,
                    Some(&bearer()),
                    Some("application/json"),
                    r#"{"text":"disposable"}"#.into(),
                )
                .await
            });
            receiver.recv().await.unwrap().ack.send(Err(error)).ok();
            assert_eq!(request.await.unwrap().status(), status);
        }
    }

    #[tokio::test]
    async fn local_tcp_post_round_trip_through_axum_and_command_ack() {
        use std::io::{Read, Write};
        let (app, mut receiver) = write_app(true);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let worker = tokio::spawn(async move {
            let command = receiver.recv().await.unwrap();
            assert_eq!(command.text, "disposable wire test");
            command
                .ack
                .send(Ok(WriteReceipt {
                    operation: 19,
                    sequence: Some(20),
                }))
                .ok();
        });
        let response = tokio::task::spawn_blocking(move || {
            let mut stream = std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(3)).unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
            let body = r#"{"text":"disposable wire test"}"#;
            write!(stream, "POST /v1/clipboard HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        }).await.unwrap();
        worker.await.unwrap();
        server.abort();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"operation\":19"));
        assert!(!response.contains("disposable wire test"));
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    fn app(value: ReadValue, observed: Option<u32>, sequence: fn() -> Option<u32>) -> Router {
        let store = ClipboardStore::default();
        let mut observation = store.begin_observation(observed);
        store.complete(&mut observation, value).unwrap();
        router(ApiState {
            store,
            token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
            sequence,
            writes: None,
        })
    }
    async fn call(app: Router, auth: Option<&str>, path: &str) -> Response {
        let mut request = Request::builder().uri(path).extension(ConnectInfo(
            "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
        ));
        if let Some(auth) = auth {
            request = request.header(header::AUTHORIZATION, auth);
        }
        app.oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
    fn available() -> Router {
        app(ReadValue::Text("Hello 😀".into()), Some(7), || Some(7))
    }
    fn bearer() -> String {
        format!("Bearer {TOKEN}")
    }

    #[tokio::test]
    async fn private_dedup_baseline_is_never_served_after_invalidation() {
        let store = ClipboardStore::default();
        let mut accepted = store.begin_observation(Some(7));
        store
            .complete(&mut accepted, ReadValue::Text("private baseline".into()))
            .unwrap();
        let old_version = store.snapshot().version;
        let mut pending = store.begin_observation(Some(8));
        assert!(!store.is_current(old_version));
        let router = router(ApiState {
            store: store.clone(),
            token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
            sequence: || Some(8),
            writes: None,
        });
        for reason in [
            UnavailableReason::Pending,
            UnavailableReason::TemporaryFailure,
            UnavailableReason::RetriesExhausted,
            UnavailableReason::WatcherStopped,
        ] {
            store
                .complete(&mut pending, ReadValue::Unavailable(reason))
                .unwrap();
            let response = call(router.clone(), Some(&bearer()), "/v1/clipboard").await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        store.complete(&mut pending, ReadValue::Empty).unwrap();
        let response = call(router, Some(&bearer()), "/v1/clipboard").await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn exact_one_mib_utf8_is_served_and_oversized_rejection_returns_413() {
        let exact = "界".repeat(MAX_TEXT_BYTES / 3) + "a";
        assert_eq!(exact.len(), MAX_TEXT_BYTES);
        let response = call(
            app(ReadValue::Text(exact), Some(7), || Some(7)),
            Some(&bearer()),
            "/v1/clipboard",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), MAX_TEXT_BYTES)
                .await
                .unwrap()
                .len(),
            MAX_TEXT_BYTES
        );
        let store = ClipboardStore::default();
        let mut observation = store.begin_observation(Some(7));
        assert!(
            store
                .complete(
                    &mut observation,
                    ReadValue::Text("x".repeat(MAX_TEXT_BYTES + 1))
                )
                .is_err()
        );
        let router = router(ApiState {
            store,
            token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
            sequence: || Some(7),
            writes: None,
        });
        let response = call(router, Some(&bearer()), "/v1/clipboard").await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(
            to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn rejects_missing_wrong_malformed_and_duplicate_auth() {
        for auth in [
            None,
            Some("Bearer wrong"),
            Some(TOKEN),
            Some("Basic wrong"),
            Some("Bearer ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
        ] {
            let response = call(available(), auth, "/v1/clipboard").await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(response.headers().contains_key(header::WWW_AUTHENTICATE));
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        let request = Request::builder()
            .uri("/v1/clipboard")
            .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
            .header(header::AUTHORIZATION, bearer())
            .header(header::AUTHORIZATION, bearer())
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            available().oneshot(request).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn available_returns_exact_utf8_and_empty_string_is_200() {
        let response = call(available(), Some(&bearer()), "/v1/clipboard").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            to_bytes(response.into_body(), MAX_TEXT_BYTES)
                .await
                .unwrap()
                .as_ref(),
            "Hello 😀".as_bytes()
        );
        assert_eq!(
            call(
                app(ReadValue::Text(String::new()), Some(7), || Some(7)),
                Some(&bearer()),
                "/v1/clipboard"
            )
            .await
            .status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn empty_unavailable_and_oversized_never_return_text() {
        for (value, status) in [
            (ReadValue::Empty, StatusCode::NO_CONTENT),
            (
                ReadValue::Unavailable(UnavailableReason::Pending),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                ReadValue::Unavailable(UnavailableReason::TemporaryFailure),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                ReadValue::Unavailable(UnavailableReason::UnsupportedFormat),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                ReadValue::Unavailable(UnavailableReason::MalformedData),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                ReadValue::Unavailable(UnavailableReason::RetriesExhausted),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                ReadValue::Unavailable(UnavailableReason::Oversized),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ] {
            let response = call(
                app(value, Some(7), || Some(7)),
                Some(&bearer()),
                "/v1/clipboard",
            )
            .await;
            assert_eq!(response.status(), status);
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn refuses_stale_or_unknown_sequences_including_wrap() {
        for (observed, current, status) in [
            (
                Some(7),
                (|| Some(8)) as fn() -> Option<u32>,
                StatusCode::CONFLICT,
            ),
            (Some(u32::MAX), || Some(1), StatusCode::CONFLICT),
            (Some(7), || None, StatusCode::SERVICE_UNAVAILABLE),
            (Some(7), || Some(0), StatusCode::SERVICE_UNAVAILABLE),
            (None, || Some(7), StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = call(
                app(ReadValue::Text("must not escape".into()), observed, current),
                Some(&bearer()),
                "/v1/clipboard",
            )
            .await;
            assert_eq!(response.status(), status);
            assert!(
                to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn health_is_public_and_public_network_peers_are_rejected() {
        let response = call(available(), None, "/v1/health").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            br#"{"service":"winios-agent","status":"ok"}"#
        );
        let request = Request::builder()
            .uri("/v1/clipboard")
            .extension(ConnectInfo("8.8.8.8:1234".parse::<SocketAddr>().unwrap()))
            .header(header::AUTHORIZATION, bearer())
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            available().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}
