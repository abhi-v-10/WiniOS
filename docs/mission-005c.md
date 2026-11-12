# Mission 005C — native popup and Windows integration

## Architecture and appearance

WiniOS now uses an **owner-drawn native Win32 popup menu**, retaining Windows'
menu navigation, focus, dismissal, screen-reader infrastructure and placement.
It has a dark surface, original teal WiniOS icon, Segoe UI typography, compact
header/status and HTTPS endpoint, subtle rounded hover rows, separators, and a
checked Start with Windows item. No Electron, WebView, Tauri, browser, UI runtime
or network-based renderer is included. The example designs were not attached in
this conversation; their branding/assets were not used.

Windows owns the outer popup frame, shadow and corner treatment. Windows versions
that round native menus can round that frame; this app does not use undocumented
theme hooks or force DWM behavior. Hover highlights are explicitly rounded.
High Contrast automatically uses the standard system menu. Font/palette/menu
initialization failures also fall back to that reliable native menu. An explicit
fallback is available after Quit:

```powershell
& "$env:LOCALAPPDATA\Programs\WiniOS\winios.exe" --native-menu
```

The manifest enables Per-Monitor-V2 DPI awareness. The hidden owner is moved to
the notification anchor's monitor before font/row sizing; width is constrained
to its working area. Signed coordinates and native TrackPopupMenu positioning
support monitors to the left/above the primary display. Small displays retain
Windows' menu scrolling. State is a snapshot when the popup opens; reopen after
connection/configuration changes. Keyboard arrows, Enter/Space, Escape, and the
context-menu key/Shift+F10 remain native. Mnemonics are P (Pause) or R (Resume),
T (Retry), W (Start with Windows) and Q (Quit). Owner-drawn rows provide stable
MSAAMENUINFO names and preserve native checked/disabled state.

## Commands and preserved security

- Click the WiniOS header for the metadata-only Status dialog.
- HTTPS endpoint contains no credentials. The tailnet hostname is still discovered,
  not hardcoded, and no token or clipboard content is exposed by the UI.
- Pause/Resume retains the existing semantics: authorized clipboard GET/POST=503
  while paused, missing/invalid bearer=401, health remains available when ready.
  Observation continues. Retry preserves Pause; full Quit/relaunch resets it.
- Retry gracefully stops/joins the old worker before starting one replacement.
- Start with Windows reflects the current user's actual WiniOS Run value each
  time the menu opens. Nothing is registered merely by installing or launching.
- Open Settings is omitted: there is no settings editor implemented in this mission.
- Quit stops the server, cancels/completes writes, joins the Win32 worker, removes
  the icon and releases the singleton. No forced process exit or thread termination.

