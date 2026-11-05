use std::net::{Ipv4Addr, SocketAddrV4};

pub struct Config {
    pub bind: SocketAddrV4,
    pub debug: bool,
    pub show_token: bool,
    pub help: bool,
}
impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, &'static str> {
        let mut ip = Ipv4Addr::LOCALHOST;
        let mut port = 8765;
        let mut debug = false;
        let mut show_token = false;
        let mut help = false;
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
                "--show-token" => show_token = true,
                "--help" | "-h" => help = true,
                _ => return Err("Unknown or repeated option; use --help"),
            }
        }
        if show_token && (lan || port_set || debug) {
            return Err("Use --show-token by itself; it never starts the agent");
        }
        Ok(Self {
            bind: SocketAddrV4::new(ip, port),
            debug,
            show_token,
            help,
        })
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
}
