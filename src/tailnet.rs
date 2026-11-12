//! Read-only Tailscale adapter. Never invokes up/down/serve mutation/funnel.
//! Local Tailscale and the Windows host are trusted; proxy headers are not.
use serde_json::Value;
use std::{
    net::IpAddr,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

const MAX_OUTPUT: u64 = 1024 * 1024;
const FRESH_FOR: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Connected,
    Disconnected,
    WaitingForNetwork,
    NotConfigured,
    Error,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Disconnected => "Tailscale disconnected",
            Self::WaitingForNetwork => "Waiting for network",
            Self::NotConfigured => "Not configured",
            Self::Error => "Error",
        }
    }
}

/// Readiness is separate from clipboard state, and expires if checks stall.
#[derive(Clone, Copy, Default)]
struct Readiness {
    checked: Option<Instant>,
    generation: u64,
}
#[derive(Clone)]
pub struct Gate(tokio::sync::watch::Sender<Readiness>);
impl Default for Gate {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(Readiness::default()).0)
    }
}
impl Gate {
    #[cfg(test)]
    pub fn set_for_test(&self, ready: bool) {
        self.update(ready);
    }
    pub fn ready(&self) -> bool {
        self.0
            .borrow()
            .checked
            .is_some_and(|checked| checked.elapsed() < FRESH_FOR)
    }
    pub fn generation(&self) -> u64 {
        self.0.borrow().generation
    }
    pub fn valid(&self, generation: u64) -> bool {
        let state = *self.0.borrow();
        state.generation == generation && state.checked.is_some_and(|t| t.elapsed() < FRESH_FOR)
    }
    fn update(&self, ready: bool) {
        self.0.send_modify(|state| {
            // A sleep/expiration or disconnect/reconnect cannot renew an old request.
            if !ready || state.checked.is_none_or(|t| t.elapsed() >= FRESH_FOR) {
                state.generation = state.generation.wrapping_add(1);
            }
            state.checked = ready.then(Instant::now);
        });
    }
    pub fn close(&self) {
        self.update(false);
    }
    pub async fn invalidated_since(&self, generation: u64) {
        let mut receiver = self.0.subscribe();
        loop {
            let state = *receiver.borrow_and_update();
            if state.generation != generation {
                return;
            }
            let Some(checked) = state.checked else {
                return;
            };
            if checked.elapsed() >= FRESH_FOR {
                return;
            }
            if !matches!(
                tokio::time::timeout_at(
                    tokio::time::Instant::from_std(checked + FRESH_FOR),
                    receiver.changed()
                )
                .await,
                Ok(Ok(()))
            ) {
                return;
            }
        }
    }
}

pub struct Report {
    pub state: State,
    pub endpoint: Option<String>,
}
impl Report {
    fn unavailable(state: State) -> Self {
        Self {
            state,
            endpoint: None,
        }
    }
}

fn dns_name(value: &str) -> Option<String> {
    let name = value
        .strip_suffix('.')
        .unwrap_or(value)
        .to_ascii_lowercase();
    if name.len() > 253
        || !name.ends_with(".ts.net")
        || name.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some(name)
}

