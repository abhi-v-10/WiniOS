//! Explicit per-user desktop integration. Startup is never enabled on launch.
use crate::{native::AppError, settings};
use std::path::{Path, PathBuf};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, PROPERTYKEY},
        System::{
            Com::{StructuredStorage::*, *},
            Registry::*,
        },
        UI::{
            Shell::{PropertiesSystem::IPropertyStore, *},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{GUID, Interface, PCWSTR, w},
};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("WiniOS");
pub const APP_ID: PCWSTR = w!("WiniOS.Desktop");
const APP_ID_KEY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub fn identify() -> Result<(), AppError> {
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(APP_ID)?;
    }
    Ok(())
}
pub fn installed_path() -> Result<PathBuf, AppError> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable.")?)
            .join("Programs")
            .join("WiniOS")
            .join("winios.exe"),
    )
}
fn installed_image() -> Result<PathBuf, AppError> {
    let path = installed_path()?;
    settings::validate_local_absolute(&path)?;
    if std::env::current_exe()?.canonicalize()? != path.canonicalize()? {
        return Err("Use the installed %LOCALAPPDATA%\\Programs\\WiniOS\\winios.exe for Windows integration.".into());
    }
    Ok(path)
}
fn startup_command(exe: &Path) -> Result<String, AppError> {
    settings::validate_local_absolute(exe)?;
    let text = exe.to_str().ok_or("Startup path must be valid Unicode.")?;
    if text.contains(['"', '\0', '\r', '\n']) {
        return Err("Invalid startup executable path.".into());
    }
    let command = format!("\"{text}\" --startup");
    if command.encode_utf16().count() >= 260 {
        return Err("Startup command exceeds the Windows Run limit.".into());
    }
    Ok(command)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Registration {
    Off,
    On,
    Conflict,
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}
fn registration_at(subkey: PCWSTR, expected: &str) -> Result<Registration, AppError> {
    let mut units = vec![0u16; 8192];
    let mut bytes = (units.len() * 2) as u32;
    let error = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(units.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if error == ERROR_FILE_NOT_FOUND {
        return Ok(Registration::Off);
    }
    if error == ERROR_MORE_DATA {
        return Ok(Registration::Conflict);
    }
    error.ok()?;
    if bytes < 2 || !bytes.is_multiple_of(2) {
        return Ok(Registration::Conflict);
    }
    units.truncate(bytes as usize / 2);
    if units.pop() != Some(0) {
        return Ok(Registration::Conflict);
    }
    Ok(if units == expected.encode_utf16().collect::<Vec<_>>() {
        Registration::On
    } else {
        Registration::Conflict
    })
}
fn set_at(subkey: PCWSTR, expected: &str, enabled: bool) -> Result<(), AppError> {
    let state = registration_at(subkey, expected)?;
    if state == Registration::Conflict {
        return Err("Another WiniOS startup command exists. Review it in Windows Startup Apps; it was not overwritten.".into());
    }
    if (enabled && state == Registration::On) || (!enabled && state == Registration::Off) {
        return Ok(());
    }
    let mut key = HKEY::default();
    if enabled {
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
            .ok()?;
        }
    } else {
        unsafe {
            RegOpenKeyExW(HKEY_CURRENT_USER, subkey, None, KEY_SET_VALUE, &mut key).ok()?;
        }
    }
    let key = Key(key);
    if enabled {
        let bytes: Vec<u8> = wide(expected)
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        unsafe {
            RegSetValueExW(key.0, VALUE, None, REG_SZ, Some(&bytes)).ok()?;
        }
    } else {
        unsafe {
            RegDeleteValueW(key.0, VALUE).ok()?;
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct StartupInfo {
    pub registered: bool,
    pub can_toggle: bool,
    pub note: &'static str,
}
pub fn startup_info(config: &Path) -> StartupInfo {
    let result = (|| -> Result<StartupInfo, AppError> {
        let exe = installed_path()?;
        let state = registration_at(RUN, &startup_command(&exe)?)?;
        let installed = installed_image().is_ok();
        let default_config = config.canonicalize()? == settings::default_path()?.canonicalize()?;
        Ok(StartupInfo {
            registered: state == Registration::On,
            can_toggle: installed && default_config && state != Registration::Conflict,
            note: if state == Registration::Conflict {
                "Existing startup entry needs review"
            } else if !installed {
                "Startup is managed by the installed WiniOS app"
            } else if !default_config {
                "Startup uses saved default settings"
            } else {
                "Registration respects Windows Startup Apps"
            },
        })
    })();
    result.unwrap_or(StartupInfo {
        registered: false,
        can_toggle: false,
        note: "Startup registration could not be read",
    })
}
/// Called only for an explicit checkbox selection. Never changes StartupApproved
/// or policy: a Windows Startup Apps disable/administrator policy stays in force.
pub fn toggle_startup(config: &Path) -> Result<StartupInfo, AppError> {
    let info = startup_info(config);
    if !info.can_toggle {
        return Err(info.note.into());
    }
    let command = startup_command(&installed_image()?)?;
    set_at(RUN, &command, !info.registered)?;
    let result = startup_info(config);
    if !result.can_toggle || result.registered == info.registered {
        return Err("Startup registration could not be verified.".into());
    }
    Ok(result)
}

struct Com;
impl Com {
    fn new() -> Result<Self, AppError> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
fn create_shortcut(path: &Path, exe: &Path) -> Result<(), AppError> {
    let _com = Com::new()?;
    let target = wide(exe.to_str().ok_or("Invalid executable path.")?);
    let directory = wide(
        exe.parent()
            .ok_or("Missing install directory.")?
            .to_str()
            .ok_or("Invalid directory.")?,
    );
    let destination = wide(path.to_str().ok_or("Invalid shortcut path.")?);
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target.as_ptr()))?;
        link.SetArguments(w!("--tray"))?;
        link.SetWorkingDirectory(PCWSTR(directory.as_ptr()))?;
        link.SetDescription(w!("WiniOS — secure clipboard continuity"))?;
        link.SetIconLocation(PCWSTR(target.as_ptr()), 0)?;
        link.SetShowCmd(SW_SHOWNORMAL)?;
        let properties: IPropertyStore = link.cast()?;
        // windows-rs owns/frees this COM string through PROPVARIANT's Drop.
        let value = PROPVARIANT::from("WiniOS.Desktop");
        properties.SetValue(&APP_ID_KEY, &value)?;
        properties.Commit()?;
        let persist: IPersistFile = link.cast()?;
        persist.Save(PCWSTR(destination.as_ptr()), true)?;
    }
    Ok(())
}
/// Explicit installation action, independent of the running clipboard agent.
/// Creates only a Programs shortcut, never a startup registration or pin.
pub fn install_start_menu_shortcut() -> Result<(), AppError> {
    let exe = installed_image()?;
    let folder = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None)? };
    let path = unsafe { folder.to_string() };
    unsafe {
        CoTaskMemFree(Some(folder.0.cast()));
    }
    let path = PathBuf::from(path?).join("WiniOS.lnk");
    settings::validate_local_absolute(&path)?;
    if path.exists() {
        // Preserve a previous shortcut before replacement; no unrelated link deleted.
        let backup = path.with_extension("lnk.winios-backup");
        if !backup.exists() {
            std::fs::copy(&path, backup)?;
        }
    }
    create_shortcut(&path, &exe)?;
    let notification = wide(path.to_str().ok_or("Invalid shortcut path.")?);
    unsafe {
        SHChangeNotify(
            SHCNE_UPDATEITEM,
            SHCNF_PATHW,
            Some(notification.as_ptr().cast()),
            None,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_path_uses_native_separators_for_shell_and_run_consistency() {
        let path = installed_path().unwrap();
        assert!(!path.to_string_lossy().contains('/'));
        assert!(path.ends_with("Programs\\WiniOS\\winios.exe"));
    }
    #[test]
    fn startup_quotes_spaces_and_rejects_invalid_or_excessive_paths() {
        assert_eq!(
            startup_command(Path::new("C:\\Users\\Dev Name\\WiniOS\\winios.exe")).unwrap(),
            "\"C:\\Users\\Dev Name\\WiniOS\\winios.exe\" --startup"
        );
        for path in ["relative.exe", "C:\\bad\"name.exe", "C:\\bad\nname.exe"] {
            assert!(startup_command(Path::new(path)).is_err());
        }
        assert!(
            startup_command(Path::new(&format!("C:\\{}\\winios.exe", "x".repeat(260)))).is_err()
        );
    }
    #[test]
    fn isolated_registry_registration_round_trip_never_touches_run() {
        let unique = format!(
            "Software\\WiniOS\\Tests\\Startup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let subkey_units = wide(&unique);
        let subkey = PCWSTR(subkey_units.as_ptr());
        let command = "\"C:\\Dev Name\\winios.exe\" --tray";
        assert_eq!(registration_at(subkey, command).unwrap(), Registration::Off);
        set_at(subkey, command, true).unwrap();
        assert_eq!(registration_at(subkey, command).unwrap(), Registration::On);
        assert_eq!(
            registration_at(subkey, "another command").unwrap(),
            Registration::Conflict
        );
        assert!(set_at(subkey, "another command", false).is_err());
        set_at(subkey, command, false).unwrap();
        assert_eq!(registration_at(subkey, command).unwrap(), Registration::Off);
        unsafe {
            RegDeleteKeyW(HKEY_CURRENT_USER, subkey).ok().unwrap();
        }
    }
    #[test]
    fn shell_shortcut_round_trip_in_temporary_directory() {
        let temp =
            std::env::temp_dir().join(format!("winios-test-shortcut-{}.lnk", std::process::id()));
        let exe = std::env::current_exe().unwrap();
        create_shortcut(&temp, &exe).unwrap();
        let _com = Com::new().unwrap();
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            let persist: IPersistFile = link.cast().unwrap();
            let path = wide(temp.to_str().unwrap());
            persist.Load(PCWSTR(path.as_ptr()), STGM_READ).unwrap();
            let mut args = [0u16; 64];
            link.GetArguments(&mut args).unwrap();
            assert_eq!(String::from_utf16_lossy(&args[..6]), "--tray");
            let mut directory = [0u16; 32768];
            link.GetWorkingDirectory(&mut directory).unwrap();
            let directory = String::from_utf16_lossy(
                &directory[..directory.iter().position(|v| *v == 0).unwrap()],
            );
            assert_eq!(Path::new(&directory), exe.parent().unwrap());
            let properties: IPropertyStore = link.cast().unwrap();
            let id = properties.GetValue(&APP_ID_KEY).unwrap();
            let actual = windows::core::BSTR::try_from(&id).unwrap().to_string();
            assert_eq!(actual, "WiniOS.Desktop");
        }
        std::fs::remove_file(temp).unwrap();
    }
}
