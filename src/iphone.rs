//! Fixed public template links and explicitly requested local setup exports.
use crate::{native, setup, writes::WriteClient};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::{
    Win32::{
        System::Com::*,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{PCWSTR, w},
};

pub const SEND_URL: &str = "https://www.icloud.com/shortcuts/dd53de7c69464ee2bc45e90401036391";
pub const RECEIVE_URL: &str = "https://www.icloud.com/shortcuts/039fa0677d764e88a399e16c3e614f30";
pub fn pairing_instructions(endpoint: Option<&str>, tailnet: bool, writes: bool) -> String {
    let url = endpoint.filter(|base| clipboard_url(base).is_ok());
    format!(
        "1. CONNECT BOTH DEVICES\r\n{}\r\n\r\n2. GET YOUR VALUES\r\nWiniOS HTTPS URL: {}\r\nIn WiniOS Setup, use Copy HTTPS URL for this private address, with no trailing slash. Use Copy WiniOS Token and confirm the warning to copy your private pairing token. Keep it private; never put it in a link or share a configured Shortcut. Test connection below checks Windows readiness.\r\n\r\n3. INSTALL TWO SHORTCUTS\r\nOpen these links on your iPhone and add WiniOS Send and WiniOS Receive:\r\nSend: {SEND_URL}\r\nReceive: {RECEIVE_URL}\r\nThe two buttons below open the links on Windows; open the same links on iPhone. The Shortcut action graphs and Import Questions have not been verified here; review each before entering your token.\r\n\r\n4. ANSWER EACH IMPORT QUESTION\r\nFor WiniOS HTTPS URL, enter the address copied above, without a trailing slash. For WiniOS Token, privately enter your copied token. Repeat the questions for the other Shortcut. If either question is missing, stop and review that Shortcut before entering a token.\r\n\r\n5. TEST BOTH DIRECTIONS\r\n{} Copy disposable text on Windows and run Receive on iPhone; paste in Notes. Copy disposable text on iPhone and run Send; paste on Windows. Repeat with a disposable screenshot/photo: enable Images in WiniOS Settings and apply the change first. Keep the laptop awake, WiniOS running and sharing resumed. A Windows connection test does not prove iPhone transfer succeeded.\r\n\r\nIf it does not work, reconnect Tailscale on both devices, check WiniOS is Connected and sharing is resumed, and verify the two answers privately. For a certificate warning, stop and check your HTTPS identity; never bypass verification.",
        if tailnet {
            "Keep Tailscale connected on Windows and iPhone in the same approved tailnet, including on cellular. Use private HTTPS Serve, never Funnel."
        } else {
            "Existing direct-LAN HTTPS is preserved. Both devices need access and existing certificate trust; no trust is installed automatically."
        },
        url.unwrap_or(
            "Not currently verified. Go Back and Check again; no old endpoint is offered."
        ),
        if writes {
            "Clipboard writes are enabled."
        } else {
            "Clipboard writes are OFF: Receive can work, but Send requires explicit permission in Settings → Clipboard and Apply & restart."
        }
    )
}
pub const TOKEN_WARNING: &str = "Copy the WiniOS bearer token to the Windows clipboard?\n\nAnyone with this token and network access can read shared clipboard text and enabled images and, when writes are enabled, replace your clipboard. Other local applications can read the clipboard.\n\nWiniOS marks this copy private and excludes it from its API and Windows clipboard history/sync. This is not process-level protection. Paste only into your reviewed Shortcut's token Import Question; never into a URL, chat or public shortcut. Inspect BOTH templates for embedded credentials and unexpected destinations first.\n\nThe existing credential will not be changed. After entering it securely on iPhone, copy disposable text to replace the token on Windows. No automatic clearing timer runs.\n\nCopy the token now?";

#[derive(Clone, Copy)]
pub enum Export {
    Endpoint,
    SetupBase,
    Token,
}

/// Only these two constants can be launched; endpoint/token never enter a URL.
pub fn open_template(send: bool) -> Result<(), &'static str> {
    let url = if send { SEND_URL } else { RECEIVE_URL };
    let wide: Vec<u16> = url.encode_utf16().chain([0]).collect();
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            .ok()
            .map_err(|_| "Could not initialize the system browser launcher.")?;
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
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if result.0 as isize <= 32 {
            Err(
                "Could not open the default browser. Open the exact public template links from docs/mission-006b.md on your iPhone.",
            )
        } else {
            Ok(())
        }
    }
}
pub fn clipboard_url(base: &str) -> Result<String, &'static str> {
    let authority = base
        .strip_prefix("https://")
        .ok_or("A verified HTTPS endpoint is required.")?;
    if authority.is_empty()
        || authority.chars().any(|ch| {
            ch.is_whitespace() || ch.is_control() || matches!(ch, '/' | '?' | '#' | '@' | '\\')
        })
    {
        return Err("Invalid HTTPS endpoint. Check connection again.");
    }
    Ok(format!("{base}/v1/clipboard"))
}
fn setup_base_url(base: &str) -> Result<String, &'static str> {
    clipboard_url(base)?;
    Ok(base.to_owned())
}
pub async fn copy_export(
    profile: setup::Profile,
    writer: WriteClient,
    export: Export,
    cancel: &AtomicBool,
) -> Result<&'static str, &'static str> {
    // Revalidate Serve/DNS immediately before export instead of trusting a stale
    // wizard label. Reading an existing WCM credential never creates/rotates it.
    let check = setup::check_serve(&profile).await;
    if !check.passed {
        return Err(
            "Private HTTPS is no longer verified. Reconnect Tailscale/check Serve, then Check again. Nothing was copied.",
        );
    }
    let endpoint = clipboard_url(
        check
            .endpoint
            .as_deref()
            .ok_or("No verified HTTPS endpoint.")?,
    )?;
    if cancel.load(Ordering::Acquire) {
        return Err("Copy cancelled.");
    }
    let text=match export {
        Export::Endpoint=>endpoint,
        Export::SetupBase=>setup_base_url(endpoint.strip_suffix("/v1/clipboard").ok_or("Invalid verified endpoint.")?)?,
        Export::Token=>native::read_token().map_err(|_|"Could not read the existing bearer credential. Nothing was changed.")?.ok_or("No bearer credential exists. Start the normal agent first; Setup will not create one.")?.secret().to_owned(),
    };
    let copy = writer.copy_private(text);
    tokio::pin!(copy);
    // Close/Quit cancellation drops the pending command future and cancels it.
    loop {
        tokio::select! {
            result=&mut copy=>return result.map(|_|match export {Export::Endpoint=>"HTTPS clipboard URL copied.",Export::SetupBase=>"WiniOS HTTPS URL copied. Enter it in each Shortcut's WiniOS HTTPS URL question.",Export::Token=>"WiniOS Token copied privately. Enter it in each reviewed Shortcut's WiniOS Token question. Copy disposable text on Windows when finished."}).map_err(|_|"Clipboard copy failed, was cancelled, or timed out. The API never reports setup-copy text. Retry when clipboard contention clears."),
            _=tokio::time::sleep(std::time::Duration::from_millis(25))=>if cancel.load(Ordering::Acquire){return Err("Copy cancelled. If already committed, the clipboard still holds the private copy; replace it when finished.");},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_steps_keep_public_links_separate_from_private_endpoint() {
        let text = pairing_instructions(Some("https://fixture.example.ts.net"), true, false);
        for phrase in [
            "WiniOS HTTPS URL",
            "Import Questions",
            "Test connection",
            "writes are OFF",
        ] {
            assert!(text.contains(phrase));
        }
        assert!(text.contains("https://fixture.example.ts.net"));
        assert!(!text.contains("https://fixture.example.ts.net/v1/clipboard"));
        assert_eq!(
            setup_base_url("https://fixture.example.ts.net").unwrap(),
            "https://fixture.example.ts.net"
        );
        assert!(setup_base_url("https://fixture.example.ts.net/").is_err());
        assert!(!SEND_URL.contains("fixture") && !RECEIVE_URL.contains("fixture"));
        let missing = pairing_instructions(None, true, true);
        assert!(missing.contains("Not currently verified"));
        assert!(!missing.contains("fixture.example"));
    }
    #[test]
    fn fixed_templates_and_endpoint_export_never_embed_credentials() {
        assert_eq!(
            SEND_URL,
            "https://www.icloud.com/shortcuts/dd53de7c69464ee2bc45e90401036391"
        );
        assert_eq!(
            RECEIVE_URL,
            "https://www.icloud.com/shortcuts/039fa0677d764e88a399e16c3e614f30"
        );
        assert_eq!(
            clipboard_url("https://pc.tail123.ts.net").unwrap(),
            "https://pc.tail123.ts.net/v1/clipboard"
        );
        assert_eq!(
            clipboard_url("https://192.168.1.7:8765").unwrap(),
            "https://192.168.1.7:8765/v1/clipboard"
        );
        for bad in [
            "http://127.0.0.1:8766",
            "https://",
            "https://pc/?token=secret",
            "https://secret@pc",
            "https://pc/#fragment",
            "https://pc\r\nx",
            "https://pc\\path",
        ] {
            assert!(clipboard_url(bad).is_err());
        }
        assert!(TOKEN_WARNING.contains("Other local applications"));
        assert!(TOKEN_WARNING.contains("not be changed"));
    }
}
