#[cfg(windows)]
mod app;
mod auth;
mod clipboard;
mod config;
mod http;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod watcher;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = config::Config::parse(std::env::args().skip(1))?;
    if config.help {
        println!(
            "winios-agent [--lan <private-IPv4>] [--port <1-65535>] [--debug-clipboard]\nwinios-agent --show-token\n\nDefault: 127.0.0.1:8765. --show-token explicitly reveals the secret and exits.\n--debug-clipboard logs sensitive clipboard contents. HTTP is unencrypted."
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
