//! Secret-free preferences: direct HTTPS or an explicit loopback Serve backend.
use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Component, Path, PathBuf},
};
const MAX_SETTINGS_BYTES: u64 = 16 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    #[serde(default)]
    mode: Mode,
    lan_ip: Option<Ipv4Addr>,
    port: Option<u16>,
    #[serde(default)]
    allow_clipboard_write: bool,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
}
#[derive(Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Mode {
    #[default]
    DirectLan,
    TailnetBackend,
}

pub fn default_path() -> Result<PathBuf, &'static str> {
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or("LOCALAPPDATA is unavailable. Supply --config with a local absolute path.")?,
    )
    .join("WiniOS")
    .join("settings.json"))
}

pub fn validate_local_absolute(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Configuration and TLS paths must be local absolute paths without '..'.");
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err("UNC, network and device paths are not accepted.");
        }
    }
    Ok(())
}

fn decode(bytes: &[u8]) -> Result<Config, &'static str> {
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err("Tray settings exceed 16 KiB.");
    }
    // Never display serde's diagnostics: an accidental secret field/value must
    // not be echoed in a dialog or log.
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let preferences: Preferences = serde_json::from_slice(bytes).map_err(
        |_| "Invalid settings.json. Use the documented schema; unknown fields are rejected.",
    )?;
    let tailnet_backend = preferences.mode == Mode::TailnetBackend;
    let port = preferences
        .port
        .unwrap_or(if tailnet_backend { 8766 } else { 8765 });
    let ip = if tailnet_backend {
        if preferences.lan_ip.is_some()
            || preferences.tls_cert.is_some()
            || preferences.tls_key.is_some()
            || port == 8765
        {
            return Err(
                "Tailnet backend requires a separate loopback port and no LAN IP or TLS identity settings.",
            );
        }
        Ipv4Addr::LOCALHOST
    } else {
        preferences
            .lan_ip
            .ok_or("Direct LAN settings require lan_ip.")?
    };
    if (!tailnet_backend && !ip.is_private()) || port == 0 {
        return Err("Settings require a specific private IPv4 address and a port from 1 to 65535.");
    }
    if preferences.tls_cert.is_some() != preferences.tls_key.is_some() {
        return Err(
            "Set both tls_cert and tls_key, or omit both for the existing default identity.",
        );
    }
    for path in [&preferences.tls_cert, &preferences.tls_key]
        .into_iter()
        .flatten()
    {
        validate_local_absolute(path)?;
    }
    Ok(Config {
        bind: SocketAddrV4::new(ip, port),
        allow_write: preferences.allow_clipboard_write,
        tls_cert: preferences.tls_cert,
        tls_key: preferences.tls_key,
        debug: false,
        show_token: false,
        help: false,
        insecure_http: false,
        tailnet_backend,
        images: crate::image::Options::default(),
    })
}

pub fn load(path: &Path) -> Result<Config, &'static str> {
    validate_local_absolute(path)?;
    let resolved_path = path.canonicalize().map_err(|_| "Tray settings are missing or inaccessible. Create %LOCALAPPDATA%\\WiniOS\\settings.json using the README.")?;
    validate_local_absolute(&resolved_path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&resolved_path).map_err(|_| "Could not read tray settings. Create %LOCALAPPDATA%\\WiniOS\\settings.json using the README.")?
        .take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes).map_err(|_| "Could not read tray settings.")?;
    let mut config = decode(&bytes)?;
    config.images = load_images(&resolved_path)?.0;
    if config.tailnet_backend {
        return Ok(config);
    }
    let (cert, key) = config.tls_paths()?.ok_or("Tray mode requires HTTPS.")?;
    // Canonicalize local files before loading, rejecting a link to a UNC share.
    for path in [&cert, &key] {
        validate_local_absolute(path)?;
    }
    let cert = cert.canonicalize().map_err(
        |_| "TLS certificate is missing or inaccessible. Existing identity was not changed.",
    )?;
    let key = key.canonicalize().map_err(
        |_| "TLS private key is missing or inaccessible. Existing identity was not changed.",
    )?;
    for path in [&cert, &key] {
        validate_local_absolute(path)?;
        if !path.is_file() {
            return Err("TLS identity paths must refer to local files.");
        }
    }
    config.tls_cert = Some(cert);
    config.tls_key = Some(key);
    Ok(config)
}

