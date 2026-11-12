//! Explicit, metadata-only public GitHub checks. No agent credentials or assets.
use serde::Deserialize;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Networking::WinHttp::*,
        System::Com::*,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{PCWSTR, w},
};

const MAX_RESPONSE: usize = 256 * 1024;
const MAX_NOTES: usize = 4000;
const OFFLINE: &str = "Update check failed. Check internet access, Windows certificate trust and time, then retry. No download or settings change occurred.";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Source(String);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    github_repository: String,
}
impl Source {
    fn parse(repository: &str) -> Result<Self, &'static str> {
        let parts: Vec<_> = repository.split('/').collect();
        if parts.len() != 2
            || parts.iter().any(|part| {
                part.is_empty()
                    || part.len() > 100
                    || !part.as_bytes()[0].is_ascii_alphanumeric()
                    || !part.as_bytes()[part.len() - 1].is_ascii_alphanumeric()
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
            })
            || parts[0].len() > 39
            || !parts[0]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(
                "Invalid updates.json repository. Use a trusted public GitHub owner/repository, not a URL or credential.",
            );
        }
        Ok(Self(repository.into()))
    }
    fn load(settings: &Path) -> Result<Option<Self>, &'static str> {
        let path = settings.with_file_name("updates.json");
        crate::settings::validate_local_absolute(&path)?;
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("Update source configuration is inaccessible."),
            Ok(_) => {
                let resolved = path
                    .canonicalize()
                    .map_err(|_| "Update source configuration is inaccessible.")?;
                let bytes = crate::settings::read_bounded(&resolved)?;
                let preferences: Preferences = serde_json::from_slice(&bytes)
                    .map_err(|_| "Invalid updates.json. Only github_repository is accepted; no credentials or URLs.")?;
                Self::parse(&preferences.github_repository).map(Some)
            }
        }
    }
    fn path(&self) -> String {
        format!("/repos/{}/releases/latest", self.0)
    }
    fn release_url(&self, tag: &str, url: &str) -> Result<(), &'static str> {
        // Exact HTTPS origin, repository identity and tag path. Reject ports,
        // userinfo, escapes, queries/fragments, assets and lookalike hosts.
        let prefix = format!("https://github.com/{}/releases/tag/", self.0);
        let Some(actual_prefix) = url.get(..prefix.len()) else {
            return Err("Release link failed validation. Nothing was opened.");
        };
        if !actual_prefix.eq_ignore_ascii_case(&prefix) || url.get(prefix.len()..) != Some(tag) {
            return Err("Release link failed validation. Nothing was opened.");
        }
        Ok(())
    }
}

/// Stable numeric release versions only; no coercion, prerelease or build suffix.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version([u32; 3]);
impl Version {
    fn parse(value: &str) -> Result<Self, &'static str> {
        let value = value.strip_prefix('v').unwrap_or(value);
        let mut result = [0; 3];
        let parts: Vec<_> = value.split('.').collect();
        if parts.len() != 3 {
            return Err("Unsupported release version. Expected stable X.Y.Z or vX.Y.Z.");
        }
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty()
                || part.len() > 10
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|b| b.is_ascii_digit())
            {
                return Err("Invalid release version.");
            }
            result[i] = part
                .parse()
                .map_err(|_| "Release version is out of range.")?;
        }
        Ok(Self(result))
    }
}

