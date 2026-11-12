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

#[cfg(test)]
pub fn router(state: ApiState) -> Router {
    controlled_router(state, None, crate::sharing::SharingControl::default())
}

#[cfg(test)]
pub fn tailnet_router(state: ApiState, gate: crate::tailnet::Gate) -> Router {
    controlled_router(state, Some(gate), crate::sharing::SharingControl::default())
}

#[derive(Clone)]
struct Boundary {
    sharing: crate::sharing::SharingControl,
    gate: Option<crate::tailnet::Gate>,
    token: Arc<BearerToken>,
    snapshot_scope: Option<Arc<str>>,
}

pub fn controlled_router(
    state: ApiState,
    gate: Option<crate::tailnet::Gate>,
    sharing: crate::sharing::SharingControl,
) -> Router {
    let boundary = Boundary {
        sharing,
        gate,
        token: state.token.clone(),
        snapshot_scope: snapshot_scope(),
    };
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/clipboard/type", get(clipboard_type))
        .route(
            "/v1/clipboard",
            get(clipboard)
                .post(write_clipboard)
                .layer(DefaultBodyLimit::max(MAX_JSON_BYTES)),
        )
        .route(
            "/v1/clipboard/image",
            get(clipboard_image)
                .post(write_image)
                .layer(DefaultBodyLimit::max(crate::image::MAX_PNG_BYTES)),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(boundary, local_only))
}

fn trusted_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        _ => false,
    }
}

