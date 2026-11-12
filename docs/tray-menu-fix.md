# Native tray menu fix

## Root cause and fix

The application selected `NOTIFYICON_VERSION_4`, but interpreted icon callbacks
only in the outer `GetMessageW` loop. Sent callbacks are dispatched directly to
the window procedure, where they were passed to `DefWindowProcW`. The outer loop
therefore never saw those context-menu events. A native `SendMessageW` regression
test reproduced that failure before the fix and passes afterward.

The window procedure now handles the Shell callback and native `WM_CONTEXTMENU`,
validates the version-4 icon ID in `HIWORD(lParam)`, decodes the event in
`LOWORD(lParam)`, and posts a menu request. Keyboard/select anchors use signed
coordinates, supporting monitors left of/above the primary display. For version-4
`WM_CONTEXTMENU`, whose `wParam` coordinates are not guaranteed, the menu uses
`Shell_NotifyIconGetRect`, falling back to the cursor. `NIN_KEYSELECT` is handled
as the Windows header expression `NIN_SELECT | NINF_KEY`.

The popup stays on the UI thread, calls `SetForegroundWindow`, uses
`TPM_RETURNCMD`, and restores notification-area focus after dismissal. Menu/window
resources still use RAII. Reentrant icon callbacks during a popup or status dialog
are suppressed. No networking, DNS lookup or clipboard contention wait runs on the
UI thread. Native graceful Close dismisses the popup/owned status dialog too.

References: [notification callback packing](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw),
[GetMessage sent-message dispatch](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmessagew),
[Shell notification versions and focus](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw).

## Menu behavior

- **Status: ...** opens a metadata-only dialog with the connection/sharing state
  and HTTPS endpoint. No token or clipboard contents are displayed.
- **Pause Clipboard Sharing / Resume Clipboard Sharing** controls only WiniOS
  clipboard HTTP requests. Authorized GET and POST return empty 503 responses while
  paused; missing/invalid bearer credentials remain 401. Health remains available
  when the underlying connection is ready. Peer restrictions and no-store remain.
- Pause drops handlers waiting for bodies or acknowledgements, using existing write
  cancellation. An operation already committed by Windows cannot be undone, and
  bytes already delivered to a client cannot be recalled. Observation continues,
  so Resume uses current sequence-checked clipboard state, never a stale pause cache.
- Pause is session-only, survives **Retry Connection**, and resets to sharing
  enabled after a full application Quit/relaunch. POST still requires the existing
  explicit `allow_clipboard_write` permission; Resume does not enable that permission.
- **Retry Connection** gracefully stops the current server/clipboard worker, waits
  for completion, rereads settings, and starts one replacement worker. It works while
  connected or after a failed start. The singleton/tray remain owned throughout;
  overlapping servers/watchers are not started. Quit cancels a pending Retry.
- **Quit WiniOS** uses the existing graceful shutdown, cancels/completes queued
  writes safely, joins the worker, removes the icon and releases the singleton.
  Tailscale/Serve remain user-managed and unchanged, including the previously
  approved persistent Serve route. No autostart is registered.

## Verification and exact manual checks

On 2026-11-06: formatting/check/tests/release build passed, with **62 passed,
0 failed**. The release PE check confirmed the GUI and console subsystems.
The new native sent-callback regression failed before the fix and passed after it.
The installed release also passed `Test-Tailnet.ps1 -NativeClipboard` through
normally verified HTTPS, including authentication, exact Unicode, contention,
acknowledgements and GET recovery.

The updated binary's graceful `--quit` path was tested after installation: the
process exited, port 8766 closed, Serve remained unchanged, and one relaunched
instance reacquired the singleton and returned ready health. It was left running
with session-default sharing enabled. This is not a claim of clicking the Quit
menu with a physical mouse.

Installed binaries match the release files. Settings and Serve configuration were
checked unchanged around installation. The previous binaries were backed up under
`%LOCALAPPDATA%\Programs\WiniOS\backup-tray-menu-20261106-105011`.

The computer-use native pipe was unavailable in this session, so no agent-observed
physical right-click, popup selection or menu Quit is claimed. Those checks are
listed below; native callback/API tests do not replace them.

The user subsequently **physically confirmed the updated menu appears with its
options and selected Pause Clipboard Sharing**. With that real tray pause active,
`Test-Tailnet.ps1 -ExpectPaused` passed through the existing HTTPS hostname:
health=200, unauthenticated GET/POST=401, authenticated GET/POST=503 with empty
no-store responses. No clipboard write was accepted. This confirms the installed
UI action controls the real server, without disconnecting Tailscale.

