use crate::{
    clipboard::{ClipboardStore, ReadValue, UnavailableReason},
    config::Config,
    http::{self, ApiState},
    native::{self, AppError, SingleInstance},
    watcher,
};
use std::{future::IntoFuture, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::oneshot};

pub async fn run(config: Config) -> Result<(), AppError> {
    if config.show_token {
        let token = native::read_token()?
            .ok_or("No WiniOS credential exists. Start the agent once first.")?;
        // Explicit secret-export command ONLY. No watcher/server is started.
        println!("{}", token.secret());
        return Ok(());
    }
    let _instance = SingleInstance::acquire()?;
    let token = Arc::new(native::load_or_create_token()?);
    let listener = TcpListener::bind(config.bind).await?;
    let address = listener.local_addr()?;
    let store = ClipboardStore::default();
    let app = http::router(ApiState {
        store: store.clone(),
        token,
        sequence: watcher::sequence,
    });
    let (ready_tx, ready_rx) = oneshot::channel();
    let (done_tx, mut done_rx) = oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("winios-clipboard".into())
        .spawn(move || {
            let status_store = store.clone();
            let result = watcher::run(store, config.debug, || {
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
        "WiniOS HTTP: http://{address} (selected IP {}, port {})",
        address.ip(),
        address.port()
    );
    println!(
        "Token is in Windows Credential Manager; reveal explicitly with: cargo run -- --show-token"
    );
    println!(
        "iPhone Shortcut: URL http://{address}/v1/clipboard -> Get Contents of URL (GET, Authorization: Bearer <token>) -> Copy to Clipboard."
    );
    if address.ip().is_loopback() {
        println!("iPhone access requires restarting with --lan <private-Wi-Fi-IPv4>.");
    }
    println!(
        "WARNING: HTTP is unencrypted. Disposable, non-sensitive text on trusted Wi-Fi only; never port-forward this service."
    );
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = stop_rx.await;
    })
    .into_future();
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