/// Only a node-owned name authorized for HTTPS in this current tailnet is used.
fn identity(status: &Value) -> Result<(String, Vec<IpAddr>), State> {
    match status["BackendState"].as_str() {
        Some("Running") => {}
        Some("Stopped") => return Err(State::Disconnected),
        Some("Starting" | "NoState") => return Err(State::WaitingForNetwork),
        Some("NeedsLogin" | "NeedsMachineAuth") => return Err(State::NotConfigured),
        _ => return Err(State::Error),
    }
    if status["Self"]["Online"].as_bool() != Some(true)
        || status["Health"]
            .as_array()
            .is_some_and(|health| !health.is_empty())
    {
        return Err(State::WaitingForNetwork);
    }
    if status["CurrentTailnet"]["MagicDNSEnabled"].as_bool() != Some(true) {
        return Err(State::NotConfigured);
    }
    let host = status["Self"]["DNSName"]
        .as_str()
        .and_then(dns_name)
        .ok_or(State::NotConfigured)?;
    let suffix = status["CurrentTailnet"]["MagicDNSSuffix"]
        .as_str()
        .and_then(dns_name)
        .ok_or(State::NotConfigured)?;
    let prefix = host
        .strip_suffix(&format!(".{suffix}"))
        .ok_or(State::NotConfigured)?;
    if prefix.is_empty()
        || prefix.contains('.')
        || !status["CertDomains"].as_array().is_some_and(|domains| {
            domains
                .iter()
                .any(|domain| domain.as_str() == Some(host.as_str()))
        })
    {
        return Err(State::NotConfigured);
    }
    let addresses: Vec<IpAddr> = status["Self"]["TailscaleIPs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str()?.parse().ok())
        .collect();
    if addresses.is_empty() {
        return Err(State::WaitingForNetwork);
    }
    Ok((host, addresses))
}

/// Deliberately narrow profile: one HTTPS root route, no Funnel or alternate
/// listeners/foreground services. Unknown configuration fails closed for review.
fn valid_serve(config: &Value, host: &str, port: u16) -> bool {
    let Some(object) = config.as_object() else {
        return false;
    };
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "TCP" | "Web" | "AllowFunnel" | "Foreground" | "Services" | "ETag"
        )
    }) {
        return false;
    }
    for field in ["Foreground", "Services"] {
        if config
            .get(field)
            .is_some_and(|v| !v.is_null() && v.as_object().is_none_or(|map| !map.is_empty()))
        {
            return false;
        }
    }
    if config.get("AllowFunnel").is_some_and(|v| {
        !v.is_null()
            && v.as_object()
                .is_none_or(|map| map.values().any(|value| value != false))
    }) {
        return false;
    }
    config["TCP"] == serde_json::json!({"443":{"HTTPS":true}})
        && config["Web"]
            == serde_json::json!({format!("{host}:443"):{"Handlers":{"/":{"Proxy":format!("http://127.0.0.1:{port}")}}}})
}

fn cli_path() -> Option<PathBuf> {
    // Never execute a workspace/PATH-supplied tailscale.exe.
    let directory =
        std::env::var_os("ProgramW6432").or_else(|| std::env::var_os("ProgramFiles"))?;
    let path = PathBuf::from(directory)
        .join("Tailscale")
        .join("tailscale.exe");
    path.is_file().then_some(path)
}

async fn query(path: &std::path::Path, args: &[&str]) -> Result<Value, State> {
    let mut command = tokio::process::Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW; tray remains console-free.
    let mut child = command.spawn().map_err(|_| State::Error)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or(State::Error)?
        .take(MAX_OUTPUT + 1);
    let result = tokio::time::timeout(Duration::from_secs(4), async {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| State::Error)?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err(State::Error);
        }
        if !child.wait().await.map_err(|_| State::Error)?.success() {
            return Err(State::Error);
        }
        // Never return or log raw CLI output (identities/keys/login URLs).
        serde_json::from_slice(&bytes).map_err(|_| State::Error)
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => Err(State::WaitingForNetwork),
    }
    // On timeout, cancellation, or oversized output, kill_on_drop closes child.
}

async fn inspect(port: u16) -> Report {
    let result = async {
        let path = cli_path().ok_or(State::NotConfigured)?;
        let status = query(&path, &["status", "--json", "--peers=false"]).await?;
        let (host, addresses) = identity(&status)?;
        let serve = query(&path, &["serve", "status", "--json"]).await?;
        if !valid_serve(&serve, &host, port) {
            return Err(State::NotConfigured);
        }
        let resolved = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::lookup_host((host.as_str(), 443)),
        )
        .await
        .map_err(|_| State::WaitingForNetwork)?
        .map_err(|_| State::WaitingForNetwork)?;
        let resolved: Vec<_> = resolved.collect();
        if resolved.is_empty()
            || resolved
                .iter()
                .any(|address| !addresses.contains(&address.ip()))
        {
            return Err(State::WaitingForNetwork);
        }
        Ok(format!("https://{host}"))
    }
    .await;
    match result {
        Ok(endpoint) => Report {
            state: State::Connected,
            endpoint: Some(endpoint),
        },
        Err(state) => Report::unavailable(state),
    }
}

