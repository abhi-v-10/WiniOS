//! Setup policy, non-overwriting preferences and read-only connection probes.
//! No Tailscale mutation, TLS bypass, pairing or clipboard content read.
use crate::{config::Config, native, settings, tailnet};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Networking::WinHttp::*,
        Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW},
    },
    core::PCWSTR,
};

#[derive(Clone, Debug)]
pub struct Profile {
    pub tailnet: bool,
    pub backend_port: u16,
    pub direct_endpoint: Option<String>,
    pub writes: bool,
}
impl Profile {
    pub fn new_tailnet(writes: bool) -> Self {
        Self {
            tailnet: true,
            backend_port: 8766,
            direct_endpoint: None,
            writes,
        }
    }
    pub(crate) fn from_config(config: Config) -> Self {
        Self {
            tailnet: config.tailnet_backend,
            backend_port: config.bind.port(),
            direct_endpoint: (!config.tailnet_backend).then(|| format!("https://{}", config.bind)),
            writes: config.allow_write,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Installation {
    New,
    Existing(Profile),
    ProtectedError(&'static str),
}
pub fn inspect_installation(path: &Path) -> Installation {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Installation::New,
        Err(_) => Installation::ProtectedError(
            "Existing settings cannot be inspected. They will not be replaced. Correct file access and reopen Setup.",
        ),
        Ok(_) => match settings::load(path) {
            Ok(config) => Installation::Existing(Profile::from_config(config)),
            Err(error) => Installation::ProtectedError(error),
        },
    }
}
/// File presence is enough to protect an existing installation from automatic
/// onboarding. Normal startup retains its full config/TLS validation and errors.
pub fn needs_first_run(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path),Err(error) if error.kind()==std::io::ErrorKind::NotFound)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Welcome,
    Client,
    Serve,
    Consent,
    Test,
    Iphone,
}
#[derive(Clone, Debug)]
pub struct Model {
    pub page: Page,
    pub installation: Option<Installation>,
    pub writes_consent: bool,
    pub passed: bool,
    pub busy: bool,
    pub applied: bool,
    pub message: String,
    pub endpoint: Option<String>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            page: Page::Welcome,
            installation: None,
            writes_consent: false,
            passed: false,
            busy: true,
            applied: false,
            message: "Checking saved configuration…".into(),
            endpoint: None,
        }
    }
}
impl Model {
    pub fn profile(&self) -> Profile {
        match &self.installation {
            Some(Installation::Existing(profile)) => profile.clone(),
            _ => Profile::new_tailnet(self.writes_consent),
        }
    }
    pub fn is_new(&self) -> bool {
        matches!(self.installation, Some(Installation::New)) && !self.applied
    }
    pub fn can_next(&self) -> bool {
        !self.busy
            && self.installation.is_some()
            && !matches!(self.installation, Some(Installation::ProtectedError(_)))
            && (matches!(self.page, Page::Welcome | Page::Consent | Page::Iphone) || self.passed)
    }
    pub fn can_back(&self) -> bool {
        !self.busy && self.page != Page::Welcome && (!self.applied || self.page == Page::Iphone)
    }
    pub fn heading(&self) -> &'static str {
        match self.page {
            Page::Welcome => "Welcome to WiniOS",
            Page::Client => "Connect your private network",
            Page::Serve => "Verify private HTTPS",
            Page::Consent => "Choose clipboard access",
            Page::Test => "Test your connection",
            Page::Iphone => "iPhone Setup",
        }
    }
    pub fn step(&self) -> &'static str {
        match self.page {
            Page::Welcome => "1 of 6 · Welcome",
            Page::Client => "2 of 6 · Tailscale",
            Page::Serve => "3 of 6 · HTTPS Serve",
            Page::Consent => "4 of 6 · Your consent",
            Page::Test => "5 of 6 · Connection test",
            Page::Iphone => "6 of 6 · iPhone Setup",
        }
    }
    pub fn next_label(&self) -> &'static str {
        match self.page {
            Page::Consent if self.is_new() => "Apply && test",
            Page::Consent => "Test connection",
            Page::Test => "iPhone setup",
            Page::Iphone => "Finish",
            _ => "Next",
        }
    }
    pub fn instructions(&self) -> String {
        if let Some(Installation::ProtectedError(error)) = &self.installation {
            return format!(
                "Existing configuration is protected.\r\n\r\n{error}\r\n\r\nNo file, credential, certificate or network setting was changed. Fix the saved settings using the README, then close and reopen Setup."
            );
        }
        let existing = matches!(self.installation, Some(Installation::Existing(_)));
        let profile = self.profile();
        match self.page {
            Page::Welcome=> if existing {"Your saved setup was found. This wizard is a protected review: it will not rewrite settings, credentials, certificates or Serve routes.\r\n\r\nThe agent keeps running while you review. No pairing or automatic Tailscale provisioning is performed.".into()} else {"Connect your Windows clipboard and iPhone over your private Tailscale network.\r\n\r\n1. Install/sign in to Tailscale on both devices.\r\n2. Review private HTTPS Serve and your device's verified endpoint.\r\n3. Choose read-only or explicitly allow clipboard writes, then Apply.\r\n4. Test HTTPS and connect your own Smart Send/Receive Shortcuts.\r\n\r\nThe first normal agent start creates a unique 256-bit local bearer credential in Windows Credential Manager, only if one is missing. No hostname or token is built into the app.\r\n\r\nWiniOS never signs in, configures Serve/Funnel, installs trust, or changes firewall rules. Cancel before Apply leaves settings untouched.".into()},
            Page::Client if !profile.tailnet=>"This saved installation uses direct-LAN HTTPS. Tailscale is not required for this mode. Its existing IP binding and certificate are preserved; this wizard will not switch modes.".into(),
            Page::Client=>"Install the official Tailscale client on Windows and iPhone, sign in to your approved tailnet, and connect both devices.\r\n\r\nDownload: https://tailscale.com/download/windows\r\n\r\nComplete sign-in/approval in Tailscale yourself, then choose Check again. This page runs read-only status checks in the background.".into(),
            Page::Serve if !profile.tailnet=>"Direct-LAN HTTPS is already configured. No Serve route or certificate will be created, replaced or changed.\r\n\r\nThe final test uses normal Windows certificate validation. If only your iPhone trusts the private LAN CA, the Windows-side test may fail without changing the working service.".into(),
            Page::Serve=>format!("{}\r\n\r\nEnable MagicDNS and HTTPS certificates in your tailnet only after reviewing Tailscale's prerequisites and certificate-transparency notice. WiniOS discovers this device's verified HTTPS name; do not substitute a router IP or another device's hostname.\r\n\r\nReview existing routes with tailscale serve status. With your explicit approval, configure this dedicated private proxy yourself:\r\n\r\ntailscale serve --bg --https=443 http://127.0.0.1:{}\r\n\r\nUse Serve, never Funnel. Do not overwrite other routes or use public/LAN HTTP. WiniOS only checks status and never executes this command.",self.endpoint.as_deref().unwrap_or("Waiting for a verified private endpoint. Follow the recovery step below, then Check again."),profile.backend_port),
            Page::Consent if existing=>format!("Saved configuration is protected. Clipboard writes are currently {}. The checkbox below is read-only for an existing installation.\r\n\r\nNo permissions will be changed by reopening Setup. To change saved permissions deliberately, use Settings → Clipboard, confirm your choice, then Apply & restart.\r\n\r\nBoth clipboard GET and POST always require your bearer token. Existing credentials are reused, never rotated by Setup.",if profile.writes {"enabled"} else {"disabled"}),
            Page::Consent=>"Every Windows → iPhone read requires bearer authentication.\r\n\r\nOptionally allow authenticated iPhone requests to REPLACE the Windows clipboard. The checkbox starts OFF. Leave it unchecked for read-only sharing.\r\n\r\nApply creates only 127.0.0.1 backend preferences, then starts the normal agent. Its first start securely generates a unique 256-bit bearer token only if none exists; an existing Credential Manager token is preserved. Never share or publish the token.\r\n\r\nNo Tailscale route, trust certificate or startup registration changes. Closing after Apply does not undo an applied choice. Images stay opt-in in Settings.".into(),
            Page::Test=>format!("The background test checks HTTPS certificate/hostname trust, WiniOS health and bearer authentication. It sends a deliberately invalid POST that cannot write clipboard text; it never reads clipboard contents.\r\n\r\n{}\r\n\r\nFor iPhone Shortcuts use the verified HTTPS URL with Authorization: Bearer <your token>. Tokens stay in Windows Credential Manager; use the README's explicit export workflow. This wizard never displays or logs a token.\r\n\r\nThe laptop must remain awake, connected and running WiniOS.",self.endpoint.as_deref().unwrap_or("Waiting for a verified endpoint.")),
            Page::Iphone=>crate::iphone::pairing_instructions(self.endpoint.as_deref(),profile.tailnet,profile.writes),
        }
    }
}

