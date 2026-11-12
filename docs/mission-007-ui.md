# Mission 007 — Windows 11 native UI overhaul

This UI follow-up is separate from the earlier Mission 007 installer work.
The clipboard engine, HTTP routes, authentication, TLS/Serve identity and Win32
clipboard memory ownership remain unchanged. The API reports its active write
permission to the desktop UI; that metadata contains no credential or text.

## Interfaces and controls

The tray opens a compact native popup with the WiniOS icon/status, actual HTTPS
endpoint, Pause/Resume, Retry, Settings, startup registration and Quit. Standard
layout is 280 DIPs wide, with 12-DIP body text, a small 24-DIP logo and flat,
tightly spaced rows. The larger Settings typography/layout is unchanged. Endpoint
caption/address are native STATIC labels with no scrollbars or edit-control state.
An unusually long URL is visually ellipsized; native accessibility retains its
full value, and Settings → Connection → Copy HTTPS endpoint exports the full URL.
Standard
Windows controls supply accessibility and focus; arrows, Tab, Enter/Space and Esc
are supported. Losing activation dismisses the popup. High contrast, unavailable
UI resources and small work areas use the native system-menu fallback, including
Windows' scrolling behavior. `--native-menu` remains available.

**Settings** is a separate modeless native window. Closing it keeps WiniOS running.
Open it from the tray or launch `winios.exe --settings`. Existing `--setup` still
opens the preserved first-run/iPhone setup wizard.

- **General:** actual Start with Windows registration and a saved option to open
  Settings on a manual launch. Startup launches always remain quiet in the tray.
  Windows Startup Apps disable/policy state is respected, not overwritten.
- **Connection:** live service/Tailscale status and verified HTTPS endpoint; private
  endpoint copy, existing HTTPS/authentication probe, graceful Retry, iPhone Setup.
  No network editor, certificate replacement or automatic Serve action is added.
- **Clipboard:** live session sharing and confirmed saved write permission.
  **Apply & restart service** applies the saved permission through the existing
  graceful stop/join/restart path. Pending writes may cancel; Pause is preserved.
  Supported content remains Unicode text, with 1 MiB UTF-16 and UTF-8 limits.
- **Privacy:** the same live sharing state, security information and explicit
  token-copy warning. No token is displayed/exported on navigation or launch.
  Setup exports use the existing private-copy queue and cannot be served by GET.
- **About:** version/build architecture, troubleshooting/setup, connection
  diagnostics and private copying of metadata-only diagnostics. No clipboard text,
  request body, token, password or key is included.

Pause/Resume remains a **live session control**, preserving the previous behavior:
authorized clipboard GET/POST=503 while paused, missing bearer=401, Retry keeps
Pause, and Quit/relaunch starts a new session. It is deliberately distinguished
from saved preferences. Pausing does not change Tailscale or write permission.
The live sharing control stays usable during a background connection test.

## Native design and compatibility

`src/fluent.rs` shares midnight-blue/light/system palettes, Segoe UI Variable text,
Fluent glyphs, rounded cards and hover/focus treatment, control construction and window
lifetime handling between Settings and popup. Both use direct Win32 child controls
with native accessible names, checkbox state and keyboard behavior. The owner-draw
button layer keeps those underlying native controls; it is not a web renderer.

The supplied `src/WiniOS_AppIcon.png` is used for the executable, notification area,
Settings, Setup/window identity, Start Menu shortcut and installer. Midnight navy,
cyan, blue and violet accents derive from that artwork. Its original pixels are
resized/converted, never redrawn; `scripts/Update-BrandAssets.ps1` regenerates the
checked-in `resources/winios.ico` (16–256px) and tiny tray fallback after an artwork
change. This needs Windows PowerShell 7.4 and System.Drawing only at conversion
time, and adds no runtime dependency. The installer stages that exact icon.

