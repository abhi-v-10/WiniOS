use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

pub struct Config {
    pub bind: SocketAddrV4,
    pub debug: bool,
    pub show_token: bool,
    pub help: bool,
    pub allow_write: bool,
    pub insecure_http: bool,
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
                "--allow-clipboard-write" => allow_write = true,
                "--allow-insecure-clipboard-write" => insecure_write = true,
                "--insecure-http" => insecure_http = true,
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
                || tls_cert.is_some()
                || tls_key.is_some())
        {
            return Err("Use --show-token by itself; it never starts the agent");
        }
        if tls_cert.is_some() != tls_key.is_some() {
            return Err("Supply both --tls-cert and --tls-key");
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
        if allow_write && !lan {
            return Err("Clipboard writes require explicit --lan <private-IPv4>");
        }
        if allow_write && debug {
            return Err("Clipboard writes cannot be combined with --debug-clipboard");
        }
        Ok(Self {
            bind: SocketAddrV4::new(ip, port),
            debug,
            show_token,
            help,
            allow_write,
            insecure_http,
            tls_cert,
            tls_key,
        })
    }

    pub fn tls_paths(&self) -> Result<Option<(PathBuf, PathBuf)>, &'static str> {
        if self.insecure_http {
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
    fn parse(args: &[&str]) -> Result<Config, &'static str> {
        Config::parse(args.iter().map(|s| s.to_string()))
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