#[derive(Deserialize)]
struct Metadata {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
}
#[derive(Clone)]
pub struct Release {
    source: Source,
    tag: String,
    url: String,
    notes: String,
}
impl Release {
    pub(crate) fn notification_version(&self) -> Option<String> {
        let version = Version::parse(&self.tag).ok()?;
        let current = Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
        (version > current).then(|| format!("{}.{}.{}", version.0[0], version.0[1], version.0[2]))
    }
    pub fn describe(&self) -> String {
        let latest = Version::parse(&self.tag).expect("validated release version");
        let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("stable Cargo version");
        let status = if latest > current {
            "Update available"
        } else if latest == current {
            "You are up to date"
        } else {
            "Installed version is newer; no downgrade is suggested"
        };
        format!(
            "{status}\r\nInstalled: {} · Latest stable: {}\r\nTrusted source: {}\r\nOfficial release: {}\r\n\r\nRelease notes (plain text from the configured publisher):\r\n{}\r\n\r\nNo installer was downloaded. Review publisher identity and signing before any manual installation; metadata or hashes alone do not authenticate an executable.",
            env!("CARGO_PKG_VERSION"),
            self.tag,
            self.source.0,
            self.url,
            self.notes
        )
    }
}
pub(crate) fn source_configured(settings: &Path) -> bool {
    matches!(Source::load(settings), Ok(Some(_)))
}
pub(crate) fn source_still_matches(settings: &Path, release: &Release) -> bool {
    Source::load(settings).ok().flatten().as_ref() == Some(&release.source)
}
fn decode(source: Source, bytes: &[u8]) -> Result<Release, &'static str> {
    if bytes.len() > MAX_RESPONSE {
        return Err("Release metadata exceeds the size limit.");
    }
    let metadata: Metadata = serde_json::from_slice(bytes)
        .map_err(|_| "Invalid release metadata; no update result is trusted.")?;
    if metadata.draft || metadata.prerelease {
        return Err("Only published stable releases are supported.");
    }
    Version::parse(&metadata.tag_name)?;
    source.release_url(&metadata.tag_name, &metadata.html_url)?;
    let body = metadata
        .body
        .as_deref()
        .unwrap_or("No release notes supplied.");
    // Remote Markdown stays inert. Remove control/bidi characters from native
    // text so publisher text cannot disguise the separately validated link.
    let mut notes: String = body.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .filter(|c| !matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .take(MAX_NOTES).collect();
    if body.chars().count() > MAX_NOTES {
        notes.push_str("\r\n[Notes truncated; read the official release page.]");
    }
    Ok(Release {
        source,
        tag: metadata.tag_name,
        url: metadata.html_url,
        notes,
    })
}
fn status_ok(status: u32) -> Result<(), &'static str> {
    match status {
        200 => Ok(()),
        404 => Err(
            "No published release was found, or the repository is unavailable or private. Check that the configured repository is public, then try again.",
        ),
        403 | 429 => {
            Err("GitHub refused or rate-limited this check. Retry later; do not add credentials.")
        }
        300..=399 => Err(
            "Update source redirected. Check the configured repository; redirects are not followed.",
        ),
        _ => Err(OFFLINE),
    }
}
fn ready(cancel: &AtomicBool, deadline: Instant) -> Result<(), &'static str> {
    if cancel.load(Ordering::Acquire) {
        Err("Update check cancelled.")
    } else if Instant::now() >= deadline {
        Err("Update check timed out. Retry later; no download occurred.")
    } else {
        Ok(())
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
fn request(source: &Source, cancel: &AtomicBool) -> Result<Vec<u8>, &'static str> {
    let deadline = Instant::now() + Duration::from_secs(10);
    ready(cancel, deadline)?;
    unsafe {
        let session = Internet(WinHttpOpen(
            w!("WiniOS update check"),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(OFFLINE);
        }
        WinHttpSetTimeouts(session.0, 2000, 2000, 2000, 2000).map_err(|_| OFFLINE)?;
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
            .map_err(|_| OFFLINE)?;
        }
        let connect = Internet(WinHttpConnect(session.0, w!("api.github.com"), 443, 0));
        if connect.0.is_null() {
            return Err(OFFLINE);
        }
        let path = wide(&source.path());
        let request = Internet(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(OFFLINE);
        }
        for (option, value) in [
            (
                WINHTTP_OPTION_AUTOLOGON_POLICY,
                WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
            ),
            (
                WINHTTP_OPTION_REDIRECT_POLICY,
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
            ),
            (WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_DISABLE_COOKIES),
            (WINHTTP_OPTION_MAX_RESPONSE_HEADER_SIZE, 16 * 1024),
        ] {
            WinHttpSetOption(Some(request.0), option, Some(&value.to_le_bytes()))
                .map_err(|_| OFFLINE)?;
        }
        let headers = wide(
            "Accept: application/vnd.github+json\r\nAccept-Encoding: identity\r\nCache-Control: no-cache\r\n",
        );
        ready(cancel, deadline)?;
        WinHttpSendRequest(
            request.0,
            Some(&headers[..headers.len() - 1]),
            None,
            0,
            0,
            0,
        )
        .map_err(|_| OFFLINE)?;
        ready(cancel, deadline)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| OFFLINE)?;
        ready(cancel, deadline)?;
        let (mut status, mut size) = (0u32, 4u32);
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|_| OFFLINE)?;
        status_ok(status)?;
        let mut bytes = Vec::new();
        loop {
            ready(cancel, deadline)?;
            let mut buffer = [0u8; 4096];
            let mut count = 0;
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut count,
            )
            .map_err(|_| OFFLINE)?;
            if count == 0 {
                break;
            }
            if bytes.len() + count as usize > MAX_RESPONSE {
                return Err("Release metadata exceeds the size limit.");
            }
            bytes.extend_from_slice(&buffer[..count as usize]);
        }
        ready(cancel, deadline)?;
        Ok(bytes)
    }
}
pub fn check(settings: &Path, cancel: &AtomicBool) -> Result<Option<Release>, &'static str> {
    ready(cancel, Instant::now() + Duration::from_secs(10))?;
    let Some(source) = Source::load(settings)? else {
        return Ok(None);
    };
    let bytes = request(&source, cancel)?;
    let release = decode(source, &bytes)?;
    ready(cancel, Instant::now() + Duration::from_secs(10))?;
    Ok(Some(release))
}
pub fn open(
    settings: &Path,
    release: &Release,
    cancel: &AtomicBool,
) -> Result<&'static str, &'static str> {
    if Source::load(settings)?.as_ref() != Some(&release.source) {
        return Err("Update source changed. Check for updates again before opening a release.");
    }
    release.source.release_url(&release.tag, &release.url)?;
    ready(cancel, Instant::now() + Duration::from_secs(10))?;
    let url = wide(&release.url);
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            .ok()
            .map_err(|_| "Could not initialize browser launcher.")?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let result = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if result.0 as isize <= 32 {
            return Err(
                "Could not open the official release page. Use the displayed HTTPS link manually.",
            );
        }
    }
    Ok("Official release page opened. Any download and installation is your separate decision.")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Source {
        Source::parse("TrustedOwner/WiniOS").unwrap()
    }
    fn metadata(tag: &str, url: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"tag_name":tag,"html_url":url,"body":"Notes","draft":false,"prerelease":false})).unwrap()
    }
    #[test]
    fn source_is_only_a_public_github_repository_without_credentials_or_url() {
        assert_eq!(
            source().path(),
            "/repos/TrustedOwner/WiniOS/releases/latest"
        );
        for value in [
            "",
            "repo",
            "a/b/c",
            "https://github.com/a/b",
            "a/b?token=x",
            "a@b/c",
            "../repo",
            "a/..",
            "a/b\\c",
            "a/b%2fc",
            " a/b",
            "a/b\n",
            "a/b#c",
            "a/b:443",
        ] {
            assert!(Source::parse(value).is_err(), "{value:?}");
        }
    }
    #[test]
    fn versions_compare_numerically_and_reject_ambiguous_or_unstable_versions() {
        assert!(Version::parse("v0.10.0").unwrap() > Version::parse("0.9.99").unwrap());
        assert_eq!(
            Version::parse("v0.1.1").unwrap(),
            Version::parse("0.1.1").unwrap()
        );
        for value in [
            "",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.2.3-beta",
            "1.2.3+hash",
            "1.2.-3",
            " 1.2.3",
            "1.2.3\n",
            "4294967296.0.0",
            "V1.2.3",
        ] {
            assert!(Version::parse(value).is_err());
        }
    }
    #[test]
    fn release_url_is_https_and_bound_to_the_configured_repository_and_exact_tag() {
        let valid = "https://github.com/TrustedOwner/WiniOS/releases/tag/v0.2.0";
        assert!(decode(source(), &metadata("v0.2.0", valid)).is_ok());
        for url in [
            "http://github.com/TrustedOwner/WiniOS/releases/tag/v0.2.0",
            "https://github.com.evil/TrustedOwner/WiniOS/releases/tag/v0.2.0",
            "https://github.com@evil.test/TrustedOwner/WiniOS/releases/tag/v0.2.0",
            "https://github.com/Other/WiniOS/releases/tag/v0.2.0",
            "https://github.com/TrustedOwner/WiniOS/releases/tag/v0.1.0",
            "https://github.com/TrustedOwner/WiniOS/releases/tag/v0.2.0?x=1",
            "https://github.com/TrustedOwner/WiniOS/releases/download/v0.2.0/setup.exe",
            "file:///C:/setup.exe",
            "https://github.com:443/TrustedOwner/WiniOS/releases/tag/v0.2.0",
        ] {
            assert!(decode(source(), &metadata("v0.2.0", url)).is_err(), "{url}");
        }
    }
    #[test]
    fn metadata_is_bounded_inert_and_draft_or_malformed_results_fail_closed() {
        assert!(decode(source(), b"invalid json").is_err());
        assert!(decode(source(), &vec![b' '; MAX_RESPONSE + 1]).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&metadata(
            "v0.2.0",
            "https://github.com/TrustedOwner/WiniOS/releases/tag/v0.2.0",
        ))
        .unwrap();
        value["draft"] = true.into();
        assert!(decode(source(), &serde_json::to_vec(&value).unwrap()).is_err());
        value["draft"] = false.into();
        value["prerelease"] = true.into();
        assert!(decode(source(), &serde_json::to_vec(&value).unwrap()).is_err());
        value["prerelease"] = false.into();
        value["body"] = format!("\0\u{202e}{}", "é".repeat(5000)).into();
        let release = decode(source(), &serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(release.notes.contains("truncated"));
        assert!(!release.notes.contains(['\0', '\u{202e}']));
        assert!(release.describe().contains("Update available"));
    }
    #[test]
    fn offline_http_redirect_cancellation_and_timeout_results_cannot_be_success() {
        assert!(status_ok(200).is_ok());
        let missing = status_ok(404).unwrap_err();
        assert!(missing.contains("No published release"));
        assert!(missing.contains("unavailable or private"));
        for status in [301, 302, 307, 401, 403, 404, 429, 500] {
            assert!(status_ok(status).is_err());
        }
        let cancel = AtomicBool::new(true);
        assert!(ready(&cancel, Instant::now() + Duration::from_secs(1)).is_err());
        cancel.store(false, Ordering::Release);
        assert!(ready(&cancel, Instant::now() - Duration::from_secs(1)).is_err());
    }
    #[test]
    fn absent_or_invalid_sidecar_is_read_only_and_no_source_means_no_network() {
        let dir = std::env::temp_dir().join(format!("winios-updates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let cancel = AtomicBool::new(false);
        assert!(check(&path, &cancel).unwrap().is_none());
        assert!(!dir.join("updates.json").exists());
        let bytes = br#"{"github_repository":"owner/repo","token":"do-not-echo"}"#;
        std::fs::write(dir.join("updates.json"), bytes).unwrap();
        let error = check(&path, &cancel).err().unwrap();
        assert!(!error.contains("do-not-echo"));
        assert_eq!(std::fs::read(dir.join("updates.json")).unwrap(), bytes);
        std::fs::remove_file(dir.join("updates.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn equal_or_older_releases_never_offer_a_downgrade() {
        let current = env!("CARGO_PKG_VERSION");
        let url = format!("https://github.com/TrustedOwner/WiniOS/releases/tag/{current}");
        assert!(
            decode(source(), &metadata(current, &url))
                .unwrap()
                .describe()
                .contains("up to date")
        );
        let old = decode(
            source(),
            &metadata(
                "0.0.0",
                "https://github.com/TrustedOwner/WiniOS/releases/tag/0.0.0",
            ),
        )
        .unwrap();
        assert!(old.describe().contains("no downgrade is suggested"));
    }
    #[test]
    fn changed_configuration_and_cancellation_refuse_browser_launch() {
        let dir = std::env::temp_dir().join(format!("winios-update-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let release = decode(
            source(),
            &metadata(
                "v0.2.0",
                "https://github.com/TrustedOwner/WiniOS/releases/tag/v0.2.0",
            ),
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        std::fs::write(
            dir.join("updates.json"),
            br#"{"github_repository":"Different/Publisher"}"#,
        )
        .unwrap();
        assert!(
            open(&path, &release, &cancel)
                .unwrap_err()
                .contains("source changed")
        );
        std::fs::write(
            dir.join("updates.json"),
            br#"{"github_repository":"TrustedOwner/WiniOS"}"#,
        )
        .unwrap();
        cancel.store(true, Ordering::Release);
        assert!(
            open(&path, &release, &cancel)
                .unwrap_err()
                .contains("cancelled")
        );
        std::fs::remove_file(dir.join("updates.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
