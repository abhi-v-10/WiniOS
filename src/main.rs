#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = winios_agent::config::Config::parse(std::env::args().skip(1))?;
    if config.help {
        println!(
            "Images: --enable-images [--max-image-mib 2|4|8]. Separate GET/POST /v1/clipboard/image (PNG); uploads also require --allow-clipboard-write. Images never use legacy plaintext mode."
        );
        println!(
            "winios-agent [--lan <private-IPv4>] [--port <1-65535>] [--tls-cert <PEM> --tls-key <PEM>] [--allow-clipboard-write]\nwinios-agent --tailnet-backend [--port 8766] [--allow-clipboard-write]\nwinios-agent --show-token\n\nDefault: HTTPS on 127.0.0.1:8765, identity under %LOCALAPPDATA%\\WiniOS\\tls. No HTTP fallback.\nPOST requires explicit --lan or --tailnet-backend and --allow-clipboard-write.\nTailnet mode: same-host HTTP on 127.0.0.1:8766 ONLY, behind separately approved Tailscale HTTPS Serve. It never changes Serve configuration.\nLegacy development only: --insecure-http [--allow-insecure-clipboard-write].\n--show-token explicitly exports the secret and exits; --debug-clipboard logs metadata only."
        );
        return Ok(());
    }
    #[cfg(windows)]
    winios_agent::app::run(config).await?;
    #[cfg(not(windows))]
    {
        let _ = config;
        return Err("The WiniOS agent requires Windows.".into());
    }
    #[cfg(windows)]
    Ok(())
}