/// Desktop-only options stay separate so older agents can still read settings.json.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopOptions {
    #[serde(default)]
    pub open_settings_on_launch: bool,
}
pub fn desktop_path(path: &Path) -> PathBuf {
    path.with_file_name("desktop.json")
}
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, &'static str> {
    validate_local_absolute(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Settings could not be read.")?
        .take(MAX_SETTINGS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Settings could not be read.")?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err("Settings exceed 16 KiB.");
    }
    Ok(bytes)
}
pub fn write_permission(bytes: &[u8]) -> Result<bool, &'static str> {
    Ok(decode(bytes)?.allow_write)
}
pub fn load_desktop(path: &Path) -> Result<(DesktopOptions, Option<Vec<u8>>), &'static str> {
    let path = desktop_path(path);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((DesktopOptions::default(), None))
        }
        Err(_) => Err("Desktop preferences are inaccessible; they were not replaced."),
        Ok(_) => {
            let bytes = read_bounded(&path)?;
            let options = serde_json::from_slice(&bytes).map_err(
                |_| "Invalid desktop.json. Correct it manually; no defaults were written over it.",
            )?;
            Ok((options, Some(bytes)))
        }
    }
}
/// Separate additive preferences keep older agents' strict settings.json usable.
pub fn load_images(path: &Path) -> Result<(crate::image::Options, Option<Vec<u8>>), &'static str> {
    let path = path.with_file_name("images.json");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((crate::image::Options::default(), None))
        }
        Err(_) => Err("Image preferences are inaccessible; they were not replaced."),
        Ok(_) => {
            let bytes = read_bounded(&path)?;
            let options: crate::image::Options = serde_json::from_slice(&bytes)
                .map_err(|_| "Invalid images.json. No preferences were replaced.")?;
            Ok((options.validate()?, Some(bytes)))
        }
    }
}
#[cfg(windows)]
pub fn save_images(
    path: &Path,
    expected: Option<&[u8]>,
    options: crate::image::Options,
) -> Result<(), &'static str> {
    options.validate()?;
    let bytes =
        serde_json::to_vec_pretty(&options).map_err(|_| "Could not encode image preferences.")?;
    atomic_save(&path.with_file_name("images.json"), expected, &bytes)
}
#[cfg(windows)]
pub(crate) fn atomic_save(
    path: &Path,
    expected: Option<&[u8]>,
    bytes: &[u8],
) -> Result<(), &'static str> {
    use std::io::Write;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACE_FILE_FLAGS, ReplaceFileW,
        },
        core::PCWSTR,
    };
    validate_local_absolute(path)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err("Settings exceed 16 KiB.");
    }
    let parent = path
        .parent()
        .ok_or("Missing settings directory.")?
        .canonicalize()
        .map_err(|_| "Settings directory is unavailable.")?;
    validate_local_absolute(&parent)?;
    let target = if expected.is_some() {
        path.canonicalize()
            .map_err(|_| "Settings disappeared; reload before saving.")?
    } else {
        parent.join(path.file_name().ok_or("Invalid settings filename.")?)
    };
    validate_local_absolute(&target)?;
    let parent = target.parent().ok_or("Invalid settings directory.")?;
    let lock_path = parent.join(".winios-preferences.lock");
    struct Temporary(PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    struct SaveLock {
        file: Option<std::fs::File>,
        path: PathBuf,
    }
    impl Drop for SaveLock {
        fn drop(&mut self) {
            drop(self.file.take());
            let _ = std::fs::remove_file(&self.path);
        }
    }
    let lock_file=std::fs::OpenOptions::new().create_new(true).write(true).open(&lock_path).map_err(|_|"Another settings save is active. Retry; inspect a stale .winios-preferences.lock only while WiniOS is stopped.")?;
    let _save_lock = SaveLock {
        file: Some(lock_file),
        path: lock_path,
    };
    let matches = || match expected {
        Some(expected) => read_bounded(&target).is_ok_and(|current| current == expected),
        None => {
            matches!(std::fs::symlink_metadata(&target),Err(error) if error.kind()==std::io::ErrorKind::NotFound)
        }
    };
    if !matches() {
        return Err(
            "Settings changed outside this window. Reload before saving; no file was overwritten.",
        );
    }
    let temp = parent.join(format!(
        ".winios-preferences-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid system clock.")?
            .as_nanos()
    ));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|_| "Could not stage settings.")?;
    let _cleanup = Temporary(temp.clone());
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not flush settings.")?;
    drop(file);
    if !matches() {
        return Err("Settings changed before commit. Reload before saving.");
    }
    let wide = |p: &Path| -> Result<Vec<u16>, &'static str> {
        Ok(p.to_str()
            .ok_or("Settings path must be valid Unicode.")?
            .encode_utf16()
            .chain([0])
            .collect())
    };
    let dest = wide(&target)?;
    let source = wide(&temp)?;
    unsafe {
        if expected.is_some() {
            // ReplaceFile preserves the replaced file's ACL/security metadata.
            ReplaceFileW(
                PCWSTR(dest.as_ptr()),
                PCWSTR(source.as_ptr()),
                PCWSTR::null(),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        } else {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(dest.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
    }
    .map_err(|_| "Atomic settings commit failed. Existing preferences were retained.")
}
#[cfg(windows)]
pub fn save_write_permission(
    path: &Path,
    expected: &[u8],
    enabled: bool,
) -> Result<(), &'static str> {
    decode(expected)?;
    let source = expected
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(expected);
    let mut value: serde_json::Value =
        serde_json::from_slice(source).map_err(|_| "Invalid settings.")?;
    value["allow_clipboard_write"] = enabled.into();
    let bytes = serde_json::to_vec_pretty(&value).map_err(|_| "Could not encode settings.")?;
    decode(&bytes)?;
    atomic_save(path, Some(expected), &bytes)
}
#[cfg(windows)]
pub fn save_desktop(
    path: &Path,
    expected: Option<&[u8]>,
    options: DesktopOptions,
) -> Result<(), &'static str> {
    let bytes =
        serde_json::to_vec_pretty(&options).map_err(|_| "Could not encode desktop preferences.")?;
    atomic_save(&desktop_path(path), expected, &bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn image_preferences_are_opt_in_bounded_atomic_and_leave_legacy_settings_unchanged() {
        let directory = std::env::temp_dir().join(format!(
            "winios-images-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("settings.json");
        let original = br#"{"mode":"tailnet-backend","allow_clipboard_write":true}"#;
        std::fs::write(&path, original).unwrap();
        assert_eq!(
            load_images(&path).unwrap(),
            (crate::image::Options::default(), None)
        );
        assert!(!path.with_file_name("images.json").exists());
        let options = crate::image::Options {
            enabled: true,
            max_mib: 4,
        };
        save_images(&path, None, options).unwrap();
        assert_eq!(load(&path).unwrap().images, options);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(save_images(&path, None, options).is_err());
        assert!(
            save_images(
                &path,
                load_images(&path).unwrap().1.as_deref(),
                crate::image::Options {
                    enabled: true,
                    max_mib: 255
                }
            )
            .is_err()
        );
        std::fs::write(path.with_file_name("images.json"), b"{invalid").unwrap();
        assert!(load_images(&path).is_err());
        std::fs::remove_file(path.with_file_name("images.json")).unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn desktop_and_write_preferences_commit_atomically_preserve_fields_and_reject_stale_edits() {
        let directory = std::env::temp_dir().join(format!(
            "winios-preferences-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("settings.json");
        let original = br#"{"mode":"tailnet-backend","port":12345,"allow_clipboard_write":true}"#;
        std::fs::write(&path, original).unwrap();
        let (default, bytes) = load_desktop(&path).unwrap();
        assert!(!default.open_settings_on_launch && bytes.is_none());
        assert!(!desktop_path(&path).exists());
        save_write_permission(&path, original, false).unwrap();
        let saved = read_bounded(&path).unwrap();
        let config = decode(&saved).unwrap();
        assert!(!config.allow_write && config.tailnet_backend && !config.insecure_http);
        assert_eq!(config.bind.port(), 12345);
        assert!(save_write_permission(&path, original, true).is_err());
        assert_eq!(read_bounded(&path).unwrap(), saved);
        save_desktop(
            &path,
            None,
            DesktopOptions {
                open_settings_on_launch: true,
            },
        )
        .unwrap();
        assert!(save_desktop(&path, None, DesktopOptions::default()).is_err());
        let (options, expected) = load_desktop(&path).unwrap();
        assert!(options.open_settings_on_launch);
        save_desktop(&path, expected.as_deref(), DesktopOptions::default()).unwrap();
        assert!(!load_desktop(&path).unwrap().0.open_settings_on_launch);
        assert_eq!(read_bounded(&path).unwrap(), saved);
        assert!(!directory.join(".winios-preferences.lock").exists());
        std::fs::write(desktop_path(&path), b"{invalid").unwrap();
        assert!(load_desktop(&path).is_err());
        assert!(save_desktop(&path, expected.as_deref(), DesktopOptions::default()).is_err());
        assert_eq!(std::fs::read(desktop_path(&path)).unwrap(), b"{invalid");
        std::fs::remove_file(desktop_path(&path)).unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn tailnet_settings_are_explicit_loopback_only_without_certificate_changes() {
        let config = decode(br#"{"mode":"tailnet-backend","allow_clipboard_write":true}"#).unwrap();
        assert!(config.tailnet_backend && config.allow_write && !config.insecure_http);
        assert_eq!(config.bind, "127.0.0.1:8766".parse().unwrap());
        assert!(
            !decode(br#"{"mode":"tailnet-backend"}"#)
                .unwrap()
                .allow_write
        );
        for bytes in [
            br#"{"mode":"tailnet-backend","lan_ip":"192.168.1.7"}"#.as_slice(),
            br#"{"mode":"tailnet-backend","port":8765}"#,
            br#"{"mode":"tailnet-backend","port":0}"#,
            br#"{"mode":"tailnet-backend","tls_cert":"C:\\identity.pem"}"#,
            br#"{"mode":"tailnet-backend","insecure_http":true}"#,
        ] {
            assert!(decode(bytes).is_err());
        }
        assert!(
            !decode(br#"{"lan_ip":"192.168.1.7"}"#)
                .unwrap()
                .tailnet_backend
        );
    }
    #[test]
    fn tray_settings_are_https_only_private_and_writes_opt_in() {
        let config = decode(br#"{"lan_ip":"192.168.1.7"}"#).unwrap();
        assert!(!config.insecure_http && !config.allow_write);
        assert!(
            decode(br#"{"lan_ip":"192.168.1.7","allow_clipboard_write":true}"#)
                .unwrap()
                .allow_write
        );
        for bytes in [
            br#"{"lan_ip":"0.0.0.0"}"#.as_slice(),
            br#"{"lan_ip":"8.8.8.8"}"#,
            br#"{"lan_ip":"192.168.1.7","insecure_http":true}"#,
            br#"{"lan_ip":"192.168.1.7","bearer_token":"must not echo this"}"#,
            br#"{"lan_ip":"192.168.1.7","tls_cert":"relative.pem","tls_key":"relative.key"}"#,
        ] {
            assert!(decode(bytes).is_err());
        }
        assert!(decode(&vec![b' '; MAX_SETTINGS_BYTES as usize + 1]).is_err());
    }
}
