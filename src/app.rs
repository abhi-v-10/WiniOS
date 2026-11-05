use crate::{
    clipboard::{ClipboardStore, ReadValue, UnavailableReason},
    config::Config,
    http::{self, ApiState},
    native::{self, AppError, SingleInstance},
    transport, watcher,
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
    let tls = match config.tls_paths()? {
        Some((cert, key)) => Some(transport::load_identity(&cert, &key).await?),
        None => None,
    };
    let scheme = if tls.is_some() { "https" } else { "http" };
    let token = Arc::new(native::load_or_create_token()?);
    let listener = TcpListener::bind(config.bind)?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let store = ClipboardStore::default();
    let (write_sender, write_receiver) = tokio::sync::mpsc::channel(QUEUE_CAPACITY);
    let writes = config
        .allow_write
        .then(|| WriteClient::new(write_sender, watcher::wake_writes));
    let app = http::router(ApiState {
        store: store.clone(),
        token,
        sequence: watcher::sequence,
        writes,
    });
    let (ready_tx, ready_rx) = oneshot::channel();
    let (done_tx, mut done_rx) = oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("winios-clipboard".into())
        .spawn(move || {
            let status_store = store.clone();
            let result = watcher::run(store, config.debug, write_receiver, || {
                let _ = ready_tx.send(());
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
    println!(
        "WiniOS API: {scheme}://{address} (selected IP {}, port {})",
        address.ip(),
        address.port()
    );
    println!(
        "Token is in Windows Credential Manager; reveal explicitly with: cargo run -- --show-token"
    );
    println!(
        "iPhone Shortcut: URL {scheme}://{address}/v1/clipboard -> Get Contents of URL (GET, Authorization: Bearer <token>) -> Copy to Clipboard."
    );
    if address.ip().is_loopback() {
        println!("iPhone access requires restarting with --lan <private-Wi-Fi-IPv4>.");
    }
    if config.insecure_http {
        println!(
            "WARNING: explicit legacy HTTP mode is unencrypted. Disposable text only; no port forwarding."
        );
    }
    if config.allow_write {
        println!("Opt-in POST /v1/clipboard is ENABLED.");
    }
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let server = transport::serve(listener, app, tls, stop_rx);
    tokio::pin!(server);
    let (server_result, watcher_result) = tokio::select! {
        result = &mut server => {
            // Errors in either subsystem shut down the other subsystem.
            watcher::request_shutdown()?;
            (result, done_rx.await)
        }
        result = &mut done_rx => {
            let _ = stop_tx.send(());
            let drained = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
            let server_result = match drained {
                Ok(result) => result,
                Err(_) => { eprintln!("HTTP drain deadline reached; closing remaining connections."); Ok(()) }
            };
            (server_result, result)
        }
    };
    thread.join().map_err(|_| "Clipboard watcher panicked")?;
    server_result?;
    watcher_result
        .map_err(|_| "Clipboard watcher exited without status")?
        .map_err(|error| -> AppError { error.into() })?;
    println!("WiniOS HTTP server stopped.");
    Ok(())
}
