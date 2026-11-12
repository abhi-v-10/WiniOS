# Mission 006A — native first-run setup

## Behavior and boundaries

`winios.exe` opens a modeless native Win32 setup window only when its selected
settings file is absent. Existing files, including invalid/inaccessible files,
are protected from automatic onboarding. Normal startup keeps its existing
validation and errors. A working installation starts exactly as before.

Choose **Settings / Setup…** in the tray to reopen the window. Reopening an
existing installation is a **read-only review**, including its saved clipboard
write permission. It never rewrites settings or restarts the service. You can
also launch `winios.exe --setup`; an existing tray process receives the request
without starting another server or watcher. `--setup --config <absolute-path>`
selects a path when starting a new process; an already running instance always
reviews its own active configuration, not another launch's supplied path.

The dark window uses standard Windows text, checkbox and button controls, Segoe
UI, DPI resizing, Tab/Shift+Tab, Enter and Escape. Windows-supported rounded
corners and dark title bars are best effort. It uses no WebView, Electron,
Tauri, browser, background renderer or new runtime dependency. Text instructions
can be selected/copied using the read-only text area. Network checks and bounded
Tailscale CLI queries run on a separate worker, not the tray/UI thread.

## First launch

1. **Welcome:** explains the checks and consent. Close before Apply leaves no
   settings file. Closing setup leaves the unconfigured tray running; reopen
   it from the menu or Quit normally.
2. **Tailscale:** install the official Windows/iPhone clients yourself, sign in
   to your approved tailnet, complete device approval if required, and connect.
   The wizard reads the protected Program Files CLI's status. It does not open
   a login session, collect account credentials or provision devices.
3. **Private HTTPS Serve:** follow
   [portable connectivity prerequisites and user-approved setup](portable-connectivity.md).
   Enable/review MagicDNS, HTTPS certificates and Tailscale's certificate
   transparency implications. Inspect existing Serve routes before manually
   configuring the dedicated HTTPS root proxy to `http://127.0.0.1:8766`.
   The wizard shows the command **as instructions only**. It never runs a
   configuration command. Serve's DNS name, current node IPs, HTTPS listener,
   exact loopback port/route and absence of Funnel/extra routes are checked.
   Failed checks disable Next; correct the issue yourself and **Check again**.
4. **Clipboard consent:** authenticated GET remains available. The checkbox
   allowing authenticated requests to replace the Windows clipboard starts
   **OFF**. Choose it only if you want iPhone → Windows transfer. **Apply & test**
   creates the selected local settings file with loopback mode and your explicit
   permission, then starts the normal agent. Atomic, non-replacing file commit
   protects against an installation appearing while the wizard is open.
5. **Connection:** wait for Connected in the tray and choose **Check again**.
   The wizard validates the actual HTTPS hostname/certificate using Windows
   trust, reads only public health, verifies an unauthenticated request gets 401,
   then reads the existing Credential Manager bearer and sends invalid JSON text
   (`{"text":null}`) to POST. Expected rejection proves authentication without
   reading or writing clipboard text. Writes disabled (403) and paused sharing
   (503) are reported explicitly. A 503 does not prove clipboard transfer works.
   Finish becomes available only after these checks pass.

The test uses TLS 1.2 or newer, no redirects, no system proxy, no automatic
Windows logon, bounded response size/time and no certificate bypass or HTTP
fallback. Bearer tokens, clipboard contents, keys and account credentials are
never displayed or logged. A fresh applied installation uses the agent's
existing Credential Manager token-creation mechanism; Setup itself never
creates/replaces/exports a credential. Follow the explicit token/Shortcut setup
in the portable-connectivity guide. Pairing/token enrollment is a later mission.

Applied choices remain saved if the window is closed afterward. Before commit,
Close cancels the work; if commit already succeeded the agent still starts.
Quit closes/cancels setup, waits for bounded in-flight checks, and gracefully
stops the existing API/watcher and releases the singleton. Closing Setup alone
does not quit, pause, resume, reconfigure Tailscale or disable sharing.

Existing direct-LAN HTTPS installations skip Tailscale prerequisites and retain
their original IP/certificate. The final Windows trust check may fail if only
the iPhone trusts the private CA; it never installs roots or changes that identity.
Existing write permissions can still be deliberately edited using the documented
settings-file workflow, followed by Retry Connection. This wizard is not a
general settings editor. Autostart remains user-controlled and unchanged.

## Validation commands and manual acceptance

```powershell
cd C:\WiniOS
cargo fmt --check
cargo check --offline
cargo test --offline
cargo build --release --offline
.\scripts\Test-DesktopBinary.ps1
# With the single installed agent already running:
cargo test --offline existing_setup_read_only_https_probe -- --ignored --nocapture
.\scripts\Test-Tailnet.ps1 -NativeClipboard
```

The final script deliberately replaces the clipboard with disposable test text;
it starts no agent and never changes Serve. The ignored probe reads no clipboard.
Unit tests use temporary settings, an isolated HTTPS listener with an untrusted
test certificate, and owned native test windows. No production credentials/trust
are modified by those tests.

1. Existing installation: launch from Start; no wizard should appear, no console,
   one tray/process/listener. Verify settings/certificates/credential/Serve and
   Start with Windows are unchanged. Open Settings / Setup repeatedly; review all
   five pages, run the test, Finish, and reopen/Close. Check Tab, Shift+Tab, Enter,
   Escape, resizing and dragging between monitors/scales. Tray Pause/Resume,
   Retry and Quit must still work while setup is open.