The user then selected **Retry Connection** while paused and confirmed that Paused
and the Resume option remained visible after reconnection. The live HTTPS paused
suite passed again after that Retry, including health=200, unauthorized GET/POST=401
and authenticated GET/POST=503. This validates Pause preservation through the actual
tray-driven worker restart, not only the lifecycle unit model.

After Resume, the user initially reported an empty laptop-to-iPhone result while
the opposite direction worked. The controlled native HTTPS suite passed after
Resume/Retry. A separate **read-only** `Test-Tailnet.ps1 -InspectCurrent` check then
observed GET=204, zero response bytes, no native Unicode text format, and a stable
Windows sequence: the Windows clipboard was genuinely empty at that observation.
This does not establish what cleared it. No stale data was returned, and no
speculative clipboard-core change was made.

The user subsequently **confirmed both copy/paste directions work correctly after
Resume**. Clipboard-modifying test runs were then stopped. The read-only inspection
mode prints only HTTP status, byte count, native format availability and sequence;
it neither replaces nor prints the text. It can be used to distinguish an actual
empty clipboard from a 401/409/503 response without leaking content.

Changed files for this fix: `src/tray.rs`, `src/lifecycle.rs`, `src/sharing.rs`,
`src/http.rs`, `src/app.rs`, `src/lib.rs`, the two tailnet/native integration
scripts, `README.md`, and this guide. The clipboard watcher/writer core, TLS and
Tailscale configuration were not changed. No new dependency was added.

Required build checks:

```powershell
cd C:\WiniOS
cargo fmt --check
cargo check --offline
cargo test --offline
cargo build --release --offline
.\scripts\Test-DesktopBinary.ps1
```

Tests cover sent Shell callbacks, icon-ID/signed-coordinate decoding, pause/resume
authentication and readiness ordering, no queued writes while paused, cancellation
of a waiting write, opt-in preservation, rapid pause/resume, and Retry/Quit ordering,
along with all existing clipboard and HTTPS/Tailscale tests.

After launching the updated standalone tray app:

1. Right-click the WiniOS icon (check the notification-area overflow arrow). The
   menu must appear near the icon. Choose **Status**, inspect the endpoint/state,
   close the dialog, and confirm a second right-click still works.
2. Press Escape to dismiss a menu; confirm it can reopen. With the icon keyboard
   focused, test Shift+F10 / the context-menu key, then Enter/Space. If available,
   test a secondary monitor with negative desktop coordinates.
3. Choose **Pause Clipboard Sharing**. The menu/tooltip must show Paused, and the
   action must change to **Resume Clipboard Sharing**. Do not disconnect Tailscale.
   Both iPhone Shortcuts must fail with 503, without accepting a write or returning
   clipboard text. Safari health should still work. On the laptop, run:

   ```powershell
   .\scripts\Test-Tailnet.ps1 -ExpectPaused
   ```

   This checks normally verified HTTPS health, unauthenticated GET/POST=401, and
   authenticated GET/POST=503. It never prints the stored bearer token or body.
4. While paused choose **Retry Connection**. It should return to the normal connection
   state with Paused still visible. Repeat the paused test. It must not change Serve
   configuration, certificate identity, write permission, or start a second agent.
5. Choose **Resume Clipboard Sharing**. Copy new disposable text in both directions
   with the existing HTTPS Shortcuts. On the laptop, optionally run the destructive
   clipboard test (it replaces clipboard contents with disposable Unicode text):

   ```powershell
   .\scripts\Test-Tailnet.ps1 -NativeClipboard
   ```

6. Choose **Retry Connection** while sharing is enabled. Wait for Connected, then
   repeat both Shortcuts. Retry does not change VPN/network/firewall configuration.
7. Choose **Quit WiniOS**. Verify the icon and process disappear, the loopback
   listener closes, and the HTTPS proxy can no longer reach WiniOS (usually 502).
   Relaunch once, verify the icon/API recover, and verify a second launch is refused.
   Leave only one instance running. The command-line `winios.exe --quit` exercises
   the same graceful close path when the icon is hidden.

Interactive Shell/menu tests require the actual desktop. Native message tests and
API tests alone do not establish that a physical mouse click was observed.

## Installation and rollback

The existing `%LOCALAPPDATA%\Programs\WiniOS` standalone installation is updated
only after graceful Quit and process exit, with the previous binaries backed up.
Settings, Credential Manager, certificates/CA, Tailscale Serve and firewall are not
changed by this update. Restore the backed-up binaries only after another graceful
Quit, then relaunch. No private key, token or clipboard payload belongs in a backup
inside the repository. The existing direct-LAN HTTPS configuration remains separate.