async fn local_only(
    State(boundary): State<Boundary>,
    mut request: Request,
    next: Next,
) -> Response {
    let allowed = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| {
            if boundary.gate.is_some() {
                peer.0.ip() == IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            } else {
                trusted_peer(peer.0.ip())
            }
        });
    let mut response = if allowed {
        request.extensions_mut().insert(boundary.clone());
        let guard =
            crate::writes::RequestGuard::new(boundary.sharing.clone(), boundary.gate.clone());
        request.extensions_mut().insert(guard.clone());
        let authenticated_clipboard = matches!(
            request.uri().path(),
            "/v1/clipboard" | "/v1/clipboard/image" | "/v1/clipboard/type"
        ) && boundary.token.authorizes(request.headers());
        // Headers cannot confer trust. Preserve authentication before readiness
        // errors for clipboard routes; the public health response is metadata.
        if authenticated_clipboard && boundary.sharing.is_paused() {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        } else if boundary.gate.as_ref().is_some_and(|gate| !gate.ready())
            && (request.uri().path() == "/v1/health"
                || boundary.token.authorizes(request.headers()))
        {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        } else if authenticated_clipboard {
            tokio::select! {
                biased;
                _ = guard.invalidated() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                response = next.run(request) => response,
            }
        } else {
            next.run(request).await
        }
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

const SNAPSHOT_HEADER: &str = "x-winios-clipboard-version";
fn snapshot_scope() -> Option<Arc<str>> {
    #[cfg(windows)]
    {
        let bytes = crate::native::snapshot_namespace().ok()?;
        Some(
            bytes
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
                .into(),
        )
    }
    #[cfg(not(windows))]
    {
        None
    } // The service requires Windows; never invent an RNG fallback.
}
fn snapshot_version(boundary: &Boundary, version: u64) -> Option<String> {
    boundary
        .snapshot_scope
        .as_ref()
        .map(|scope| format!("w1-{scope}-{version:016x}"))
}
fn bound_fetch(headers: &HeaderMap, boundary: &Boundary, version: u64) -> Result<(), StatusCode> {
    let mut values = headers.get_all(SNAPSHOT_HEADER).iter();
    let Some(value) = values.next() else {
        return Ok(());
    };
    let value = value.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
    if values.next().is_some()
        || value.len() != 52
        || !value.starts_with("w1-")
        || value.as_bytes()[35] != b'-'
        || !value.as_bytes()[3..35]
            .iter()
            .chain(&value.as_bytes()[36..])
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let current = snapshot_version(boundary, version).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if value != current {
        return Err(StatusCode::PRECONDITION_FAILED);
    }
    Ok(())
}
#[derive(serde::Serialize)]
struct ClipboardType {
    kind: &'static str,
    version: String,
}
async fn clipboard_type(
    State(state): State<ApiState>,
    axum::Extension(boundary): axum::Extension<Boundary>,
    headers: HeaderMap,
) -> Response {
    if !state.token.authorizes(&headers) {
        return unauthorized();
    }
    let snapshot = state.store.snapshot();
    let kind = match &snapshot.state {
        ClipboardState::Available(_) => "text",
        ClipboardState::Image(_) if !state.store.image_options().enabled => {
            return StatusCode::FORBIDDEN.into_response();
        }
        ClipboardState::Image(_) => "image",
        ClipboardState::Empty
        | ClipboardState::Unavailable(crate::clipboard::UnavailableReason::UnsupportedFormat) => {
            "empty"
        }
        ClipboardState::Unavailable(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(version) = snapshot_version(&boundary, snapshot.version) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    current_response(
        &state,
        snapshot.version,
        snapshot.sequence,
        Json(ClipboardType { kind, version }).into_response(),
    )
}

async fn clipboard(
    State(state): State<ApiState>,
    axum::Extension(boundary): axum::Extension<Boundary>,
    headers: HeaderMap,
) -> Response {
    if !state.token.authorizes(&headers) {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"winios\""),
        );
        return response;
    }
    let snapshot = state.store.snapshot();
    if let Err(status) = bound_fetch(&headers, &boundary, snapshot.version) {
        return status.into_response();
    }
    let version = snapshot.version;
    let expected = snapshot.sequence;
    let response = match snapshot.state {
        ClipboardState::Available(text) if text.text.len() <= MAX_TEXT_BYTES => Response::builder()
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(text.text))
            .expect("static response headers"),
        ClipboardState::Available(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        ClipboardState::Image(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        ClipboardState::Empty => StatusCode::NO_CONTENT.into_response(),
        ClipboardState::Unavailable(crate::clipboard::UnavailableReason::Oversized) => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        ClipboardState::Unavailable(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    // Build the body first, then validate store revision and the native sequence
    // immediately before handing it to Axum. Never serve the private baseline.
    current_response(&state, version, expected, response)
}
fn current_response(
    state: &ApiState,
    version: u64,
    expected: Option<u32>,
    response: Response,
) -> Response {
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

struct ImageStream {
    receiver: tokio::sync::mpsc::Receiver<Result<axum::body::Bytes, std::io::Error>>,
    revision: tokio::sync::watch::Receiver<u64>,
    version: u64,
    sequence: fn() -> Option<u32>,
    expected: Option<u32>,
    guard: crate::writes::RequestGuard,
    deadline: tokio::time::Instant,
    stopped: bool,
}
impl futures_core::Stream for ImageStream {
    type Item = Result<axum::body::Bytes, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.stopped {
            return std::task::Poll::Ready(None);
        }
        if !self.guard.valid()
            || *self.revision.borrow() != self.version
            || (self.sequence)() != self.expected
            || tokio::time::Instant::now() >= self.deadline
        {
            self.receiver.close();
            while self.receiver.try_recv().is_ok() {}
            self.stopped = true;
            return std::task::Poll::Ready(Some(Err(std::io::ErrorKind::ConnectionAborted.into())));
        }
        self.receiver.poll_recv(context)
    }
}
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"winios\"")],
    )
        .into_response()
}
async fn clipboard_image(
    State(state): State<ApiState>,
    axum::Extension(guard): axum::Extension<crate::writes::RequestGuard>,
    axum::Extension(boundary): axum::Extension<Boundary>,
    headers: HeaderMap,
) -> Response {
    if !state.token.authorizes(&headers) {
        return unauthorized();
    }
    let options = state.store.image_options();
    if !options.enabled {
        return StatusCode::FORBIDDEN.into_response();
    }
    let snapshot = state.store.snapshot();
    if let Err(status) = bound_fetch(&headers, &boundary, snapshot.version) {
        return status.into_response();
    }
    let response = match snapshot.state {
        ClipboardState::Image(image) if image.len() <= options.max_bytes() => {
            let checked = current_response(
                &state,
                snapshot.version,
                snapshot.sequence,
                StatusCode::OK.into_response(),
            );
            if checked.status() != StatusCode::OK {
                return checked;
            }
            let permit = match state.store.image_downloads.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            };
            let length = image.len();
            let bytes = image.png();
            let store = state.store.clone();
            let version = snapshot.version;
            let sequence = state.sequence;
            let expected = snapshot.sequence;
            let revision = state.store.revisions();
            let stream_guard = guard.clone();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            tokio::spawn(async move {
                let _permit = permit;
                let cancelled = guard.invalidated();
                let changed = store.changed_since(version);
                tokio::pin!(cancelled, changed);
                let current = || {
                    tokio::time::Instant::now() < deadline
                        && store.is_current(version)
                        && (sequence)() == expected
                        && guard.valid()
                };
                for chunk in bytes.chunks(64 * 1024) {
                    if !current() {
                        break;
                    }
                    // Copy bounded chunks: a stalled socket cannot retain the entire
                    // previous image after this producer's absolute deadline expires.
                    let slot = tokio::select! {biased;
                        _=&mut cancelled=>break,
                        _=&mut changed=>break,
                        slot=tokio::time::timeout_at(deadline,tx.reserve())=>match slot {Ok(Ok(slot))=>slot,_=>break},
                    };
                    // State may have changed while a slow consumer held capacity.
                    if !current() {
                        break;
                    }
                    slot.send(Ok(axum::body::Bytes::copy_from_slice(chunk)));
                }
                // Truncation on timeout/change deliberately fails Content-Length;
                // clients must reject incomplete PNGs instead of accepting stale data.
            });
            Response::builder()
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CONTENT_LENGTH, length)
                .body(Body::from_stream(ImageStream {
                    receiver: rx,
                    revision,
                    version,
                    sequence,
                    expected,
                    guard: stream_guard,
                    deadline,
                    stopped: false,
                }))
                .expect("static image response")
        }
        ClipboardState::Image(_)
        | ClipboardState::Unavailable(crate::clipboard::UnavailableReason::Oversized) => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        ClipboardState::Empty | ClipboardState::Available(_) => {
            StatusCode::NO_CONTENT.into_response()
        }
        ClipboardState::Unavailable(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    current_response(&state, snapshot.version, snapshot.sequence, response)
}
async fn write_image(
    State(state): State<ApiState>,
    axum::Extension(guard): axum::Extension<crate::writes::RequestGuard>,
    request: Request,
) -> Response {
    if !state.token.authorizes(request.headers()) {
        return unauthorized();
    }
    let options = state.store.image_options();
    let Some(writer) = &state.writes else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !options.enabled {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request
        .headers()
        .get_all(header::CONTENT_TYPE)
        .iter()
        .count()
        != 1
        || request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            != Some("image/png")
        || request.headers().contains_key(header::CONTENT_ENCODING)
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    // Acquire before receiving bytes; cancelled work keeps its permit until its
    // worker/queued command is actually dropped. Images cannot flood the queue.
    let permit = match state.store.image_uploads.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let bytes = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        axum::body::to_bytes(request.into_body(), options.max_bytes()),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    let budget = crate::image::Budget::new();
    let _cancel = crate::image::CancelBudget(budget.clone());
    let task = tokio::task::spawn_blocking(move || {
        crate::image::prepare_png(&bytes, options, &budget).map(|image| (image, permit))
    });
    let (image, permit) = match tokio::time::timeout(
        crate::image::CODEC_TIMEOUT + std::time::Duration::from_millis(250),
        task,
    )
    .await
    {
        Ok(Ok(Ok(value))) => value,
        Ok(Ok(Err(crate::image::ImageError::TooLarge))) => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        Ok(Ok(Err(crate::image::ImageError::Invalid))) => {
            return StatusCode::UNPROCESSABLE_ENTITY.into_response();
        }
        Ok(Ok(Err(crate::image::ImageError::Timeout))) | Err(_) => {
            return StatusCode::GATEWAY_TIMEOUT.into_response();
        }
        Ok(Err(_)) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match writer.guarded(guard).write_image(image, permit).await {
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

async fn write_clipboard(
    State(state): State<ApiState>,
    axum::Extension(guard): axum::Extension<crate::writes::RequestGuard>,
    request: Request,
) -> Response {
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
    match writer.guarded(guard).write(payload.text).await {
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

    async fn sharing_request(app: Router, method: &str, auth: Option<&str>) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri("/v1/clipboard")
            .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(auth) = auth {
            request = request.header(header::AUTHORIZATION, auth);
        }
        app.oneshot(
            request
                .body(Body::from(r#"{"text":"disposable paused test"}"#))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn pause_blocks_authenticated_get_post_but_preserves_auth_health_and_tailnet_gate() {
        for tailnet in [false, true] {
            let control = crate::sharing::SharingControl::default();
            let gate = crate::tailnet::Gate::default();
            gate.set_for_test(true);
            let store = ClipboardStore::default();
            let mut observation = store.begin_observation(Some(7));
            store
                .complete(&mut observation, ReadValue::Text("disposable".into()))
                .unwrap();
            let (sender, mut receiver) = tokio::sync::mpsc::channel(crate::writes::QUEUE_CAPACITY);
            let app = controlled_router(
                ApiState {
                    store,
                    token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
                    sequence: || Some(7),
                    writes: Some(WriteClient::new(sender, || true)),
                },
                tailnet.then(|| gate.clone()),
                control.clone(),
            );
            control.set_paused(true);
            for method in ["GET", "POST"] {
                for auth in [None, Some("Bearer wrong")] {
                    let response = sharing_request(app.clone(), method, auth).await;
                    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                    assert!(response.headers().contains_key(header::WWW_AUTHENTICATE));
                }
                let response = sharing_request(app.clone(), method, Some(&bearer())).await;
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                assert!(
                    to_bytes(response.into_body(), 1024)
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
            assert!(receiver.try_recv().is_err());
            assert_eq!(
                call(app.clone(), None, "/v1/health").await.status(),
                StatusCode::OK
            );
            control.set_paused(false);
            assert_eq!(
                sharing_request(app.clone(), "GET", Some(&bearer()))
                    .await
                    .status(),
                StatusCode::OK
            );
            let task = tokio::spawn(sharing_request(app.clone(), "POST", Some(TOKEN)));
            assert_eq!(task.await.unwrap().status(), StatusCode::UNAUTHORIZED);
            if tailnet {
                gate.close();
                assert_eq!(
                    sharing_request(app, "GET", Some(&bearer())).await.status(),
                    StatusCode::SERVICE_UNAVAILABLE
                );
            }
        }
    }

    #[tokio::test]
    async fn pause_cancels_waiting_write_ack_and_resume_requires_fresh_ack() {
        let control = crate::sharing::SharingControl::default();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(crate::writes::QUEUE_CAPACITY);
        let app = controlled_router(
            ApiState {
                store: ClipboardStore::default(),
                token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
                sequence: || Some(7),
                writes: Some(WriteClient::new(sender, || true)),
            },
            None,
            control.clone(),
        );
        let task_app = app.clone();
        let task =
            tokio::spawn(async move { sharing_request(task_app, "POST", Some(&bearer())).await });
        let command = receiver.recv().await.unwrap();
        assert!(!command.expired());
        control.set_paused(true);
        let response = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(command.expired());
        control.set_paused(false);
        let task = tokio::spawn(async move { sharing_request(app, "POST", Some(&bearer())).await });
        let command = receiver.recv().await.unwrap();
        assert!(!task.is_finished());
        command
            .ack
            .send(Ok(WriteReceipt {
                operation: 123,
                sequence: Some(7),
            }))
            .ok();
        assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn resume_does_not_enable_disabled_writes() {
        let control = crate::sharing::SharingControl::default();
        let app = controlled_router(
            ApiState {
                store: ClipboardStore::default(),
                token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
                sequence: || Some(7),
                writes: None,
            },
            None,
            control.clone(),
        );
        control.set_paused(true);
        assert_eq!(
            sharing_request(app.clone(), "POST", Some(&bearer()))
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        control.set_paused(false);
        assert_eq!(
            sharing_request(app, "POST", Some(&bearer())).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn tailnet_gate_requires_auth_loopback_and_fresh_readiness_ignores_headers() {
        let gate = crate::tailnet::Gate::default();
        let store = ClipboardStore::default();
        let mut observation = store.begin_observation(Some(7));
        store
            .complete(&mut observation, ReadValue::Text("disposable".into()))
            .unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(crate::writes::QUEUE_CAPACITY);
        let app = tailnet_router(
            ApiState {
                store,
                token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
                sequence: || Some(7),
                writes: Some(WriteClient::new(sender, || true)),
            },
            gate.clone(),
        );
        async fn request(
            app: Router,
            peer: &str,
            method: &str,
            auth: bool,
            health: bool,
        ) -> Response {
            let mut builder = Request::builder()
                .method(method)
                .uri(if health {
                    "/v1/health"
                } else {
                    "/v1/clipboard"
                })
                .extension(ConnectInfo(peer.parse::<SocketAddr>().unwrap()))
                .header("host", "untrusted.example")
                .header("x-forwarded-for", "127.0.0.1")
                .header("tailscale-user-login", "injected@example.invalid")
                .header("content-type", "application/json");
            if auth {
                builder = builder.header(header::AUTHORIZATION, bearer());
            }
            app.oneshot(
                builder
                    .body(Body::from(r#"{"text":"disposable"}"#))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
        for method in ["GET", "POST"] {
            assert_eq!(
                request(app.clone(), "127.0.0.1:1", method, false, false)
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                request(app.clone(), "127.0.0.1:1", method, true, false)
                    .await
                    .status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        assert_eq!(
            request(app.clone(), "127.0.0.1:1", "GET", false, true)
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(receiver.try_recv().is_err());
        gate.set_for_test(true);
        for peer in [
            "192.168.1.5:1",
            "100.64.1.2:1",
            "8.8.8.8:1",
            "[::1]:1",
            "127.0.0.2:1",
        ] {
            for method in ["GET", "POST"] {
                assert_eq!(
                    request(app.clone(), peer, method, true, false)
                        .await
                        .status(),
                    StatusCode::FORBIDDEN
                );
            }
        }
        for method in ["GET", "POST"] {
            assert_eq!(
                request(app.clone(), "127.0.0.1:1", method, false, false)
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let response = request(app.clone(), "127.0.0.1:1", "GET", true, false).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let post_app = app.clone();
        let post = tokio::spawn(request(post_app, "127.0.0.1:1", "POST", true, false));
        let command = receiver.recv().await.unwrap();
        assert!(!post.is_finished());
        command
            .ack
            .send(Ok(WriteReceipt {
                operation: 99,
                sequence: Some(7),
            }))
            .ok();
        assert_eq!(post.await.unwrap().status(), StatusCode::OK);
        gate.close();
        assert_eq!(
            request(app, "127.0.0.1:1", "GET", true, false)
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

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
            (
                ReadValue::Unavailable(UnavailableReason::PrivateContent),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
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