**Visibility fix:** native GDI text/checkbox controls always use opaque client
surfaces. DWM backdrop is explicitly NONE and frame extension is zero on both
interfaces, including after theme/composition changes. Full-client Mica/Acrylic
previously caused real display text to disappear while screenshots looked correct:
ordinary GDI controls do not maintain alpha suitable for glass composition.
See [Microsoft's explanation](https://devblogs.microsoft.com/oldnewthing/20110113-00/?p=11763).
We retain native DWM rounded corners/borders and dark caption styling without
putting glass behind text. This compatibility choice applies regardless of the
Windows transparency preference; WiniOS never changes that preference.
High contrast uses system colors; light/dark follows AppsUseLightTheme.
Reduced motion disables DWM transitions. Theme notifications refresh appearance
without continuous polling or animation timers. OS settings are read, never changed.
Common Controls v6 is an OS manifest dependency, not a bundled UI framework.

Monitor work areas and signed coordinates constrain placement. Fonts/spacing
scale per window DPI and respond to monitor changes. Very small popup work areas
fall back to the reliable system menu. Read-only text areas are scrollable and
keyboard accessible. Scrollbars appear only for actual vertical overflow, including
wrapped lines. Text/size/font updates remeasure at full width before reserving a
gutter; short status messages and fitting pages have neither bars nor empty gutters.
The same overflow behavior applies to Setup instructions/status. The compact tray
endpoint uses non-scrolling static labels instead.
Hover repainting is event-driven; unchanged Settings state
does not repeatedly reset/repaint all controls.

## Persistence and threading

All settings/registry I/O, network checks, CLI calls and copies run on a background
worker. The UI only queues work and processes native messages. The dedicated
clipboard message thread and existing bounded command acknowledgements are reused.
Quit closes/cancels both UIs and waits for bounded work and the existing API/watcher
shutdown before removing the icon and releasing the singleton. No forced exit.

Existing `settings.json` is edited only after an explicit write-permission change
and confirmation. The save validates the existing schema, keeps all other fields,
checks expected bytes for stale edits, flushes a temporary file and commits with
Windows ReplaceFile (retaining existing security metadata). A sidecar lock serializes
WiniOS saves. External edits detected before commit cause a reload error. This is
not a transaction against a hostile local process concurrently replacing files.

Desktop launch behavior lives in **desktop.json beside the selected settings file**:

```json
{"open_settings_on_launch": false}
```

Missing means false; merely opening Settings creates no file. It uses the same
bounded/atomic save mechanism and contains no secrets. Separating it preserves
compatibility with older agents' strict settings.json schema. Invalid preferences
are reported, not silently replaced. A stale `.winios-preferences.lock` after a
crash must be inspected/removed only while all WiniOS processes are stopped.

## Build, deploy and rollback

```powershell
cd C:\WiniOS
cargo fmt --check
cargo check --offline
cargo test --offline
cargo build --release --offline
.\scripts\Build-Installer.ps1
.\scripts\Test-Installer.ps1
```

The installer retains its existing identity, per-user path and ownership rules;
it now includes this guide. Back up the installed GUI/CLI and the prior installer
before deployment. Gracefully stop the old app, install the verified package at
the same path, compare protected settings/TLS/startup/Serve and relaunch exactly
one GUI process. No network/trust/credential configuration is changed by deployment.

Rollback: Quit, run the saved older compatible installer or restore the matching
backed-up pair and then repair package ownership with that installer. `desktop.json`
can remain; older builds ignore it. Existing write permission remains in the old
compatible schema. Do not delete settings, token, TLS or reset Serve. See the
[installer rollback guide](mission-007.md).

## Manual visual/accessibility acceptance

1. Open Settings and navigate all five pages with mouse, Tab/Shift+Tab, sidebar
   Up/Down and Enter. Verify names/states with Narrator. Esc/Close must leave
   clipboard service running. Retry and tray Quit must remain responsive.
2. Check dark/light, transparency OFF, high contrast and reduced motion using
   your own Windows preferences. Verify opaque/system-color fallbacks, readable
   focus and no animation when reduced motion is enabled. Maximize/restore and
drag between different-DPI monitors, including a negative-origin monitor.
   View text directly for at least a minute without screen capture; opening or
   closing a capture overlay must never be needed to make it visible.
3. Open the tray at every screen edge; verify Esc/outside-click dismissal and
   keyboard actions. Try `--native-menu` and a small/high-DPI work area.
4. General: confirm actual startup registration; test launch behavior ON/OFF,
   restore your preferred values, and verify startup launches stay quiet. Do not
   override Windows Startup Apps' separate disabled/policy state.
5. Clipboard: use disposable text. Pause/Resume must affect GET/POST immediately.
   Save a permission change only after reviewing its warning; before restart the
   active permission stays unchanged. Apply/restart preserves Pause. Restore the
   original permission deliberately, then repeat both physical iPhone Shortcuts.
6. Privacy: navigation must never expose a token. Cancel the token warning first;
   nothing should be read/copied. If deliberately confirming export, keep it private,
   verify it cannot be returned by GET and replace it with disposable text afterward.
7. Trigger a connection failure, missing/invalid preferences or a stale external
   edit. Status must report real failure and saving must not overwrite stale data.
   Quit while testing/copying, relaunch, and confirm one loopback listener/agent.

Hidden test-owned window bitmaps in `target/fluent-*.bmp` validate opaque layout
without capturing or interacting with the desktop. They do not prove live DWM
translucency, Narrator, a real DPI transition or physical iPhone transfer passed.

## Verification on this Windows laptop (6 November 2026)

`cargo fmt --check`, `cargo check --offline`, `cargo test --offline` and
`cargo build --release --offline` passed; the artwork/visibility follow-up's test
suite, including the scrollbar follow-up, has **90 passed, 0 failed, 3 ignored**.
The ignored read-only existing-setup HTTPS probe was
also run explicitly and passed. Native tests require permission for isolated
registry keys, atomic temporary-file replacement and local test sockets; the
restricted execution sandbox alone cannot run all of them.

The installed Settings window was reviewed interactively: General, Connection
and About rendered, Connection's HTTPS/authentication test succeeded, and Esc
closed Settings while the service stayed running. Hidden native fixtures render
all five pages. `scripts/Test-Tailnet.ps1 -NativeClipboard` passed against the
existing certificate-validated HTTPS endpoint: GET/POST authentication, no-store,
acknowledged writes, Unicode and line-ending fidelity, contention exhaustion and
GET/local-copy recovery. GUI/CLI executable subsystem checks also passed.

Deployment compared installed payload hashes and protected settings/TLS files,
Credential Manager fingerprint, startup registration and Serve configuration;
all were retained. The previous package and binaries are backed up at
`C:\WiniOS\target\compact-tray-backup-20261106-213901` (before the compact tray follow-up).
The scrollbar follow-up backup is `C:\WiniOS\target\scrollbar-backup-20261106-212812`.
The earlier artwork/visibility backup is `C:\WiniOS\target\brand-backup-20261106-211311`.
The final installer is
`C:\WiniOS\target\dist\WiniOS-Setup.exe`.

Full notification-area mouse interaction, Narrator announcements, OS preference
transitions, real mixed-DPI monitor moves, login/autostart and physical iPhone
Shortcuts still need the manual acceptance steps above for this build. Existing
hardware verification from earlier missions is not a new test of this UI update.

The artwork/opaque-surface follow-up adds regression tests for supplied icon alpha,
native no-glass policy across repeated theme refreshes, small/large Settings
geometry and hidden branded tray rendering/action dispatch/dismissal. The user
confirmed on the physical laptop display that text stays visible while switching
pages after installing this fix. Do not treat hidden
bitmaps or capture screenshots as proof that live scanout is correct: the original
bug specifically differed between capture and the physical display. Direct visual
confirmation remains part of acceptance on other hardware. Full-client Mica and
Acrylic are deliberately disabled until an alpha-safe rendering architecture is
implemented; the app uses the reliable opaque native design now.