pub fn create_new_preferences(
    path: &Path,
    writes: bool,
    cancel: &AtomicBool,
) -> Result<Profile, &'static str> {
    settings::validate_local_absolute(path)?;
    if cancel.load(Ordering::Acquire) {
        return Err("Setup cancelled.");
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => {
            return Err(
                "Settings appeared or cannot be inspected. They were not overwritten. Close and reopen Setup.",
            );
        }
    }
    let parent = path.parent().ok_or("Missing settings directory.")?;
    std::fs::create_dir_all(parent)
        .map_err(|_| "Could not create the per-user settings directory.")?;
    let parent = parent
        .canonicalize()
        .map_err(|_| "Could not resolve the settings directory.")?;
    settings::validate_local_absolute(&parent)?;
    let target = parent.join(path.file_name().ok_or("Invalid settings filename.")?);
    let temp = parent.join(format!(
        ".winios-setup-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "System time is invalid.")?
            .as_nanos()
    ));
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|_| "Could not stage settings.")?;
    let _cleanup = Temp(temp.clone());
    let bytes = serde_json::to_vec_pretty(
        &serde_json::json!({"mode":"tailnet-backend","port":8766,"allow_clipboard_write":writes}),
    )
    .map_err(|_| "Could not encode settings.")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save settings.")?;
    drop(file);
    if cancel.load(Ordering::Acquire) {
        return Err("Setup cancelled.");
    }
    let wide = |p: &Path| -> Result<Vec<u16>, &'static str> {
        Ok(p.to_str()
            .ok_or("Invalid Unicode settings path.")?
            .encode_utf16()
            .chain([0])
            .collect())
    };
    let source = wide(&temp)?;
    let destination = wide(&target)?;
    // No REPLACE_EXISTING flag: an existing/dangling-link destination is protected.
    unsafe{MoveFileExW(PCWSTR(source.as_ptr()),PCWSTR(destination.as_ptr()),MOVEFILE_WRITE_THROUGH)}
        .map_err(|_|"Settings could not be committed. Existing files were not replaced; close and reopen Setup.")?;
    Ok(Profile::new_tailnet(writes))
}