2. First run on a **separate Windows user or test VM**, not by deleting your
   working settings: launch with no settings; verify the welcome appears and no
   backend starts before Apply. Test missing Tailscale, signed out, disconnected,
   absent/wrong Serve port, DNS failure and Funnel/extra routes. Checks must fail
   closed with instructions. Restore manually, Check again and continue.
3. Close before Apply and relaunch: no settings or write consent should be saved.
   Complete with checkbox OFF: new preferences contain `allow_clipboard_write:false`;
   authenticated POST returns 403. On a separate fresh path/user, explicitly
   consent ON and verify both transfer directions with disposable text. Existing
   files must never be replaced, even if one appears while setup is open.
4. Pause from the tray, reopen/test setup: authentication remains required and
   paused state is reported. Resume, test again, and check both actual iPhone
   Shortcuts over the same HTTPS URL. Safari must have no certificate warning.
   Wrong/untrusted/expired certificates must block the connection; never bypass.
5. Close/Quit while a check is active; the UI must remain responsive and the
   process must exit after bounded work. Relaunch and verify the singleton was
   released. Tailscale/Serve must remain exactly as before.

Full interactive first-run, accessibility/DPI and physical iPhone checks must be
recorded separately from automated/native/API checks. No simulated test is a
claim of real iPhone or roaming validation.

## Install and rollback

Keep the stable `%LOCALAPPDATA%\Programs\WiniOS\winios.exe` path. Quit gracefully,
wait for the process/listener to stop, back up both installed binaries into a
timestamped folder under that directory, copy both release executables, verify
hashes/subsystems and launch the installed GUI. Do not replace settings, token,
certificates, shortcut, startup entry or Serve configuration. If replacement
fails, restore the matching backed-up binary pair before relaunching. To roll
back later, Quit and restore that pair; new local preferences are compatible with
the existing tailnet backend. Never reset Serve or delete working credentials
to troubleshoot this UI.

## Recorded validation — 2026-11-06

- `cargo fmt --check` and `cargo check --offline`: passed.
- `cargo test --offline`: **76 passed, 0 failed, 1 ignored**. The existing isolated
  HKCU test was initially denied inside the sandbox; the full suite passed when
  authorized outside it. This test never edits the production Run value.
- The ignored `existing_setup_read_only_https_probe` was run explicitly before
  and after installation: **passed**, with the current real Tailscale HTTPS
  identity and existing bearer; settings bytes unchanged, clipboard untouched.
- `cargo build --release --offline`: passed. Desktop/CLI PE subsystem check
  passed (GUI=2, CLI=3). Installed binary hashes matched the release pair.
- Installed at the existing `%LOCALAPPDATA%\Programs\WiniOS` path after graceful
  Quit. Backup: `backup-006a-20261106-132814` in that directory. Settings and all
  existing local TLS file hashes, actual startup registration and read-only Serve
  configuration were compared before/after and unchanged. Startup happened to be
  registered at inspection; the update did not enable or disable it.
- `Test-Tailnet.ps1 -NativeClipboard` against the installed app: **passed** HTTPS
  health, GET/POST authorization and no-store, accepted/repeated/retried writes,
  exhausted contention=503, exact native Unicode/LF/CRLF readback, GET and later
  local-copy recovery. Only disposable test clipboard data was used.
- One installed GUI process remained responsive, listening on **127.0.0.1:8766**
  only. `--setup` activation returned successfully without starting another agent.
- Owned native test windows covered rendering all pages with both permission
  states, protected/disabled existing consent, close dispatch, cancel before
  settings creation, commit/one-time start notification and preserving existing
  settings. The Windows HTTPS probe reached a real untrusted TLS listener and
  rejected it without a plaintext retry.

The Computer Use native pipe was unavailable. The user reviewed the installed
wizard and confirmed it works well without crashing. A subsequent installed-app
graceful Quit/relaunch test passed: old process exited, listener disappeared,
singleton was reacquired on relaunch, one loopback listener returned, and settings
remained unchanged. The app was left running normally, with setup closed.

Full fresh-user installation, detailed keyboard/accessibility/multi-monitor DPI
review and new physical iPhone acceptance remain manual. No Tailscale, certificate,
firewall, trust or autostart change was performed by this mission.

## Files changed for this mission

- `src/setup.rs`: policy/state, safe new-preferences commit, read-only checks,
  Windows-validated HTTPS probe and tests.
- `src/wizard.rs`: native dark window, controls, background work, cancellation,
  queued actions, DPI layout and native lifecycle tests.
- `src/tray.rs`: first-run gate, Settings / Setup menu, activation and shutdown
  integration; existing clipboard/service lifecycle retained.
- `src/bin/winios.rs`: explicit `--setup` activation option.
- `src/tailnet.rs`: read-only setup prerequisite/status adapters, using the
  existing protected CLI, strict identity and Serve validators.
- `src/lib.rs`: module registration. `Cargo.toml`: Windows WinHTTP/DWM/input API
  feature bindings; no new runtime crate.
- `README.md` and this document: setup, validation, security and rollback.

Pre-existing workspace changes from earlier missions were preserved. Clipboard,
writer, authentication, HTTP routes, TLS server and autostart implementation were
not rewritten.
