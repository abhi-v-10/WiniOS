use crate::{
    clipboard::{ClipboardStore, ReadValue, UnavailableReason},
    config::Config,
    http::{self, ApiState},
    lifecycle::{self, Event},
    native::{self, AppError, SingleInstance},
    tailnet, transport, watcher,
    writes::{QUEUE_CAPACITY, WriteClient},
};
use std::{net::TcpListener, sync::Arc, time::Duration};
use tokio::sync::oneshot;

pub async fn run(config: Config) -> Result<(), AppError> {
    if config.show_token {
        let token = native::read_token()?
            .ok_or("No WiniOS credential exists. Start the agent once first.")?;
        // Explicit secret-export command ONLY. No watcher/server is started.
        println!("{}", token.secret());
        return Ok(());
    }
    let _instance = SingleInstance::acquire()?;
    let (_stop, receiver) = tokio::sync::watch::channel(false);
    run_managed(
        config,
        true,
        receiver,
        Arc::new(|_| {}),
        crate::sharing::SharingControl::default(),
    )
    .await
}

/// The desktop UI holds the singleton until its worker and tray resources exit.
pub(crate) async fn run_managed(
    config: Config,
    console: bool,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    report: Arc<dyn Fn(Event) + Send + Sync>,
    sharing: crate::sharing::SharingControl,
) -> Result<(), AppError> {
    if *shutdown.borrow() {
        return Ok(());
    }
    if config.images.enabled && config.insecure_http {
        return Err("Images require validated HTTPS or the dedicated Serve backend.".into());
    }
    if config.tailnet_backend
        && (*config.bind.ip() != std::net::Ipv4Addr::LOCALHOST
            || config.bind.port() == 8765
            || config.insecure_http
            || config.tls_cert.is_some()
            || config.tls_key.is_some())
    {
        return Err(
            "Invalid tailnet boundary: only the dedicated 127.0.0.1 backend is permitted.".into(),
        );
    }
    let tls = match config.tls_paths()? {
        Some((cert, key)) => {
            transport::validate_server_ip(&cert, *config.bind.ip())?;
            Some(transport::load_identity(&cert, &key).await?)
        }
        None => None,
    };
    let scheme = if tls.is_some() { "https" } else { "http" };
    let token = Arc::new(native::load_or_create_token()?);
    let listener = TcpListener::bind(config.bind)?;
    listener.set_nonblocking(true)?;
    if *shutdown.borrow() {
        return Ok(());
    }
    let address = listener.local_addr()?;
    let store = ClipboardStore::with_images(config.images);
    let (write_sender, write_receiver) = tokio::sync::mpsc::channel(QUEUE_CAPACITY);
    let local_writer = WriteClient::new(write_sender, watcher::wake_writes);
    let writes = config.allow_write.then(|| local_writer.clone());
    let state = ApiState {
        store: store.clone(),
        token,
        sequence: watcher::sequence,
        writes,
    };
    let gate = tailnet::Gate::default();
    let app = http::controlled_router(state, config.tailnet_backend.then(|| gate.clone()), sharing);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (done_tx, mut done_rx) = oneshot::channel();
    let early_shutdown = shutdown.clone();
    let thread = std::thread::Builder::new()
        .name("winios-clipboard".into())
        .spawn(move || {
            let status_store = store.clone();
            let result = watcher::run(store, config.debug, console, write_receiver, || {
                let _ = ready_tx.send(());
                // Quit may arrive before native thread registration/initial read.
                if *early_shutdown.borrow() {
                    let _ = watcher::request_shutdown();
                }
            })
            .map_err(|error| error.to_string());
            let mut stopped = status_store.begin_observation(None);
            let _ = status_store.complete(
                &mut stopped,
                ReadValue::Unavailable(UnavailableReason::WatcherStopped),
            );
            let _ = done_tx.send(result);
        })?;
    if ready_rx.await.is_err() {
        let result = done_rx.await;
        thread.join().map_err(|_| "Clipboard watcher panicked")?;
        return Err(result
            .ok()
            .and_then(Result::err)
            .unwrap_or_else(|| "Clipboard watcher exited during startup".into())
            .into());
    }
    if !*shutdown.borrow() {
        report(Event::Permissions(config.allow_write));
        report(Event::ImageOptions(config.images));
        report(Event::ClipboardWriter(local_writer));
    }
    if !*shutdown.borrow() && !config.tailnet_backend {
        report(Event::Running(address));
    }
    crate::diagnostics::info!(
        "WiniOS API: {scheme}://{address} (selected IP {}, port {})",
        address.ip(),
        address.port()
    );
    crate::diagnostics::info!(
        "Token is in Windows Credential Manager; reveal explicitly with: cargo run -- --show-token"
    );
    if !config.tailnet_backend {
        crate::diagnostics::info!(
            "iPhone Shortcut: URL {scheme}://{address}/v1/clipboard -> Get Contents of URL (GET, Authorization: Bearer <token>) -> Copy to Clipboard."
        );
    }
    if address.ip().is_loopback() && !config.tailnet_backend {
        crate::diagnostics::info!(
            "iPhone access requires restarting with --lan <private-Wi-Fi-IPv4>."
        );
    }
    if config.insecure_http {
        crate::diagnostics::info!(
            "WARNING: explicit legacy HTTP mode is unencrypted. Disposable text only; no port forwarding."
        );
    }
    if config.allow_write {
        crate::diagnostics::info!("Opt-in POST /v1/clipboard is ENABLED.");
    }
    if config.tailnet_backend {
        crate::diagnostics::info!(
            "Tailnet backend is same-host HTTP only. Waiting for verified Tailscale HTTPS Serve; no settings are changed automatically."
        );
    }
    let monitoring = async {
        if config.tailnet_backend {
            tailnet::monitor(gate.clone(), address.port(), |observation| {
                crate::diagnostics::info!(
                    "Tailscale: {}{}",
                    observation.state.label(),
                    observation
                        .endpoint
                        .as_ref()
                        .map_or(String::new(), |url| format!(" — {url}"))
                );
                report(Event::Connectivity(observation));
            })
            .await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::pin!(monitoring);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let server = transport::serve(listener, app, tls, stop_rx);
    tokio::pin!(server);
    let (server_result, watcher_result) = tokio::select! {
        _ = lifecycle::cancelled(&mut shutdown) => {
            gate.close();
            watcher::request_shutdown()?;
            let _ = stop_tx.send(());
            let result = done_rx.await;
            let drained = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
            (drained.unwrap_or(Ok(())), result)
        }
        result = &mut server => {
            gate.close();
            // Errors in either subsystem shut down the other subsystem.
            watcher::request_shutdown()?;
            (result, done_rx.await)
        }
        result = &mut done_rx => {
            gate.close();
            let _ = stop_tx.send(());
            let drained = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
            let server_result = match drained {
                Ok(result) => result,
                Err(_) => { crate::diagnostics::error!("HTTP drain deadline reached; closing remaining connections."); Ok(()) }
            };
            (server_result, result)
        }
        _ = &mut monitoring => unreachable!("tailnet monitor runs until application shutdown"),
    };
    thread.join().map_err(|_| "Clipboard watcher panicked")?;
    server_result?;
    watcher_result
        .map_err(|_| "Clipboard watcher exited without status")?
        .map_err(|error| -> AppError { error.into() })?;
    crate::diagnostics::info!("WiniOS HTTP server stopped.");
    Ok(())
}
