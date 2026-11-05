#[cfg(windows)]
mod app;
mod auth;
mod clipboard;
mod config;
mod http;
#[cfg(windows)]
mod native;
mod transport;
#[cfg(test)]
mod transport_tests;
#[cfg(windows)]
mod watcher;
mod writes;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = config::Config::parse(std::env::args().skip(1))?;
    if config.help {
        println!(
            "winios-agent [--lan <private-IPv4>] [--port <1-65535>] [--tls-cert <PEM> --tls-key <PEM>] [--allow-clipboard-write]\nwinios-agent --show-token\n\nDefault: HTTPS on 127.0.0.1:8765, identity under %LOCALAPPDATA%\\WiniOS\\tls. No HTTP fallback.\nPOST requires explicit --lan and --allow-clipboard-write.\nLegacy development only: --insecure-http [--allow-insecure-clipboard-write].\n--show-token explicitly exports the secret and exits; --debug-clipboard logs metadata only."
        );
        return Ok(());
    }
    #[cfg(windows)]
    app::run(config).await?;
    #[cfg(not(windows))]
    {
        let _ = config;
        return Err("The WiniOS agent requires Windows.".into());
    }
    #[cfg(windows)]
    Ok(())
}