Clipboard code, ownership, Unicode handling, retries, deduplication, origin tracking,
HTTPS, bearer authentication, request limits, no-store and private peer checks are
unchanged. Tailscale mode remains HTTP **only on 127.0.0.1 behind HTTPS Serve**.
No certificate/CA, Serve/Funnel, firewall, network settings or credential is changed.
The previously approved persistent Serve route still outlives WiniOS Quit; see
[portable connectivity rollback](portable-connectivity.md#serve-lifetime-removal-and-rollback).

## Windows installation and Start Menu

The stable path stays `%LOCALAPPDATA%\Programs\WiniOS\winios.exe`; it is not moved
or renamed. `winios-agent.exe` beside it remains the debug console binary. The GUI
executable contains an original multi-resolution icon, product metadata, an
asInvoker manifest (no elevation) and DPI declarations. The only new third-party
dependency is **build-only** `winresource`, which invokes the Windows resource
compiler. It adds no runtime framework. Existing Rust/MSVC Windows SDK build tools
are required for `cargo build --release`.

After backing up/replacing binaries following a graceful Quit, create the shortcut
with this explicit installation action (it starts no clipboard server/watcher):

```powershell
$exe = Join-Path $env:LOCALAPPDATA 'Programs\WiniOS\winios.exe'
Start-Process -FilePath $exe -ArgumentList '--install-shortcut' -Wait -WindowStyle Hidden
.\scripts\Test-WindowsIntegration.ps1 -ExpectedStartup Off
```

This creates the **current user's** Start Menu `Programs\WiniOS.lnk`, using the
installed executable as target and icon, `--tray` arguments, the install directory
as working directory, and the stable `WiniOS.Desktop` AppUserModelID shared with
the process. An existing WiniOS shortcut is backed up before replacement. Other
applications/shortcuts are untouched. Normal Windows file notifications are issued;
Explorer or the search service is not killed/restarted. Search indexing may lag.

Search Start for **WiniOS** and launch it. A subsequent search/pinned launch opens
the existing instance's popup rather than creating another agent. To pin, use
Windows' **Pin to Start** / **Pin to taskbar** from the installed shortcut's context
menu (on Windows 11, taskbar pin may be under More or Show more options). Pinning
is a user choice; the app does not edit pin databases, use unsupported Shell verbs,
or circumvent enterprise pin policies. Remove old pins targeting `C:\WiniOS\target`
and repin the new Start Menu shortcut if needed. A tray-only app intentionally has
no permanent running taskbar window; a pin is a launcher for the tray popup.

## Start with Windows

**Default OFF.** Only selecting the checkbox creates/removes WiniOS's own value:

```text
HKCU\Software\Microsoft\Windows\CurrentVersion\Run
  WiniOS = "<LOCALAPPDATA>\Programs\WiniOS\winios.exe" --startup
```

The actual value contains the fully expanded local path, correctly quoted, stored
as REG_SZ. It contains no credentials or network flags. The command is bounded by
the documented Run-key length limit. The GUI-subsystem `--startup` launch reads
the existing default settings, opens no console, and silently declines duplicate
startup launches. All configuration/identity paths are absolute, so the login
working directory need not be the installation directory. No admin rights, service,
task scheduler or all-users entry is used. No entry is added to the Startup folder.

Development copies or a tray using an alternate `--config` cannot change startup
registration: the installed app and saved default settings must be used. The
checkbox still reports the real registered command where it can be read. A
conflicting WiniOS command or unreadable registration disables mutation and reports
the limitation. Unrelated Run values are never changed. No startup Boolean is
written into settings.json, so existing settings and TLS identity are preserved.

Checked means the exact WiniOS command is **registered**, not that Windows policy
has guaranteed execution. Windows may delay Run entries, and Startup Apps/Task
Manager or an administrator can disable/block them. WiniOS does not alter the
undocumented StartupApproved values or override that separate Windows decision.
If login startup is blocked, review **Settings → Apps → Startup** manually. Disabling
the WiniOS checkbox removes its Run entry; there is no hidden secondary mechanism.

Tailscale may start later. The tray/loopback backend waits safely with the existing
Not configured / Tailscale disconnected / Waiting for network states, with bounded
background rechecks. Clipboard access stays unavailable until the exact approved
HTTPS Serve profile, current node identity, connection and DNS checks pass. It
does not configure Tailscale, regenerate certificates, expose LAN HTTP or downgrade.

## Tests and exact manual validation

```powershell
cd C:\WiniOS
cargo fmt --check
cargo check --offline
cargo test --offline
cargo build --release --offline
.\scripts\Test-DesktopBinary.ps1
.\scripts\Test-WindowsIntegration.ps1 -ExpectedStartup Off
```

The isolated registry test may require permission to run outside a restricted
sandbox; it writes only a unique `HKCU\Software\WiniOS\Tests` key and deletes that
test key. It never touches the production Run entry. Shortcut tests use a temporary
link, verify arguments/working directory/AppUserModelID, and remove it afterward.
Native GDI preview/owner-draw tests exercise measurement, accessibility names,
keyboard mnemonics and drawing without opening a real popup or starting an agent.
`target\popup-preview.bmp` is a generated preview, not a captured desktop.

1. Search Start for WiniOS; verify the correct app icon and installed path, launch
   it, and confirm one tray agent with no terminal. Alternatively validate/launch
   the actual shortcut via `Test-WindowsIntegration.ps1 -Launch`.
2. Right-click the tray icon. Inspect dark styling and clear endpoint/status. Open
   and close Status. Test keyboard arrows, Enter/Space, Escape, mnemonics, and
   Shift+F10. Test 100/125/150/200% DPI and secondary monitors/negative coordinates
   if available. In High Contrast confirm the native fallback remains usable.
3. Verify startup is initially unchecked. **Select it yourself** to enable; reopen
   and confirm checked. Run `Test-WindowsIntegration.ps1 -ExpectedStartup On`.
   Select it again to disable; verify unchecked and `-ExpectedStartup Off`.
   Automated tests/installers do not toggle the real registration for you.
4. If you choose to use startup, enable it again, save work, and at a convenient
   time sign out/in yourself. Do not force a sign-out/reboot merely for this test.
   Verify one tray, no console, saved settings, and safe recovery when Tailscale
   becomes ready. An explicit `--startup` launch with System32 as working directory
   can test the command/path behavior, but is not an actual login test.
5. Pause and run `Test-Tailnet.ps1 -ExpectPaused`; verify authenticated GET/POST=503,
   unauthenticated=401, health=200. Retry while paused, repeat, then Resume.
6. Test both physical iPhone Shortcuts over the existing trusted HTTPS URL. The
   optional `Test-Tailnet.ps1 -NativeClipboard` overwrites clipboard contents with
   disposable Unicode test text; avoid running it while copying your own text.
7. Quit from the popup; icon/process/listener disappear. Relaunch from Start. Click
   the pin/shortcut again and verify the existing popup opens without a duplicate.
   Test the explicit fallback after Quit with `winios.exe --native-menu`.

Record hardware/UI/login results separately from automated tests. Sign-in,
multi-monitor, Windows Search ranking and pin-policy behavior cannot be inferred
from a unit test. Installation/relaunch does not establish physical login success.

## Rollback

If enabled, uncheck Start with Windows before rolling back to a version without
the `--startup` option. Quit gracefully, restore backed-up binaries at the same
install path, then relaunch. Restore the backed-up Start Menu shortcut or remove
only the WiniOS shortcut if desired. Do not delete unrelated Run entries, pin data,
settings, certificates, Credential Manager entries or Tailscale configuration.
Saved configuration is not migrated or overwritten by this mission.

## Recorded implementation results

On 2026-11-06, `cargo fmt --check`, `cargo check --offline`,
`cargo test --offline` (**69 passed, 0 failed**) and `cargo build --release --offline`
passed. New coverage includes native GDI drawing/measurement, keyboard mnemonics,
MSAA naming, DPI metrics, startup command quoting/native separators, isolated
registry on/off/conflict handling, and COM shortcut round trips. All existing
clipboard, auth, TLS, queue, pause and shutdown tests remain passing.

The generated GDI preview was visually inspected. This is a native rendering check,
not a claim of a physical popup/keyboard/multi-monitor interaction.

The installed GUI and console subsystem checks passed. The real per-user Start Menu
shortcut was verified for target, `--tray` arguments, native working directory,
embedded-icon reference and AppUserModelID, then **launched successfully**, yielding
one agent from the stable installed path. Startup was verified **OFF** afterward.
Both installed binaries matched the release files. Settings and Serve snapshots
were unchanged. An initial shortcut validation caught mixed path separators; the
installer restored the previous app, the path builder was corrected with a
regression test, and the corrected installation passed all metadata checks.

The real `Test-Tailnet.ps1 -NativeClipboard` suite passed on the new installed app:
normal verified HTTPS, authorization, exact Unicode/LF/CRLF, acknowledgements,
contention recovery/exhaustion, and local-copy GET recovery. Disposable clipboard
test text was used; no token or content was printed or persisted.

Installed binary backups: `%LOCALAPPDATA%\Programs\WiniOS\backup-005c-20261106-121204`.
The prior build-directory executable was also retained under
`C:\WiniOS\target\pre-005c-20261106-115322`. The executable was not moved or renamed.

The desktop-automation native pipe was unavailable. Windows' Start app listing
reported `WiniOS / WiniOS.Desktop`, and the user confirmed its Start-bar discovery.
The user also verified the corrected popup and selected the real checkbox; its ON
and OFF registrations were read back and validated. Pin interactions, other DPI/
monitor configurations, High Contrast and actual login remain manual checks.
The installer/test scripts never enable real startup behind the checkbox.

### Live-popup crash correction

The first custom popup release crashed on the initially unchecked startup row.
Windows recorded a USER32 access violation. The unchecked checkbox passed an empty
UTF-16 Vec to `DrawTextW`; its dangling empty-buffer pointer was not safe for that
native path. The renderer now skips empty text before calling Win32. A regression
that runs the **actual native popup message loop** reproduced the crash before the
fix and passed afterward. It now covers unchecked, checked, repeated openings and
the native fallback. The offscreen preview also uses an unchecked box now.

The fixed release was installed at the same path. Start Menu activation survived,
and the user physically confirmed that the premium popup is smooth/responsive and
that Start with Windows was unchecked after testing. Production registration had
also been observed ON during testing, with its exact quoted command validated;
no installer/script enabled it. User selection controls that value.

An actual sign-out/sign-in, pin behavior and multi-monitor/High Contrast interactions
remain separate manual tests. Do not confuse a simulated `--startup` launch with
a Windows login. The user was never forcibly signed out or rebooted for testing.

### Final installed checks

After the user switched startup OFF, `Test-WindowsIntegration.ps1 -ExpectedStartup
Off` verified that the production Run entry was absent. The fixed GUI gracefully
quit and port 8766 closed; a cold `--startup` launch from `C:\Windows\System32`
reacquired the singleton, loaded the saved settings and returned ready health.
A second `--startup` launch exited quietly without replacing/duplicating that
process. Registration stayed OFF throughout this simulated-command test.

The final fixed release passed the real HTTPS/native Unicode, authentication,
acknowledgement, contention and GET-recovery integration suite again. One installed
agent was left running with sharing enabled, the expected 127.0.0.1:8766 listener,
and a binary hash matching the release artifact. A five-second idle sample measured
0 ms agent CPU and 14.9 MiB working set; the GUI executable was 4,562,944 bytes.
These are observations, not a general benchmark or aggregate Tailscale resource cost.

The known stable pre-005C binaries remain in
`%LOCALAPPDATA%\Programs\WiniOS\backup-005c-20261106-121204`. The intermediate
renderer build was also backed up before the fix; use the stable backup above for
rollback, not the unfixed custom renderer. Disable startup before restoring a
version that does not support `--startup`.

References: [Win32 Run-key behavior](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys),
[AppUserModelIDs and shortcuts](https://learn.microsoft.com/en-us/windows/win32/shell/appids),
[Win32 native menus](https://learn.microsoft.com/en-us/windows/win32/menurc/using-menus),
[build-only Windows resources](https://docs.rs/winresource/latest/winresource/).

Changed files: `Cargo.toml`, `Cargo.lock`, `build.rs`, `resources/winios.manifest`,
`src/brand.rs`, `src/popup.rs`, `src/integration.rs`, `src/tray.rs`, `src/lib.rs`,
`src/bin/winios.rs`, `scripts/Test-WindowsIntegration.ps1`, `README.md`, and this guide.
The clipboard core, HTTP/auth policy, TLS, tailnet adapter and saved settings schema
were not changed for this mission.