/// Read-only prerequisite check for the native setup UI. No login/provisioning.
pub async fn setup_client() -> Result<(), &'static str> {
    let path = cli_path().ok_or("Tailscale is not installed in Windows Program Files. Install the official Windows client from https://tailscale.com/download/windows, sign in yourself, and choose Check again.")?;
    let status = query(&path, &["status", "--json", "--peers=false"]).await
        .map_err(|_| "Could not read Tailscale status. Open the Tailscale app, check that its service is running and your Windows user has access, then check again.")?;
    setup_client_status(&status)
}
fn setup_client_status(status: &Value) -> Result<(), &'static str> {
    match status["BackendState"].as_str() {
        Some("Running")
            if status["Self"]["Online"].as_bool() == Some(true)
                && status["Health"]
                    .as_array()
                    .is_none_or(|health| health.is_empty()) =>
        {
            Ok(())
        }
        Some("NeedsLogin") => Err(
            "Tailscale needs sign-in. Sign in using the Tailscale app on Windows and iPhone. WiniOS never handles account credentials.",
        ),
        Some("NeedsMachineAuth") => Err(
            "This Tailscale device needs administrator approval in your tailnet. Approve it yourself, then check again.",
        ),
        Some("Stopped") => {
            Err("Tailscale is disconnected. Connect it in the Tailscale app, then check again.")
        }
        _ => Err(
            "Tailscale is starting or waiting for network access. Check your connection and try again.",
        ),
    }
}

fn setup_identity(status: &Value) -> Result<(String, Vec<IpAddr>), &'static str> {
    setup_client_status(status)?;
    if status["CurrentTailnet"]["MagicDNSEnabled"].as_bool() != Some(true) {
        return Err(
            "MagicDNS is not ready. Ask your tailnet administrator to review and enable MagicDNS, then Check again. WiniOS will not change it.",
        );
    }
    identity(status).map_err(|_| "Private HTTPS identity is not ready. Review HTTPS certificates in the Tailscale admin console, its certificate-transparency notice, and this device's tailnet DNS name. Do not type an arbitrary hostname or bypass certificate verification.")
}
fn setup_route(config: &Value, host: &str, port: u16) -> Result<(), &'static str> {
    if valid_serve(config, host, port) {
        Ok(())
    } else {
        Err(
            "Private Serve route is missing or does not match this device and localhost backend. Review tailscale serve status and the command above yourself. Funnel, extra routes, alternate ports and public/LAN HTTP are refused. Never reset unrelated routes; WiniOS changes nothing.",
        )
    }
}
pub async fn setup_serve(port: u16) -> Result<String, &'static str> {
    let path = cli_path().ok_or("Install the official Tailscale Windows client first, sign in and connect; then Check again.")?;
    let status = query(&path, &["status", "--json", "--peers=false"]).await
        .map_err(|_| "Could not read Tailscale status. Open Tailscale, check its service and network, then Check again.")?;
    let (host, addresses) = setup_identity(&status)?;
    let serve = query(&path, &["serve", "status", "--json"]).await
        .map_err(|_| "Could not read Serve routes. Review tailscale serve status yourself; WiniOS will not replace a route.")?;
    setup_route(&serve, &host, port)?;
    let resolved = tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host((host.as_str(), 443))).await
        .map_err(|_| "Tailnet DNS timed out. Reconnect Tailscale and check internet access, then Check again.")?
        .map_err(|_| "Tailnet DNS failed. Reconnect Tailscale on both devices and Check again; do not substitute a router IP.")?;
    let resolved: Vec<_> = resolved.collect();
    if resolved.is_empty()
        || resolved
            .iter()
            .any(|address| !addresses.contains(&address.ip()))
    {
        return Err(
            "Tailnet DNS does not resolve exclusively to this device. Review MagicDNS and other VPN/DNS settings; no endpoint is accepted.",
        );
    }
    Ok(format!("https://{host}"))
}