#[derive(Clone)]
pub struct Check {
    pub passed: bool,
    pub message: String,
    pub endpoint: Option<String>,
}
impl Check {
    fn ok(message: &str) -> Self {
        Self {
            passed: true,
            message: message.into(),
            endpoint: None,
        }
    }
    fn fail(message: &str) -> Self {
        Self {
            passed: false,
            message: message.into(),
            endpoint: None,
        }
    }
}
pub async fn check_client(profile: &Profile) -> Check {
    if !profile.tailnet {
        return Check::ok("Direct-LAN mode preserved; Tailscale is not required.");
    }
    match tailnet::setup_client().await {
        Ok(()) => Check::ok("Tailscale is installed, signed in and connected."),
        Err(error) => Check::fail(error),
    }
}
pub async fn check_serve(profile: &Profile) -> Check {
    if !profile.tailnet {
        return Check {
            passed: true,
            message: "Existing direct-LAN HTTPS configuration is preserved.".into(),
            endpoint: profile.direct_endpoint.clone(),
        };
    }
    match tailnet::setup_serve(profile.backend_port).await {
        Ok(endpoint) => Check {
            passed: true,
            message: format!(
                "Verified private HTTPS Serve and this device's DNS identity: {endpoint}. No network settings were changed."
            ),
            endpoint: Some(endpoint),
        },
        Err(error) => Check::fail(error),
    }
}

