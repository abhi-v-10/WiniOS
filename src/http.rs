use crate::{
    auth::BearerToken,
    clipboard::{ClipboardState, ClipboardStore, MAX_TEXT_BYTES},
};
use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Request, State},
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
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/clipboard", get(clipboard))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{ReadValue, UnavailableReason};
    use axum::{body::to_bytes, http::Request};
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    fn app(value: ReadValue, observed: Option<u32>, sequence: fn() -> Option<u32>) -> Router {
        let store = ClipboardStore::default();
        let mut observation = store.begin_observation(observed);
        store.complete(&mut observation, value).unwrap();
        router(ApiState {
            store,
            token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
            sequence,
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
