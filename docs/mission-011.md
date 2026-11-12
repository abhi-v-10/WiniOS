# Mission 011 — small UX polish

Initially delivered as source only; deployed on 2026-11-10 by explicit request.
No API, Shortcut URL/header, setting, credential, clipboard-engine, TLS/Tailscale
or layout/style change. No new polling, networking, diagnostics collection or logs.

## Changes

- Tray/header/tooltip keep **Paused — reconnecting** visible during Retry. Quit
  takes precedence and shows Stopping/Stopped, rather than an obsolete Paused label.
- The existing Connection page and explicit Copy safe diagnostics include a short
  next step based on current connection/pause status. Connected describes the
  connection; it does not claim the clipboard has transferable content.
- Safe diagnostics use Enabled/Disabled/Not known instead of `Some(true)`-style
  permission flags and include the already-known active image permission. They
  still contain no clipboard contents, bearer token, version IDs or request bodies.
  Generating this text reads only the UI model; it does not query the clipboard,
  network, registry or credentials. Existing private-copy protection is unchanged.
- The connection-test 503 message distinguishes Resume-if-paused from waiting or
  retrying after a connection change. It no longer always prescribes Resume.

## What the existing responses mean

| Situation | Correct action |
| --- | --- |
| Cannot connect / DNS unavailable | Keep laptop awake and WiniOS running; connect Tailscale on both devices and wait for verified Connected. Retry reconnects WiniOS, not Tailscale itself. |
| TLS/certificate error | Stop and check the configured HTTPS identity. Never bypass certificate validation or use plaintext as a fallback. |
| Authenticated 503 while tray says Paused | Resume sharing explicitly. Retry preserves Pause. |
| Authenticated 503 while offline/starting | Restore connectivity or wait for startup, then retry. 503 can also mean pending/unavailable clipboard data; it is not always a Pause error. |
| Type `empty`, or legacy GET 204 | No supported content to receive. Stop without copying or claiming success; copy new supported content on Windows first. Image opt-out can also make image-only content unsupported. |
| HTTP 412, or 409 during a sequence race | Windows clipboard changed. Start a fresh Smart Receive run at the type query. Discard old metadata/version/results; never fetch without the version header as a workaround. This is not a bearer-token error. |
| 401 | Authorization failed. Check the existing token/header privately; do not rotate credentials to fix a 412. |
| 403 | Review the existing image/write opt-in as applicable. Only change permission deliberately. |
| 413 | Content exceeds existing limits; resize/reduce it. Limits are unchanged. |

Bodies/status codes remain unchanged, including empty failure bodies. No automatic
retry, historical clipboard fallback, relaxed authentication or error-body text
that might accidentally be copied to iPhone was introduced.

## Necessary iPhone changes only

No changes are needed if Smart Receive already follows the
[Mission 010 flow](mission-010.md): store fresh kind/version per run, send bearer
and version on the payload GET, stop on empty/missing output/error, and copy only
the successful branch's explicitly selected result.

If missing, add only those stop guards. Keep Copy to Clipboard and success
notifications inside successful text/image branches; do not place them after End
If or in an error/empty branch. On 412, manually rerun the complete Shortcut. Do
not reuse a previous run's result or remove the version header. A Stop and Output
action used on failure must use Do Nothing, not Copy to Clipboard.

Shortcuts may terminate Get Contents of URL with an action error without exposing
a status variable. Let it stop; this mission does not assume a status-catching
action exists or add a workaround. Current two-Shortcut setup is in mission-018.md.

## Verification / application

Verified on 2026-11-10:

```powershell
cargo fmt --check
cargo check --offline
cargo test --offline lifecycle::tests
cargo test --offline safe_diagnostics_describe_permissions_and_recovery_without_reading_clipboard
cargo test --offline fluent_menu_tracks_live_sharing_startup_and_shutdown
cargo test --offline receive_tests
```

All passed: **13 targeted tests**, no failures. Cargo emitted incremental-cache
hardlink fallback warnings (copies were used), not compiler errors. Full-suite,
release-build and installation were initially skipped for this small patch.
No live clipboard or network changes were made for these targeted tests.

Changed files: `src/lifecycle.rs`, `src/preferences_window.rs`, `src/setup.rs`,
README and this guide. No installer or dependency changes.

## Requested deployment — 2026-11-10

Built once with `cargo build --release --locked --offline`, using the existing
`target/installer-build` cache and `RUSTFLAGS=-C target-feature=+crt-static`.
Verified the GUI/CLI payload and installed hashes, then replaced only those two
binaries in the existing per-user installation. No installer was rebuilt.

Verified the visible Connected tray popup and Settings Connected → Paused →
Connected transitions. `scripts/Test-Tailnet.ps1 -ExpectPaused` passed verified
HTTPS health, no-store, authenticated GET/POST 503 and unauthenticated POST 401
without accepting a clipboard write. After Resume and graceful Quit/relaunch,
the default HTTPS health/authentication checks passed again. Quit was verified
through `winios.exe --quit`; automated tray-menu selection dismissed the popup,
so a physical click on tray Quit remains a manual check. The agent/listener
stopped, its singleton was released, and one installed tray process restarted
with its backend bound only to `127.0.0.1:8766`.

Before/after fingerprints confirmed settings, Credential Manager token, TLS files,
startup registration/preference, Start Menu shortcut and Tailscale Serve unchanged.
No clipboard payload was transferred. Offline/412 recovery and physical iPhone
acceptance were not retested. No full suite or second release build was run.

Rollback backup: `C:\WiniOS\target\mission011-deploy-backup-20261110-142518`.
To roll back, gracefully quit the installed app with `winios.exe --quit`, wait
until WiniOS exits, restore `installed\winios.exe` and `installed\winios-agent.exe`
from that backup to `%LOCALAPPDATA%\Programs\WiniOS`, then launch its `winios.exe`.
Do not restore/delete personal settings or credentials. The backup also contains
the prior Mission 010 installer; the existing distributed installer is unchanged.