/// Bounded async checks run on the API runtime, never the Win32 UI thread.
/// WiniOS only reads Serve configuration; its lifetime is owned by the user.
pub async fn monitor(gate: Gate, port: u16, report: impl Fn(Report)) {
    let mut pinned_endpoint = None;
    let mut previous = None;
    loop {
        let mut observation = inspect(port).await;
        if observation.state == State::Connected {
            if pinned_endpoint.is_none() {
                pinned_endpoint = observation.endpoint.clone();
            }
            if pinned_endpoint != observation.endpoint {
                observation = Report::unavailable(State::NotConfigured);
            }
        }
        gate.update(observation.state == State::Connected);
        let delay = if observation.state == State::Connected {
            30
        } else {
            5
        };
        let signature = (observation.state, observation.endpoint.clone());
        if previous.as_ref() != Some(&signature) {
            previous = Some(signature);
            report(observation);
        }
        tokio::time::sleep(Duration::from_secs(delay)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[tokio::test]
    async fn cli_queries_bound_output_and_time_and_reject_failed_or_malformed_results() {
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let args = ["-NoProfile", "-NonInteractive", "-Command"];
        let run = |script| {
            let mut arguments = args.to_vec();
            arguments.push(script);
            arguments
        };
        assert_eq!(
            query(&shell, &run("[Console]::Write('{}')")).await.unwrap(),
            serde_json::json!({})
        );
        for script in [
            "exit 7",
            "[Console]::Write('{')",
            "[Console]::Write(('x' * 1048577))",
        ] {
            assert_eq!(query(&shell, &run(script)).await.unwrap_err(), State::Error);
        }
        let started = Instant::now();
        assert_eq!(
            query(&shell, &run("Start-Sleep -Seconds 15"))
                .await
                .unwrap_err(),
            State::WaitingForNetwork
        );
        assert!(started.elapsed() < Duration::from_secs(7));
    }
    fn status() -> Value {
        serde_json::json!({"BackendState":"Running","Self":{"Online":true,"DNSName":"pc.example.ts.net.","TailscaleIPs":["100.64.1.2"]},"CurrentTailnet":{"MagicDNSEnabled":true,"MagicDNSSuffix":"example.ts.net"},"CertDomains":["pc.example.ts.net"]})
    }
    #[test]
    fn onboarding_recovery_distinguishes_login_dns_identity_and_unsafe_routes() {
        for state in ["NeedsLogin", "NeedsMachineAuth", "Stopped", "Starting"] {
            let mut value = status();
            value["BackendState"] = state.into();
            assert!(setup_identity(&value).is_err());
        }
        let mut value = status();
        value["CurrentTailnet"]["MagicDNSEnabled"] = false.into();
        assert!(setup_identity(&value).unwrap_err().contains("MagicDNS"));
        value = status();
        value["CertDomains"] = serde_json::json!([]);
        assert!(
            setup_identity(&value)
                .unwrap_err()
                .contains("HTTPS identity")
        );
        assert_eq!(setup_identity(&status()).unwrap().0, "pc.example.ts.net");
        assert!(setup_route(&serve(), "pc.example.ts.net", 8766).is_ok());
        let mut route = serve();
        route["AllowFunnel"] = serde_json::json!({"pc.example.ts.net:443":true});
        assert!(setup_route(&route, "pc.example.ts.net", 8766).is_err());
        assert!(setup_route(&serve(), "pc.example.ts.net", 9999).is_err());
    }
    fn serve() -> Value {
        serde_json::json!({"TCP":{"443":{"HTTPS":true}},"Web":{"pc.example.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8766"}}}}})
    }
    #[test]
    fn verifies_current_node_dns_certificate_permission_and_connection() {
        assert_eq!(identity(&status()).unwrap().0, "pc.example.ts.net");
        for (pointer, value, expected) in [
            (
                "/BackendState",
                serde_json::json!("Stopped"),
                State::Disconnected,
            ),
            (
                "/BackendState",
                serde_json::json!("NeedsLogin"),
                State::NotConfigured,
            ),
            (
                "/Self/Online",
                serde_json::json!(false),
                State::WaitingForNetwork,
            ),
            (
                "/Self/DNSName",
                serde_json::json!("pc.attacker.example.ts.net"),
                State::NotConfigured,
            ),
            (
                "/Self/DNSName",
                serde_json::json!("pc.example.ts.net/evil"),
                State::NotConfigured,
            ),
            ("/CertDomains", serde_json::json!([]), State::NotConfigured),
            (
                "/CurrentTailnet/MagicDNSEnabled",
                serde_json::json!(false),
                State::NotConfigured,
            ),
            (
                "/Health",
                serde_json::json!(["network problem"]),
                State::WaitingForNetwork,
            ),
        ] {
            let mut data = status();
            if pointer == "/Health" {
                data["Health"] = value;
            } else {
                *data.pointer_mut(pointer).unwrap() = value;
            }
            assert_eq!(identity(&data).unwrap_err(), expected);
        }
    }
    #[test]
    fn serve_requires_exact_loopback_https_route_without_funnel_or_other_routes() {
        assert!(valid_serve(&serve(), "pc.example.ts.net", 8766));
        for target in [
            "http://192.168.1.7:8766",
            "http://localhost:8766",
            "http://127.0.0.1:8765",
            "https+insecure://127.0.0.1:8766",
        ] {
            let mut data = serve();
            data["Web"]["pc.example.ts.net:443"]["Handlers"]["/"]["Proxy"] = target.into();
            assert!(!valid_serve(&data, "pc.example.ts.net", 8766));
        }
        for extra in [
            serde_json::json!({"AllowFunnel":{"pc.example.ts.net:443":true}}),
            serde_json::json!({"Services":{"svc:other":{}}}),
            serde_json::json!({"Foreground":{"session":{}}}),
            serde_json::json!({"TCP":{"80":{"HTTP":true}}}),
        ] {
            let mut data = serve();
            for (key, value) in extra.as_object().unwrap() {
                data[key] = value.clone();
            }
            assert!(!valid_serve(&data, "pc.example.ts.net", 8766));
        }
        assert!(!valid_serve(&serve(), "other.example.ts.net", 8766));
        assert!(!valid_serve(
            &serde_json::json!({}),
            "pc.example.ts.net",
            8766
        ));
    }
    #[test]
    fn gate_defaults_closed_expires_and_recovers_without_clipboard_cache() {
        let gate = Gate::default();
        assert!(!gate.ready());
        gate.update(true);
        assert!(gate.ready());
        gate.0
            .send_modify(|s| s.checked = Some(Instant::now() - FRESH_FOR));
        assert!(!gate.ready());
        gate.update(true);
        gate.close();
        assert!(!gate.ready());
    }
    #[tokio::test]
    async fn simulated_sleep_and_reconnect_invalidate_old_authority_but_refresh_does_not() {
        let gate = Gate::default();
        gate.update(true);
        let original = gate.generation();
        gate.update(true);
        assert!(gate.valid(original));
        // Advance the observation's age directly: no wall-clock sleeping needed.
        gate.0
            .send_modify(|s| s.checked = Some(Instant::now() - FRESH_FOR - Duration::from_secs(1)));
        assert!(!gate.valid(original));
        gate.invalidated_since(original).await;
        gate.update(true);
        assert!(gate.ready());
        assert!(!gate.valid(original));
        let resumed = gate.generation();
        gate.close();
        gate.update(true);
        gate.invalidated_since(resumed).await;
        assert!(!gate.valid(resumed));
        assert!(gate.ready());
    }
}
