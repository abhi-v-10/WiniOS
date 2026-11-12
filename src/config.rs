use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

pub struct Config {
    pub bind: SocketAddrV4,
    pub debug: bool,
    pub show_token: bool,
    pub help: bool,
    pub allow_write: bool,
    pub insecure_http: bool,
    pub tailnet_backend: bool,
    pub images: crate::image::Options,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}
impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, &'static str> {
        let mut ip = Ipv4Addr::LOCALHOST;
        let mut port = 8765;
        let mut debug = false;
        let mut show_token = false;
        let mut help = false;
        let mut allow_write = false;
        let mut insecure_write = false;
        let mut insecure_http = false;
        let mut tailnet_backend = false;
        let mut images = crate::image::Options::default();
        let mut image_limit_set = false;
        let mut tls_cert = None;
        let mut tls_key = None;
        let mut lan = false;
        let mut port_set = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--lan" if !lan => {
                    ip = args
                        .next()
                        .ok_or("--lan requires your private Wi-Fi IPv4 address")?
                        .parse::<Ipv4Addr>()
                        .map_err(|_| "Invalid --lan IPv4 address")?;
                    if !ip.is_private() {
                        return Err("--lan accepts only a specific RFC1918 private IPv4 address");
                    }
                    lan = true;
                }
                "--port" if !port_set => {
                    port = args
                        .next()
                        .ok_or("--port requires a port number")?
                        .parse::<u16>()
                        .map_err(|_| "Invalid port")?;
                    if port == 0 {
                        return Err("Port must be between 1 and 65535");
                    }
                    port_set = true;
                }
                "--debug-clipboard" => debug = true,
                "--enable-images" if !images.enabled => images.enabled = true,
                "--max-image-mib" if !image_limit_set => {
                    images.max_mib = args
                        .next()
                        .ok_or("--max-image-mib requires 2, 4 or 8")?
                        .parse()
                        .map_err(|_| "Invalid image maximum")?;
                    images.validate()?;
                    image_limit_set = true;
                }
                "--allow-clipboard-write" => allow_write = true,
                "--allow-insecure-clipboard-write" => insecure_write = true,
                "--insecure-http" => insecure_http = true,
                "--tailnet-backend" if !tailnet_backend => tailnet_backend = true,
                "--tls-cert" if tls_cert.is_none() => {
                    tls_cert = Some(PathBuf::from(
                        args.next().ok_or("--tls-cert requires a PEM file path")?,
                    ))
                }
                "--tls-key" if tls_key.is_none() => {
                    tls_key = Some(PathBuf::from(
                        args.next().ok_or("--tls-key requires a PEM file path")?,
                    ))
                }
                "--show-token" => show_token = true,
                "--help" | "-h" => help = true,
                _ => return Err("Unknown or repeated option; use --help"),
            }
        }
        if show_token
            && (lan
                || port_set
                || debug
                || allow_write
                || insecure_write
                || insecure_http
                || tailnet_backend
                || images.enabled
                || image_limit_set
                || tls_cert.is_some()
                || tls_key.is_some())
        {
            return Err("Use --show-token by itself; it never starts the agent");
        }
        if tls_cert.is_some() != tls_key.is_some() {
            return Err("Supply both --tls-cert and --tls-key");
        }
        if tailnet_backend {
            if lan || insecure_http || insecure_write || tls_cert.is_some() {
                return Err(
                    "--tailnet-backend is loopback-only and cannot use LAN, legacy HTTP or TLS identity flags",
                );
            }
            if !port_set {
                port = 8766;
            }
            if port == 8765 {
                return Err(
                    "Tailnet backend must use a separate port (default 8766), not LAN port 8765",
                );
            }
        }
        if insecure_http && tls_cert.is_some() {
            return Err("--insecure-http cannot be combined with TLS identity flags");
        }
        if insecure_write && !insecure_http {
            return Err(
                "Legacy write permission requires --insecure-http; use --allow-clipboard-write for HTTPS",
            );
        }
        if allow_write && insecure_http {
            return Err("Plaintext writes require --allow-insecure-clipboard-write instead");
        }
        allow_write |= insecure_write;
        if allow_write && !lan && !tailnet_backend {
            return Err(
                "Clipboard writes require explicit --lan <private-IPv4> or --tailnet-backend",
            );
        }
        if allow_write && debug {
            return Err("Clipboard writes cannot be combined with --debug-clipboard");
        }
        if images.enabled && insecure_http {
            return Err("Image sharing requires HTTPS or the dedicated Tailscale backend.");
        }
        if image_limit_set && !images.enabled {
            return Err("--max-image-mib requires --enable-images.");
        }
        Ok(Self {
            bind: SocketAddrV4::new(ip, port),
            debug,
            show_token,
            help,
            allow_write,
            insecure_http,
            tailnet_backend,
            images,
            tls_cert,
            tls_key,
        })
    }

    pub fn tls_paths(&self) -> Result<Option<(PathBuf, PathBuf)>, &'static str> {
        if self.insecure_http || self.tailnet_backend {
            return Ok(None);
        }
        if let (Some(cert), Some(key)) = (&self.tls_cert, &self.tls_key) {
            return Ok(Some((cert.clone(), key.clone())));
        }
        let directory = PathBuf::from(
            std::env::var_os("LOCALAPPDATA")
                .ok_or("Set explicit --tls-cert and --tls-key paths")?,
        )
        .join("WiniOS")
        .join("tls");
        Ok(Some((
            directory.join("server.cert.pem"),
            directory.join("server.key.pem"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn images_are_opt_in_capped_and_never_use_legacy_http() {
        assert!(!Config::parse(Vec::<String>::new()).unwrap().images.enabled);
        let parse = |args: &[&str]| Config::parse(args.iter().map(|v| v.to_string()));
        assert_eq!(
            parse(&[
                "--tailnet-backend",
                "--enable-images",
                "--max-image-mib",
                "4"
            ])
            .unwrap()
            .images
            .max_mib,
            4
        );
        for args in [
            vec!["--insecure-http", "--enable-images"],
            vec!["--max-image-mib", "8"],
            vec!["--enable-images", "--max-image-mib", "255"],
            vec!["--enable-images", "--show-token"],
        ] {
            assert!(parse(&args).is_err());
        }
    }
    fn parse(args: &[&str]) -> Result<Config, &'static str> {
        Config::parse(args.iter().map(|s| s.to_string()))
    }
    #[test]
    fn tailnet_is_dedicated_loopback_not_legacy_http_and_writes_remain_opt_in() {
        let config = parse(&["--tailnet-backend"]).unwrap();
        assert_eq!(config.bind, "127.0.0.1:8766".parse().unwrap());
        assert!(!config.insecure_http && !config.allow_write);
        assert!(config.tls_paths().unwrap().is_none());
        assert!(
            parse(&["--tailnet-backend", "--allow-clipboard-write"])
                .unwrap()
                .allow_write
        );
        for args in [
            vec!["--tailnet-backend", "--lan", "192.168.1.7"],
            vec!["--tailnet-backend", "--insecure-http"],
            vec!["--tailnet-backend", "--allow-insecure-clipboard-write"],
            vec!["--tailnet-backend", "--tls-cert", "a", "--tls-key", "b"],
            vec!["--tailnet-backend", "--port", "8765"],
            vec!["--tailnet-backend", "--show-token"],
        ] {
            assert!(parse(&args).is_err());
        }
    }
    #[test]
    fn defaults_to_loopback_and_requires_explicit_private_lan_ip() {
        assert_eq!(
            parse(&[]).unwrap().bind,
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8765)
        );
        assert_eq!(
            *parse(&["--lan", "192.168.1.5"]).unwrap().bind.ip(),
            Ipv4Addr::new(192, 168, 1, 5)
        );
        for ip in ["0.0.0.0", "127.0.0.1", "8.8.8.8", "::", "169.254.1.1"] {
            assert!(parse(&["--lan", ip]).is_err());
        }
        assert!(parse(&["--port", "0"]).is_err());
        assert!(parse(&["--show-token", "--lan", "192.168.1.5"]).is_err());
    }

    #[test]
    fn writes_are_opt_in_lan_only_and_cannot_log_contents() {
        assert!(!parse(&[]).unwrap().allow_write);
        assert!(!parse(&["--lan", "192.168.1.5"]).unwrap().allow_write);
        assert!(parse(&["--allow-insecure-clipboard-write"]).is_err());
        assert!(
            parse(&["--lan", "192.168.1.5", "--allow-clipboard-write"])
                .unwrap()
                .allow_write
        );
        assert!(
            parse(&[
                "--lan",
                "192.168.1.5",
                "--allow-insecure-clipboard-write",
                "--debug-clipboard"
            ])
            .is_err()
        );
    }

    #[test]
    fn tls_is_default_and_legacy_transport_never_appears_implicitly() {
        assert!(!parse(&[]).unwrap().insecure_http);
        assert!(!parse(&["--lan", "192.168.1.7"]).unwrap().insecure_http);
        assert!(parse(&["--tls-cert", "server.pem"]).is_err());
        assert!(parse(&["--insecure-http", "--tls-cert", "a", "--tls-key", "b"]).is_err());
        assert!(parse(&["--lan", "192.168.1.7", "--allow-insecure-clipboard-write"]).is_err());
        assert!(
            parse(&[
                "--insecure-http",
                "--lan",
                "192.168.1.7",
                "--allow-clipboard-write"
            ])
            .is_err()
        );
        let legacy = parse(&[
            "--insecure-http",
            "--lan",
            "192.168.1.7",
            "--allow-insecure-clipboard-write",
        ])
        .unwrap();
        assert!(legacy.insecure_http && legacy.allow_write);
        assert!(legacy.tls_paths().unwrap().is_none());
        let secure = parse(&[
            "--lan",
            "192.168.1.7",
            "--tls-cert",
            "a",
            "--tls-key",
            "b",
            "--allow-clipboard-write",
        ])
        .unwrap();
        assert_eq!(
            secure.tls_paths().unwrap().unwrap(),
            (PathBuf::from("a"), PathBuf::from("b"))
        );
    }
}
