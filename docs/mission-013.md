# Mission 013 — explicit update checks

Initially delivered as a source-only change. Deployed by explicit request on
2026-11-10; the GUI/CLI binaries remain version 0.1.1. The existing release
installer was not rebuilt and still contains the earlier Mission 012 payload.

In the updated native **Settings → About**, choose **Check for updates**.
The installed/running binary's embedded Cargo version is compared numerically
with the configured publisher's latest stable GitHub release. The page displays
the result, version, plain-text release notes and validated official release URL.
**View Update** opens that page only on your explicit click.
No installer or asset is downloaded or executed by WiniOS. Existing About help,
connection diagnostics and safe diagnostics remain available.

## Configure a trusted source

There is deliberately no assumed official repository. Configure only a public
GitHub repository whose publisher you have independently verified. Replace the
placeholder in `updates.json.example`, then save **updates.json beside the active
settings.json**; the normal path is `%LOCALAPPDATA%\WiniOS\updates.json`:

```json
{
  "github_repository": "TRUSTED-OWNER/WiniOS"
}
```

Do not add this field to settings.json or desktop.json. Existing strict schemas,
settings and credentials are preserved. No configuration is created or rewritten
by the checker. A missing sidecar means Not configured and sends no request.
An invalid sidecar produces a safe error rather than guessing another source.
The bounded sidecar accepts only `github_repository`, an ASCII `owner/repository`
pair. The owner uses letters/digits/hyphens (up to 39 characters); the repository
can also contain internal dots/underscores/hyphens (up to 100 characters). Both
segments must start/end alphanumerically. URLs, escapes, tokens and other fields
are rejected. No Git remote or saved GitHub login is consulted.

Checks reread the file on demand, so no restart is needed after editing it. To
disable checks, remove only this optional sidecar; other settings remain intact.
Mission 021 adds separately opted-in automatic checks and notification history;
see [automatic checks](mission-021.md). Manual checks remain available.

## Network and trust boundaries

- One metadata-only GET goes to
  `https://api.github.com/repos/{owner}/{repository}/releases/latest`, using native
  WinHTTP on the existing background Settings worker. No UI-thread network work,
  new runtime/dependency, agent bearer header, cookie or ambient Windows login.
- TLS 1.2 or newer uses Windows certificate/hostname verification. HTTPS redirects
  are refused; HTTP, arbitrary servers, alternate ports and executable URLs are
  never accepted as release sources. Repository transfers/renames require explicit
  configuration review. A browser may subsequently redirect a page under its own
  normal security rules; WiniOS does not control or download through that browser.
- Header limit 16 KiB; metadata/body limit 256 KiB; displayed notes limit 4,000
  characters. Notes are inert plain text with control/bidi characters removed.
  The request has two-second per-I/O timeouts and a ten-second cooperative deadline
  checked between blocking calls. It is not a hard real-time ten-second guarantee;
  an in-progress Windows call can finish later. Close/Quit requests cancellation,
  and the worker releases WinHTTP handles after the bounded call returns.
- Only stable `X.Y.Z`/`vX.Y.Z` tags are accepted: three unsigned decimal components,
  no leading-zero ambiguity, prerelease/build suffix or overflow. Draft/prerelease
  flags are rejected. Equal/older versions never suggest a downgrade.
- The release URL must match HTTPS `github.com`, the configured repository and
  the exact validated tag under `/releases/tag/`. No userinfo, query, fragment,
  encoded path or asset/download links. The source is reread and matched again
  before an explicit browser launch. A failed new check clears the previous link.
- A trusted repository response identifies release metadata, **not an authenticated
  executable**. Asset URLs/hashes/digests are ignored. Independently review publisher
  identity, code signing and compatibility before any manual download/install.
  Never put GitHub personal access tokens or WiniOS credentials in this file.
- GitHub receives the public metadata request and normal connection information;
  clipboard content, WiniOS endpoint/token, settings, certificates and diagnostics
  are never attached. Private repositories/authenticated feeds are unsupported.

Offline, DNS/TLS failure, timeout, malformed/oversized metadata, no public release,
redirect and rate-limit results are reported in About. They never become a false
“up to date”, trigger an HTTP fallback, interrupt sharing or change Tailscale.
Direct internet access to GitHub is required; proxy-authenticated-only networks
may fail safely because no proxy/Windows credentials are used.

Reference behavior: [GitHub latest-release API](https://docs.github.com/en/rest/releases/releases#get-the-latest-release),
[WinHTTP request options](https://learn.microsoft.com/en-us/windows/win32/winhttp/option-flags).

## Focused verification and manual checks

```powershell
cargo check --offline
cargo test --offline --lib updates::tests
cargo test --offline --lib update_action_uses_background_worker_without_config_changes_or_network
cargo test --offline --lib native_settings_navigation_live_pause_close_and_render_cache
cargo test --offline --lib cards_and_controls_fit_small_and_large_native_client_areas
```

The tests use local metadata/configuration/native-window fixtures, not the live
clipboard agent or a public release service. They cover version ordering, malicious
URLs/configuration, malformed/oversized data, notes limits, redirects/rate limits,
cancellation/deadlines, changed-source browser refusal and the native About action.
No real internet interruption, live GitHub check, browser launch, release GUI
visual/accessibility or iPhone validation is claimed by these tests.

Recorded on 2026-11-10: `cargo check --offline`, `cargo fmt --check` and all
**11 targeted tests** above passed (eight update tests and three affected native
Settings tests). Cargo reported incremental-cache hardlink fallbacks to copying;
there were no compile or test failures. No full suite, release build, installer,
installation or agent restart was performed. Changed files: `src/updates.rs`,
`src/lib.rs`, `src/preferences_window.rs`, `updates.json.example`, README and this
guide. No dependency, API, existing setting schema or packaging changes.

After a separately approved deployment, manually configure a trusted repository,
check newer/equal/older releases, inspect notes/link and open the page explicitly.
Try offline/unconfigured/no-release/rate-limit cases and Close/Quit during a check;
sharing should stay intact. Confirm small-window/DPI/keyboard access to both new
actions. Existing APIs, Smart Send/Receive, HTTPS/Tailscale, Pause, singleton,
autostart and credentials are unchanged.

## Requested deployment — 2026-11-10

One `cargo build --release --locked --offline` passed (22.96 s), using the
established `target/installer-build` cache and static CRT. Backed up and verified
the installed GUI/CLI hashes, quit only WiniOS gracefully, replaced the binary
pair in the existing installed directory under the singleton gate, and launched
one updated tray instance. No installer rebuild or test suite was run.

Observed the Connected tray popup and native About page. Clicking Check for
updates displayed **Update source not configured**, configuration instructions
and **No request was sent**. Open official release page remained disabled. No
repository was invented and no updates.json was created. Live GitHub metadata
and browser opening remain untested because no trusted source is configured.

Before/after checks matched settings, Credential Manager fingerprint, TLS files,
startup preferences/StartupApproved, Serve, Start Menu shortcut and installer
registration. Installed binary hashes and one running process were verified.

Rollback folder: `C:\WiniOS\target\mission013-deploy-backup-20261110-163836`.
Gracefully quit the installed app, wait for exit, restore only the backed-up
`winios.exe` and `winios-agent.exe` to `%LOCALAPPDATA%\Programs\WiniOS`, then
launch installed `winios.exe --tray`. Do not change settings, credentials,
certificates, Tailscale routes or uninstaller metadata. Deployment hashes and
process details are recorded in `target/mission013-deployment.json`.
