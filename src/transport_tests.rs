//! Real TCP/TLS integration tests. Keys exist only in test memory, never the repo.
use crate::{
    auth::BearerToken,
    clipboard::{ClipboardStore, ReadValue},
    http::{self, ApiState},
    transport,
    writes::{WriteClient, WriteReceipt},
};
use axum::{extract::ConnectInfo, routing::get};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName},
};
use std::{
    net::{SocketAddr, TcpListener},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpSocket, TcpStream},
    sync::{mpsc, oneshot},
};
use tokio_rustls::{TlsConnector, client::TlsStream};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn identity(expired: bool) -> (RustlsConfig, CertificateDer<'static>) {
    transport::install_crypto_provider();
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca.name_constraints = Some(rcgen::NameConstraints {
        permitted_subtrees: vec![
            rcgen::GeneralSubtree::DnsName("winios.invalid".into()),
            rcgen::GeneralSubtree::IpAddress(rcgen::CidrSubnet::V4([127, 0, 0, 1], [255; 4])),
        ],
        excluded_subtrees: vec![],
    });
    ca.not_before = date_time_ymd(2020, 1, 1);
    ca.not_after = date_time_ymd(2040, 1, 1);
    let root = ca.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca, ca_key);
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.not_before = date_time_ymd(2020, 1, 1);
    params.not_after = date_time_ymd(if expired { 2021 } else { 2040 }, 1, 1);
    let leaf = params.signed_by(&key, &issuer).unwrap();
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
    (
        RustlsConfig::from_config(Arc::new(config)),
        root.der().clone(),
    )
}

fn connector(root: Option<CertificateDer<'static>>) -> TlsConnector {
    let mut roots = RootCertStore::empty();
    if let Some(root) = root {
        roots.add(root).unwrap();
    }
    TlsConnector::from(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

async fn start(
    expired: bool,
) -> (
    SocketAddr,
    CertificateDer<'static>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let (tls, root) = identity(expired);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let store = ClipboardStore::default();
    let mut observation = store.begin_observation(Some(7));
    store
        .complete(
            &mut observation,
            ReadValue::Text("disposable TLS GET 😀".into()),
        )
        .unwrap();
    let (sender, mut receiver) = mpsc::channel::<crate::writes::WriteCommand>(4);
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            assert_eq!(command.text, "disposable TLS POST");
            let _ = command.ack.send(Ok(WriteReceipt {
                operation: 22,
                sequence: Some(8),
            }));
        }
    });
    let app = http::router(ApiState {
        store,
        token: Arc::new(BearerToken::from_hex(TOKEN.as_bytes()).unwrap()),
        sequence: || Some(7),
        writes: Some(WriteClient::new(sender, || true)),
    })
    .route(
        "/test-peer",
        get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() }),
    );
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(transport::serve(listener, app, Some(tls), stopped));
    (address, root, stop, task)
}

async fn connect(
    address: SocketAddr,
    root: Option<CertificateDer<'static>>,
    name: &str,
) -> std::io::Result<TlsStream<TcpStream>> {
    let stream = TcpStream::connect(address).await?;
    connector(root)
        .connect(ServerName::try_from(name.to_owned()).unwrap(), stream)
        .await
}

async fn exchange(mut stream: TlsStream<TcpStream>, request: String) -> String {
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    // Some TLS implementations close TCP without close_notify after HTTP close;
    // this is an error only if the expected HTTP response is incomplete.
    let result = tokio::time::timeout(Duration::from_secs(4), stream.read_to_end(&mut response))
        .await
        .unwrap();
    if let Err(error) = result {
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
    String::from_utf8(response).unwrap()
}

async fn request(
    address: SocketAddr,
    root: CertificateDer<'static>,
    method: &str,
    path: &str,
    auth: bool,
    body: &str,
) -> String {
    let auth = if auth {
        format!("Authorization: Bearer {TOKEN}\r\n")
    } else {
        String::new()
    };
    exchange(connect(address, Some(root), "127.0.0.1").await.unwrap(),
        format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())).await
}

