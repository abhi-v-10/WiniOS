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
use std::{io::Read, net::Ipv4Addr};
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

/// Startup guard, not a replacement for clients' normal trust validation.
/// Never silently select another IP or regenerate an identity.
pub fn validate_server_ip(path: &Path, ip: Ipv4Addr) -> io::Result<()> {
    use x509_parser::{
        extensions::GeneralName,
        prelude::{FromDer, X509Certificate},
    };
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "TLS certificate is invalid, expired/not-yet-valid, or lacks serverAuth/IP SAN for the configured address. Correct settings or safely reprovision; no rebinding or downgrade occurred.",
        )
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "TLS certificate is missing or inaccessible.",
            )
        })?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() > 64 * 1024 {
        return Err(invalid());
    }
    let (_, pem) = x509_parser::pem::parse_x509_pem(&bytes).map_err(|_| invalid())?;
    let (_, certificate) = X509Certificate::from_der(&pem.contents).map_err(|_| invalid())?;
    if !certificate.validity().is_valid() {
        return Err(invalid());
    }
    if certificate
        .basic_constraints()
        .map_err(|_| invalid())?
        .is_some_and(|extension| extension.value.ca)
    {
        return Err(invalid());
    }
    if !certificate
        .extended_key_usage()
        .map_err(|_| invalid())?
        .is_some_and(|extension| extension.value.server_auth)
    {
        return Err(invalid());
    }
    let san = certificate
        .subject_alternative_name()
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    if !san
        .value
        .general_names
        .iter()
        .any(|name| matches!(name, GeneralName::IPAddress(bytes) if *bytes == ip.octets()))
    {
        return Err(invalid());
    }
    Ok(())
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
