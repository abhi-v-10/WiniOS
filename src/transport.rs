//! Only the TCP/TLS boundary. Routing, auth and clipboard code are transport independent.
use axum::Router;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use std::{
    future::Future,
    io,
    net::{SocketAddr, TcpListener},
    path::Path,
    pin::Pin,
    time::Duration,
};
use tokio::sync::oneshot;

pub fn install_crypto_provider() {
    // A single explicit provider avoids feature-unification ambiguity.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub async fn load_identity(cert: &Path, key: &Path) -> io::Result<RustlsConfig> {
    install_crypto_provider();
    RustlsConfig::from_pem_file(cert, key).await.map_err(|_| io::Error::new(
        io::ErrorKind::InvalidInput,
        "TLS identity could not be loaded (missing/invalid PEM, unsupported or mismatched key). Check certificate setup; HTTP fallback is disabled.",
    ))
}

pub async fn serve(
    listener: TcpListener,
    app: Router,
    tls: Option<RustlsConfig>,
    stop: oneshot::Receiver<()>,
) -> io::Result<()> {
    let handle = axum_server::Handle::new();
    // axum-server passes the accepted TCP SocketAddr to the make-service for
    // both transports. No proxy headers or synthetic loopback address are used.
    let service = app.into_make_service_with_connect_info::<SocketAddr>();
    let server: Pin<Box<dyn Future<Output = io::Result<()>> + Send>> = match tls {
        Some(tls) => Box::pin(
            axum_server::from_tcp(listener)?
                .acceptor(RustlsAcceptor::new(tls).handshake_timeout(Duration::from_secs(3)))
                .handle(handle.clone())
                .serve(service),
        ),
        None => Box::pin(
            axum_server::from_tcp(listener)?
                .handle(handle.clone())
                .serve(service),
        ),
    };
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => result,
        _ = stop => {
            handle.graceful_shutdown(Some(Duration::from_secs(5)));
            server.await
        }
    }
}