struct Internet(*mut std::ffi::c_void);
impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn request(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    token: Option<&str>,
) -> Result<(u32, Vec<u8>), &'static str> {
    unsafe {
        let session = Internet(WinHttpOpen(
            windows::core::w!("WiniOS setup"),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err("Could not initialize the Windows HTTPS client.");
        }
        WinHttpSetTimeouts(session.0, 2000, 2000, 2000, 2000)
            .map_err(|_| "Could not set HTTPS timeouts.")?;
        let protocols = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        if WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            Some(&protocols.to_le_bytes()),
        )
        .is_err()
        {
            WinHttpSetOption(
                Some(session.0),
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                Some(&WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2.to_le_bytes()),
            )
            .map_err(|_| "TLS 1.2 or newer is required.")?;
        }
        let host = wide(host);
        let connect = Internet(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), port, 0));
        if connect.0.is_null() {
            return Err("Could not connect to the verified HTTPS host.");
        }
        let verb = wide(method);
        let resource = wide(path);
        let request = Internet(WinHttpOpenRequest(
            connect.0,
            PCWSTR(verb.as_ptr()),
            PCWSTR(resource.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err("Could not create the HTTPS request.");
        }
        // Never send ambient Windows credentials in response to a challenge.
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_AUTOLOGON_POLICY,
            Some(&WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH.to_le_bytes()),
        )
        .map_err(|_| "Could not disable automatic Windows authentication.")?;
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_le_bytes()),
        )
        .map_err(|_| "Could not disable HTTPS redirects.")?;
        let headers = token.map(|token| {
            wide(&format!(
                "Authorization: Bearer {token}\r\nContent-Type: application/json\r\n"
            ))
        });
        let body = b"{\"text\":null}";
        let post = method == "POST";
        WinHttpSendRequest(request.0,headers.as_deref().map(|h|&h[..h.len()-1]),post.then_some(body.as_ptr().cast()),if post{body.len() as u32}else{0},if post{body.len() as u32}else{0},0)
            .and_then(|_|WinHttpReceiveResponse(request.0,std::ptr::null_mut()))
            .map_err(|_|"HTTPS failed. Check reachability, DNS, certificate hostname/expiry, Windows trust and time. TLS verification stays enabled; no HTTP fallback is used.")?;
        let mut status = 0u32;
        let mut size = 4;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|_| "Could not read HTTPS status.")?;
        let mut bytes = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        if method == "GET" {
            loop {
                if std::time::Instant::now() >= deadline {
                    return Err("HTTPS health response timed out.");
                }
                let mut buffer = [0u8; 1024];
                let mut count = 0;
                WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut count,
                )
                .map_err(|_| "HTTPS health response was interrupted.")?;
                if count == 0 {
                    break;
                }
                if bytes.len() + count as usize > 4096 {
                    return Err("Unexpected health response size.");
                }
                bytes.extend_from_slice(&buffer[..count as usize]);
            }
        }
        Ok((status, bytes))
    }
}
pub async fn test_connection(profile: &Profile, cancel: &AtomicBool) -> Check {
    let verified = check_serve(profile).await;
    if !verified.passed {
        return verified;
    }
    let Some(endpoint) = verified.endpoint else {
        return Check::fail("No verified HTTPS endpoint is available.");
    };
    let Some(authority) = endpoint.strip_prefix("https://") else {
        return Check::fail("HTTPS is required.");
    };
    let (host, port) = if profile.tailnet {
        (authority, 443)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (
                host,
                match port.parse::<u16>() {
                    Ok(port) => port,
                    Err(_) => return Check::fail("Invalid saved HTTPS port."),
                },
            ),
            None => return Check::fail("Invalid saved HTTPS endpoint."),
        }
    };
    if host.is_empty() || host.contains(['/', '?', '#', '@']) {
        return Check::fail("Invalid HTTPS identity.");
    }
    if cancel.load(Ordering::Acquire) {
        return Check::fail("Setup cancelled.");
    }
    let (status, body) = match request(host, port, "GET", "/v1/health", None) {
        Ok(value) => value,
        Err(error) => return Check::fail(error),
    };
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    if status != 200 || health["service"] != "winios-agent" || health["status"] != "ok" {
        return Check::fail(
            "HTTPS responded, but WiniOS is not ready. Wait for the agent to start, check tray status, then Check again. No clipboard data was accessed.",
        );
    }
    if cancel.load(Ordering::Acquire) {
        return Check::fail("Setup cancelled.");
    }
    match request(host, port, "POST", "/v1/clipboard", None) {
        Ok((401, _)) => {}
        _ => {
            return Check::fail(
                "The clipboard endpoint did not enforce bearer authentication. No credential was sent. Check your Serve target.",
            );
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Check::fail("Setup cancelled.");
    }
    let token = match native::read_token() {
        Ok(Some(token)) => token,
        _ => {
            return Check::fail(
                "The agent's bearer credential is unavailable. Start the normal agent first. Setup does not replace credentials.",
            );
        }
    };
    let status = match request(host, port, "POST", "/v1/clipboard", Some(token.secret())) {
        Ok((status, _)) => status,
        Err(error) => return Check::fail(error),
    };
    let message = match status {
        422 => {
            "HTTPS trust, WiniOS health and bearer authentication verified. The invalid test payload was rejected; no clipboard data was read or written."
        }
        403 => {
            "HTTPS trust, health and bearer authentication verified. Clipboard writes remain disabled, as configured. No clipboard data was read or written."
        }
        503 => {
            "HTTPS health and bearer authentication verified; sharing is paused or the connection changed. Check tray status: Resume if paused, otherwise wait for the connection or Retry. No clipboard data was read or written."
        }
        401 => {
            return Check::fail(
                "Bearer authentication was rejected. Credentials were not changed. Check that this is your running WiniOS agent.",
            );
        }
        _ => {
            return Check::fail(
                "Unexpected authentication-probe response. No success is assumed; review the running agent and retry.",
            );
        }
    };
    Check {
        passed: true,
        message: message.into(),
        endpoint: Some(endpoint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "winios-setup-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn missing_only_triggers_setup_and_writes_default_off_with_explicit_consent() {
        for consent in [false, true] {
            let path = temp();
            assert!(needs_first_run(&path));
            create_new_preferences(&path, consent, &AtomicBool::new(false)).unwrap();
            assert!(!needs_first_run(&path));
            let config = settings::load(&path).unwrap();
            assert_eq!(config.allow_write, consent);
            assert!(config.tailnet_backend && !config.insecure_http);
            assert_eq!(config.bind.to_string(), "127.0.0.1:8766");
            std::fs::remove_file(path).unwrap();
        }
    }
    #[test]
    fn existing_or_cancelled_settings_are_never_overwritten() {
        let path = temp();
        let original = b"existing protected bytes";
        std::fs::write(&path, original).unwrap();
        assert!(!needs_first_run(&path));
        assert!(create_new_preferences(&path, true, &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(matches!(
            inspect_installation(&path),
            Installation::ProtectedError(_)
        ));
        std::fs::remove_file(&path).unwrap();
        assert!(create_new_preferences(&path, true, &AtomicBool::new(true)).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn model_guards_steps_and_existing_permissions() {
        let mut model = Model::default();
        assert!(!model.can_next());
        assert!(!model.writes_consent);
        model.installation = Some(Installation::New);
        model.busy = false;
        assert!(model.can_next());
        model.page = Page::Serve;
        assert!(!model.can_next());
        model.passed = true;
        assert!(model.can_next());
        model.installation = Some(Installation::Existing(Profile::new_tailnet(true)));
        model.page = Page::Consent;
        assert!(!model.is_new());
        assert!(model.profile().writes);
        assert!(model.instructions().contains("protected"));
        model.installation = Some(Installation::ProtectedError("invalid settings"));
        assert!(!model.can_next());
    }

    #[test]
    fn iphone_step_follows_connection_testing_and_keeps_existing_permissions() {
        let mut model = Model {
            busy: false,
            installation: Some(Installation::Existing(Profile::new_tailnet(false))),
            page: Page::Test,
            ..Model::default()
        };
        assert!(!model.can_next());
        assert_eq!(model.next_label(), "iPhone setup");
        model.passed = true;
        assert!(model.can_next());
        model.page = Page::Iphone;
        model.applied = true;
        model.passed = false;
        assert!(model.can_next() && model.can_back());
        assert_eq!(model.next_label(), "Finish");
        assert!(!model.profile().writes);
        let instructions = model.instructions();
        for phrase in [
            "Import Questions",
            "not been verified",
            "Tailscale connected",
            "WiniOS HTTPS URL",
            "disposable text",
        ] {
            assert!(instructions.contains(phrase));
        }
        assert!(matches!(
            model.installation,
            Some(Installation::Existing(_))
        ));
    }

    #[test]
    fn new_settings_commit_is_atomic_under_competing_setup_attempts() {
        let path = temp();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let jobs = [false, true].map(|consent| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (
                    consent,
                    create_new_preferences(&path, consent, &AtomicBool::new(false)),
                )
            })
        });
        let results = jobs.map(|job| job.join().unwrap());
        assert_eq!(
            results.iter().filter(|(_, result)| result.is_ok()).count(),
            1
        );
        let expected = results.iter().find(|(_, result)| result.is_ok()).unwrap().0;
        assert_eq!(settings::load(&path).unwrap().allow_write, expected);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn windows_https_probe_rejects_untrusted_certificate_without_downgrade() {
        use std::{
            io::Write,
            net::TcpListener,
            sync::Arc,
            time::{Duration, Instant},
        };
        crate::transport::install_crypto_provider();
        let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(
                    certificate.signing_key.serialize_der(),
                )
                .into(),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let accepted = Arc::new(AtomicBool::new(false));
        let observed = accepted.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        observed.store(true, Ordering::Release);
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
                        let mut stream = rustls::StreamOwned::new(connection, stream);
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => return,
                }
            }
        });
        assert!(request("127.0.0.1", port, "GET", "/v1/health", None).is_err());
        server.join().unwrap();
        assert!(
            accepted.load(Ordering::Acquire),
            "the test must reach the TLS handshake"
        );
    }

    #[test]
    #[ignore = "Read-only live HTTPS/Tailscale/WCM probe; requires an already running configured agent."]
    fn existing_setup_read_only_https_probe() {
        let path = settings::default_path().unwrap();
        let before = std::fs::read(&path).unwrap();
        let Installation::Existing(profile) = inspect_installation(&path) else {
            panic!("Existing settings required.")
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let check = runtime.block_on(test_connection(&profile, &AtomicBool::new(false)));
        assert!(check.passed, "{}", check.message);
        assert_eq!(std::fs::read(path).unwrap(), before);
        println!(
            "PASS: verified HTTPS, health and required bearer authentication. Existing settings unchanged; clipboard not read or written."
        );
    }
}