async fn stop(sender: oneshot::Sender<()>, task: tokio::task::JoinHandle<std::io::Result<()>>) {
    sender.send(()).ok();
    tokio::time::timeout(Duration::from_secs(6), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn validated_https_health_bearer_get_and_post() {
    let (address, root, shutdown, task) = start(false).await;
    assert!(
        request(address, root.clone(), "GET", "/v1/health", false, "")
            .await
            .starts_with("HTTP/1.1 200")
    );
    let get = request(address, root.clone(), "GET", "/v1/clipboard", true, "").await;
    assert!(get.starts_with("HTTP/1.1 200"));
    assert!(get.contains("disposable TLS GET 😀"));
    assert!(get.contains("cache-control: no-store"));
    let post = request(
        address,
        root.clone(),
        "POST",
        "/v1/clipboard",
        true,
        r#"{"text":"disposable TLS POST"}"#,
    )
    .await;
    assert!(post.starts_with("HTTP/1.1 200"));
    assert!(post.contains(r#""operation":22"#));
    for method in ["GET", "POST"] {
        assert!(
            request(address, root.clone(), method, "/v1/clipboard", false, "{}")
                .await
                .starts_with("HTTP/1.1 401")
        );
    }
    stop(shutdown, task).await;
}

#[tokio::test]
async fn tls_preserves_exact_tcp_peer_in_connect_info_not_forwarded_headers() {
    let (address, root, shutdown, task) = start(false).await;
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.2:0".parse().unwrap()).unwrap();
    let tcp = socket.connect(address).await.unwrap();
    let actual_peer = tcp.local_addr().unwrap();
    let tls = connector(Some(root))
        .connect(ServerName::try_from("127.0.0.1").unwrap(), tcp)
        .await
        .unwrap();
    let response = exchange(tls, format!("GET /test-peer HTTP/1.1\r\nHost: {address}\r\nX-Forwarded-For: 8.8.8.8\r\nConnection: close\r\n\r\n")).await;
    assert!(response.ends_with(&actual_peer.to_string()));
    assert_eq!(actual_peer.ip().to_string(), "127.0.0.2");
    stop(shutdown, task).await;
}

#[tokio::test]
async fn untrusted_wrong_identity_and_expired_certificates_fail_closed() {
    let (address, root, shutdown, task) = start(false).await;
    assert!(connect(address, None, "127.0.0.1").await.is_err());
    assert!(
        connect(address, Some(root), "wrong.winios.invalid")
            .await
            .is_err()
    );
    stop(shutdown, task).await;
    let (address, root, shutdown, task) = start(true).await;
    assert!(connect(address, Some(root), "127.0.0.1").await.is_err());
    stop(shutdown, task).await;
}

#[tokio::test]
async fn plaintext_and_malformed_tls_are_not_http_and_do_not_break_https() {
    let (address, root, shutdown, task) = start(false).await;
    for bytes in [
        b"GET /v1/clipboard HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n".as_slice(),
        b"\x16\x03\x03\x00\x04\xff\x00\x00\x00".as_slice(),
    ] {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(bytes).await.unwrap();
        let mut response = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(4), stream.read_to_end(&mut response))
            .await
            .unwrap();
        assert!(!response.windows(5).any(|part| part == b"HTTP/"));
        assert!(!response.windows(10).any(|part| part == b"disposable"));
    }
    assert!(
        request(address, root, "GET", "/v1/health", false, "")
            .await
            .starts_with("HTTP/1.1 200")
    );
    stop(shutdown, task).await;
}

#[tokio::test]
async fn invalid_or_missing_pem_is_an_error_without_creating_a_listener() {
    let path = std::env::temp_dir().join(format!("winios-missing-{}.pem", std::process::id()));
    assert!(transport::load_identity(&path, &path).await.is_err());
}

#[tokio::test]
async fn malformed_pem_and_mismatched_key_are_rejected() {
    transport::install_crypto_provider();
    assert!(
        RustlsConfig::from_pem(b"not a certificate".to_vec(), b"not a key".to_vec())
            .await
            .is_err()
    );
    let key = KeyPair::generate().unwrap();
    let wrong_key = KeyPair::generate().unwrap();
    let certificate = CertificateParams::new(vec!["127.0.0.1".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    assert!(
        RustlsConfig::from_pem(
            certificate.pem().into_bytes(),
            wrong_key.serialize_pem().into_bytes()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn graceful_shutdown_is_bounded_with_an_incomplete_handshake() {
    let (address, _, shutdown, task) = start(false).await;
    let mut connection = TcpStream::connect(address).await.unwrap();
    connection
        .write_all(b"\x16\x03\x03\x00\x10\x01")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    stop(shutdown, task).await;
}
