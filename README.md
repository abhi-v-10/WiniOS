# WiniOS — private beta 0.1.3

Mission 017 produces packaging-only beta revision `0.1.2-r1`; application binaries
are unchanged. [Resolved and remaining blockers](docs/mission-017.md).

Private-beta candidate **0.1.2** packages current onboarding with a clean,
explicit documentation payload and privacy scan. See the [beta install/rollback
guide and remaining blockers](docs/private-beta.md). Actual Smart Shortcut links
remain pending; this is not a claim of complete beta acceptance.

Mission 015 adds numbered native iPhone setup guidance and a safe connection test.
Install the two standalone Smart Shortcuts using docs/mission-018.md.
See [pairing instructions and validation](docs/mission-015.md).

Mission 014 improves native first-run recovery and Smart Shortcut guidance while
preserving configured installations. See [onboarding and validation](docs/mission-014.md).

Mission 012 packages the existing app in a versioned per-user installer without
installing it automatically. See the [short install/upgrade/rollback guide](docs/release-0.1.1.md).

Settings → About supports manual update checks and optional automatic checks
against a trusted public GitHub repository. Automatic checks are off by default;
WiniOS never downloads or installs updates. See [update notification setup and
security](docs/mission-021.md).

Mission 011 polish clarifies paused/offline/retry status, safe diagnostics and
empty/412 recovery without API or settings changes. Deployed on 2026-11-10 by
explicit request. See [changes, verification and rollback](docs/mission-011.md).

Authenticated **GET `/v1/clipboard/type`** returns only `kind` and an opaque
`version`. A single manually configured iPhone Receive Shortcut can branch to
the existing text or image GET, passing **X-WiniOS-Clipboard-Version**. Changed
observations fail closed instead of returning the wrong data. Existing helpers
and requests without that header retain their contracts; published iCloud links
are unchanged. See [exact Shortcut actions, API/race semantics and manual tests](docs/mission-010.md).

## Stability and security hardening

Mission 009 fixes admitted-upload and native-queue cancellation across Pause or
detected Tailscale readiness loss, aborts stale/backpressured image bodies promptly,
and avoids DIB allocation after cancellation. APIs/settings/Shortcuts and security
identities are unchanged. About includes the requested developer/support footer.
See [audit findings, actual tests/benchmarks, limitations and rollback](docs/mission-009.md).

## Private image clipboard

Bidirectional PNG transfers use separate authenticated **GET/POST
`/v1/clipboard/image`** endpoints. Existing text endpoints and Shortcuts stay
unchanged. Enable images explicitly in **Settings → Images**, select a 2/4/8 MiB
cap and apply/restart; uploads also require the existing clipboard-write
permission. Images default OFF. Native capture/conversion, version checks,
acknowledged writes, Pause/Resume and private HTTPS boundaries apply to images too.
No clipboard images are saved to disk by WiniOS. See [image API, limits, separate
iPhone image Shortcuts, Windows validation and rollback](docs/mission-008.md).

## Native Windows companion

The tray now opens a native Fluent-style popup and a separate **Settings** window
with General, Connection, Clipboard, Images, Privacy and About pages. The supplied app icon
sets the midnight-blue, cyan and violet palette. Rounded native cards use opaque
surfaces so text remains visible on the physical display; full-client glass is
disabled for GDI controls. Live sharing,
saved write permission and saved launch behavior are clearly separated; no token
appears automatically. See [UI controls, persistence, tests and rollback](docs/mission-007-ui.md).
The existing setup wizard remains available through iPhone Setup or `--setup`.

Build **WiniOS-Setup.exe** with `.\scripts\Build-Installer.ps1` (Inno Setup 7).
The native dark installer uses `%LOCALAPPDATA%\Programs\WiniOS`, requests no admin,
adds Start Menu/Installed Apps integration and gracefully stops the agent for
upgrades. Existing data, credentials, TLS, Serve and startup choice are preserved.
Uninstall removes packaged files/shortcut/startup but keeps personal data and warns
about user-managed Serve. See [build/release instructions, actual test results,
security limits and rollback](docs/mission-007.md). Prototype builds are unsigned;
publisher signing and clean-user interactive checks remain release requirements.

The native wizard now includes **iPhone Setup** with the supplied Send/Receive
template links, **Copy HTTPS Endpoint**, and an explicitly confirmed **Copy Bearer
Token** action. Setup copies are private and excluded from WiniOS GET and Windows
clipboard history/sync; other local apps can still read the clipboard. Review both
templates and their Import Questions before entering credentials: their contents
are not assumed credential-free. See [installation, manual audit, testing and
rollback](docs/mission-006b.md). No pairing or automatic Tailscale changes.

Fresh desktop installs now open a dark native **Settings / Setup** wizard for
Tailscale checks, private HTTPS Serve verification, explicit clipboard-write
consent and a connection test. Existing configured installations skip it; reopening
from the tray is a protected read-only review. No automatic network provisioning,
pairing or credential/certificate changes. See [setup behavior, security boundaries,
manual tests and rollback](docs/mission-006a.md). Existing 005C tray/startup features
below are preserved.

The desktop app now has a dark, compact native popup, an embedded WiniOS icon,
a proper per-user Start Menu shortcut, and an explicit **Start with Windows**
checkbox. Startup defaults OFF and changes only when selected. Windows retains
menu keyboard behavior, with a standard-menu fallback for High Contrast or
rendering failure. See [Mission 005C setup, tests and rollback](docs/mission-005c.md).
Saved settings, credentials, certificates and Tailscale configuration are preserved.

The native tray menu now supports **Status**, **Pause/Resume Clipboard Sharing**,
**Retry Connection**, and **Quit WiniOS**. Pause blocks authenticated clipboard
GET/POST with 503 while unauthorized requests remain 401. It does not stop
Tailscale or alter Serve, certificates, permissions or write opt-in. See the
[tray menu fix and manual checks](docs/tray-menu-fix.md).

**Roaming adapter:** see [portable connectivity setup, security boundaries,
tests and rollback](docs/portable-connectivity.md). Tailscale Serve provides the
external HTTPS identity and forwards only to a dedicated loopback backend.
WiniOS never changes Serve, Funnel, firewall or network settings automatically.
The original direct-LAN HTTPS mode below remains available, with its existing
certificate and explicit IP binding. Startup is now an optional Mission 005C feature.

Mission 005B is installed and user-verified over the new tailnet HTTPS URL: Safari
loads without a certificate warning and both Shortcuts transfer text successfully.
The Mission 005C baseline had **69 passing tests**; Mission 006B now has **80**,
plus two explicitly run native/live probes. The real Tailscale HTTPS/native clipboard
integration passes. See [actual results, changed files and remaining
roaming tests](docs/mission-005b-validation.md). The Windows configuration now uses
the explicit loopback backend; the approved Serve route is user-managed and
persists after Quit. The direct-LAN setup below is retained for optional rollback.

The standalone desktop application is **winios.exe**. It uses direct Win32
notification-area APIs and a native message loop, with no web engine, frontend,
Electron, Tauri or browser. The existing **winios-agent.exe** remains a console CLI.
Both use the same library, HTTPS endpoints, credential, clipboard state and native
clipboard worker. Mission 005A creates **no autostart entry** and changes no TLS
identity, trust, network adapter or firewall configuration.

## Desktop setup: use the existing identity

Your already trusted `%LOCALAPPDATA%\WiniOS\tls` identity is reused. Do **not**
regenerate/rotate it just to install tray mode. Existing iPhone URLs, tokens and
Shortcuts do not change. Stop any older WiniOS process before building or replacing
its binary; only one console/tray service may run per interactive Windows session.

```powershell
cd C:\WiniOS
cargo build --release
.\scripts\Test-DesktopBinary.ps1
```

Outputs:

- `target\release\winios.exe`: Windows GUI-subsystem executable; double-click it
  for tray mode with no console. The release PE-header test checks subsystem 2.
- `target\release\winios-agent.exe`: existing Windows console-subsystem CLI
  (subsystem 3), preserved for debugging and native/API tests.

Copy `winios.exe` to a stable per-user folder such as
`%LOCALAPPDATA%\Programs\WiniOS\winios.exe`. No installer, admin privilege or Windows
Service is needed for normal operation. The GUI binary is standalone; it does not
need the source tree or a frontend asset folder. The CLI binary is optional but
useful for diagnostics. Keep certificate/key files in their protected existing
location, not next to downloaded/shared binaries. Moving from the debug CLI to the
release GUI changes the executable path: if necessary, review a Private-profile,
LocalSubnet firewall rule for this **new program path** yourself. No Public rule
or automatic firewall exception is created.

For a fresh **Tailscale** installation, launch the GUI without creating settings
and follow its wizard. Existing settings are preserved automatically. The manual
**direct-LAN HTTPS** alternative below remains supported; do not overwrite an
existing `%LOCALAPPDATA%\WiniOS\settings.json`:

```powershell
$settingsDir = Join-Path $env:LOCALAPPDATA 'WiniOS'
New-Item -ItemType Directory -Force -Path $settingsDir | Out-Null
$settingsPath = Join-Path $settingsDir 'settings.json'
if (-not (Test-Path -LiteralPath $settingsPath)) {
    Copy-Item -LiteralPath .\settings.json.example -Destination $settingsPath
}
notepad $settingsPath
```

Minimal UTF-8 JSON (the example defaults writes to **false**):

```json
{
  "lan_ip": "192.168.1.7",
  "port": 8765,
  "allow_clipboard_write": false
}
```

Set `allow_clipboard_write` to `true` yourself to enable the working POST Shortcut.
This permission is persisted safely as a boolean; there is no insecure-write flag
in tray mode. The configured IP must be assigned to your PC and match the existing
certificate's **IP SAN**. No wildcard/public/automatic bind or plaintext option is
accepted. Optional `tls_cert` and `tls_key` must be supplied together as absolute
local file paths (escape backslashes in JSON). Omit both to use the existing default
identity. Do not put a token, password, private key, CA signing key or other secrets
in settings. Unknown fields, non-private IPs, UNC/device/relative paths and files
larger than 16 KiB are rejected. Local paths are resolved and checked before use.

Launch by double-clicking the installed `winios.exe` in Explorer. No terminal is
needed. An explicit alternative config path is supported for controlled testing:
`winios.exe --config C:\absolute\settings.json`. Running settings are not silently
reloaded: Quit and relaunch or choose **Retry Connection** after changing them.
Retry reads settings/certificate files again after the old worker has stopped.

## Tray behavior and graceful Quit

Right-click the WiniOS W icon (possibly under the notification-area overflow arrow):

- **Status**: opens a metadata-only dialog with connection state and HTTPS endpoint.
  The menu and tooltip also show when clipboard sharing is Paused.
- **Pause/Resume Clipboard Sharing**: blocks authenticated clipboard GET/POST with
  503 while keeping unauthorized requests at 401. It leaves Tailscale connected.
- **HTTPS endpoint**: address/port only, never a credential.
- **Start with Windows**: actual per-user WiniOS Run registration, default OFF.
  Select the checkbox to enable/disable it. Windows Startup Apps policy remains
  separate and is never overridden. Other applications' startup entries are untouched.
- **Retry Connection**: gracefully stops the existing worker, rereads settings,
  and starts one replacement after cleanup. Pause survives Retry. It cannot start
  overlapping servers or silently select another network IP.
- **Quit WiniOS**: requests orderly application shutdown, not process termination.

Quit signals the background service, stops accepting API work, prevents pending
native writes from starting, removes the clipboard listener/window, joins the
worker, and then removes the tray icon and releases the singleton. The UI continues
processing messages while shutdown proceeds. Already accepted native writes cannot
be undone. The normal HTTP drain remains bounded to five seconds. A Windows
clipboard provider stuck inside a synchronous Win32 call may delay native cleanup;
there is no unsafe thread termination to fake an immediate Quit.

For rollback/automation if the icon is hidden, `winios.exe --quit` posts the same
local graceful close request to the tray app. It starts no server/watcher. Wait for
process exit before replacing the executable. It does not stop the separate console
agent; that mode still uses Ctrl+C/Ctrl+Break.

The tray loop owns only icon/menu/window resources. HTTPS runs on a background
Tokio thread, and the existing clipboard message loop remains on its dedicated OS
thread. No network or clipboard retry waits run on the tray UI thread. State changes
wake the UI via native messages. Windows address-removal notifications stop the
service if the configured IP disappears; no address polling or automatic rebind is
used. Other connectivity faults (firewall, AP isolation, interface down while its IP
remains assigned) still need connectivity checks; Running means the service bound,
not that every phone can reach it. Explorer-restart notifications re-add the icon.

## Errors, diagnostics and recovery

Missing settings, missing TLS files, invalid/expired certificates, IP-SAN mismatch,
bind conflicts and unavailable addresses fail visibly with an error dialog/status.
A mismatched certificate is never regenerated or trusted automatically, and no
HTTPS failure downgrades to HTTP. Network unavailable does not select a replacement
IP. Restore the original assigned IP or safely reprovision identity as documented
in Mission 004A, then fix settings and Retry. If an IP is removed while running,
restart requires explicit Retry after the failure has settled.

The GUI has no disk log and disables stdout/stderr diagnostics. No clipboard text,
request bodies, bearer tokens, passwords or private keys are logged. For metadata-only
console diagnostics after quitting tray mode:

```powershell
.\target\release\winios-agent.exe --lan 192.168.1.7 --port 8765 --allow-clipboard-write
```

Alternatively, `winios.exe --console <CLI options>` explicitly attaches/allocates a
console and runs console mode instead of a tray. This is the only GUI-executable
path that calls console allocation APIs. The normal no-argument launch never does.
The existing explicit token-export command stays in console mode, separate from
normal logs; do not redirect its secret output. Use `winios-agent.exe --help` for
CLI options. Legacy insecure development mode is not available through tray settings.

## Mission 005A validation and remaining manual checks

Required checks:

```powershell
cargo fmt --check
cargo check
cargo test
cargo build --release
.\scripts\Test-DesktopBinary.ps1
```

The Mission 005A automated suite had **46 passing tests** (now 62 with the tray menu fix), including lifecycle/error-retry models,
sticky early shutdown, singleton duplicate rejection/release, safe persisted config,
certificate IP/expiry/CN-only rejection, network notification registration/removal
policy, API authentication/limits/continuity, TLS negatives, and queued-write shutdown.
The release PE test confirms GUI/no-automatic-console and preserved console binaries.

The existing HTTPS native integration script passed against the release GUI-hosted
service, using the existing certificate and a temporary explicit-write config:

```powershell
.\scripts\Test-ClipboardPost.ps1 -BaseUri https://192.168.1.7:8765 -CaCertificate (Join-Path $env:LOCALAPPDATA 'WiniOS\tls\winios-ca.cer')
```

It verified exact Unicode/LF/CRLF writes, GET readback, authentication, repeated
operations, temporary contention recovery, exhaustion without a false success, and
subsequent local-copy recovery. The independent graceful `--quit` test completed.
A four-second idle sample of the installed app measured **0 ms CPU**
and **17.3 MiB working set**; this is one observation, not a universal performance
guarantee. The native loops are event-driven and contain no idle polling timer.

**User-verified on actual devices:** the installed tray app is visible and both
Windows → iPhone and iPhone → Windows copy/paste work. It is installed under
`%LOCALAPPDATA%\Programs\WiniOS`, using `%LOCALAPPDATA%\WiniOS\settings.json`
originally with HTTPS at `192.168.1.7:8765` and authenticated writes enabled. Mission
005B now selects the tailnet backend as documented above. The existing
certificate and Credential Manager token were reused without rotation or trust
changes. The GUI app was left running after setup; no autostart was registered.

Computer Use was stopped by the user's physical Escape key, so no claim is made
that the agent interactively exercised tray-menu commands. The independent
graceful `--quit` path was tested. Explorer restart, actual adapter/IP removal,
startup error dialogs and interactive Retry/Quit remain additional manual checks:

1. Stop the old console agent. Put the release GUI in its stable folder, create
   settings using the existing LAN IP/cert, and opt into writes if desired.
2. Double-click the GUI in Explorer. Verify no terminal appears, one W icon appears
   (check overflow), and its menu reports Running, the correct HTTPS endpoint and
   display-only startup status. Launch it again: no second watcher/server should
   start; the second launch should report that WiniOS is already running.
3. Use both existing iPhone Shortcuts and paste into Notes/Notepad. Run the HTTPS
   integration script using disposable text. The token/URLs/JSON remain unchanged.
4. Quit from the tray while idle and during queued requests. Verify pending writes
   fail or finish safely, the endpoint stops responding, the icon disappears and
   the process exits. Relaunch to verify mutex release and no orphan listener.
5. While stopped, use a separate test settings file with a wrong IP, invalid key
   path or unavailable port. Verify Error/Network unavailable and no API listener.
   Correct it and use Retry. Never change trust or accept a bad certificate as a fix.
6. If convenient, test actual network/IP removal: expect service stop and Network
   unavailable with no rebind. Restore the same address and Retry; a permanent new
   address requires a safely matching identity. Do not disrupt active work merely
   for this test. Explorer restart/re-add behavior is another optional manual check.
7. Observe Task Manager at idle. No constant CPU activity should be caused by the
   tray or clipboard loops; repeat with the icon tooltip/menu open as well.

To roll back, Quit the GUI (or use its graceful `--quit` control), wait for exit,
and launch the existing console CLI with the same HTTPS arguments. Remove only the
copied desktop executable/temporary test settings if desired. Leave certificates,
Credential Manager and iPhone trust intact for the console agent. No startup entry,
scheduled task, Windows Service or firewall rule needs to be undone by Mission 005A.

## Existing Mission 004A HTTPS/certificate reference

The following sections retain the certificate lifecycle and API reference. Normal
Mission 005A installation reuses the existing identity; do not rerun certificate
provisioning as a tray-install step.

# WiniOS — Mission 004A: HTTPS on a private LAN

Stage A adds Rustls HTTPS around the physically verified Win32 clipboard core.
There is no device pairing, cloud relay, Bluetooth, frontend, automatic forwarding,
port forwarding, firewall modification, or periodic clipboard polling.
Bearer authentication remains required for both clipboard endpoints. HTTPS protects
it in transit when clients validate the correct server certificate. Continue using
non-sensitive/disposable text during Stage A validation.

## Architecture and defaults

- Axum routes, auth, state machine, native message loop, command queue, origin
  tracking, Unicode handling, acknowledgements and native retries are unchanged.
- `src/transport.rs` uses maintained `axum-server` 0.8 / Rustls 0.23 with the explicit
  ring provider and TLS 1.2/1.3. The actual accepted TCP peer reaches `ConnectInfo`;
  proxy headers are not trusted. The test suite proves this with a `127.0.0.2` client.
- HTTPS is the default, including loopback. Default bind remains `127.0.0.1:8765`.
  LAN requires a specific assigned RFC1918 IPv4 via `--lan`; wildcard/public/IPv6
  binds remain rejected. Only private/loopback IPv4 peers reach the API.
- The default identity paths are `%LOCALAPPDATA%\WiniOS\tls\server.cert.pem` and
  `server.key.pem`. Override with **both** `--tls-cert` and `--tls-key`.
- Missing/invalid PEM or mismatched keys fail startup before binding. Expired,
  untrusted or wrong-identity certificates are refused by validating clients.
  Rustls loads identities but does not replace client-side certificate validation;
  monitor certificate dates. There is no HTTP fallback or redirect on this port.
- The single Win32 OS thread owns clipboard reads/writes and Ctrl+C/Ctrl+Break.
  Tokio runs the API. Shutdown closes queued writes and drains the server for up
  to five seconds; TLS handshakes are limited to three seconds. A single-instance
  Windows mutex prevents multiple service instances in the session.

## 1. Provision certificates on Windows

Use PowerShell **7.4+**, the existing MSVC Rust toolchain and Windows SDK. No Mac,
Xcode, additional iOS app, OpenSSL installation, or commercial/public CA is needed.
Stop older agents before rebuilding; Windows locks running executable files.

```powershell
cd C:\WiniOS
cargo build
.\scripts\New-WiniOSTlsCertificate.ps1 -IpAddress 192.168.1.7
```

The script prompts securely for a **unique, strong CA password, at least 20
characters**. Prefer a long random password held in your password manager. Do not
put it on a command line, in source, in logs, or in a plaintext configuration file.

What is created (no certificates are trusted/installed automatically):

| Location | Contents / handling |
| --- | --- |
| `%LOCALAPPDATA%\WiniOS\ca-offline` | Public CA certificate and password-encrypted `ca.key.encrypted.pem`. Keep signing key + password separate from routine service operation. |
| `%LOCALAPPDATA%\WiniOS\tls` | Server-only `server.key.pem`, `server.cert.pem`, and public `winios-ca.cer`. Only the public `.cer` goes to the phone. |

Both directories receive protected Windows DACLs allowing only the current user
and SYSTEM; new key files inherit those permissions. Paths inside the Git workspace,
reparse-point paths and nested CA/runtime directories are rejected. The CA key uses
PBKDF2-SHA256 (210,000 iterations) / AES-256 encrypted PKCS#8. The runtime server
key is unencrypted PKCS#8 PEM protected by its DACL, because unattended Rustls must
read it. The agent never reads the CA signing key or asks for its password.
Administrators can take ownership, and malware running as your user can read the
runtime key. ACLs do not protect a compromised Windows account.

**A directory named `ca-offline` is not automatically offline.** After issuance,
move its encrypted signing material to a protected offline NTFS/BitLocker volume
and keep the password separately. Alternatively provision directly there:

```powershell
.\scripts\New-WiniOSTlsCertificate.ps1 -IpAddress 192.168.1.7 -CaDirectory E:\WiniOS-CA-192.168.1.7
```

Use an NTFS location supporting Windows ACLs; unsupported permissions cause failure.
Keep an encrypted backup. The server needs only the runtime leaf identity. Neither
private key belongs in Git; `.gitignore` additionally excludes PEM/key/PFX/P12 files.
Do not copy whole CA/runtime directories to the phone.

The dedicated CA and separate leaf use ECDSA P-256 / SHA-256. CA lifetime is one
year, `CA=true`, path length zero, cert-signing usage and serverAuth-only EKU.
Critical name constraints permit only the selected private IP and `127.0.0.1`,
and the reserved `winios.invalid` DNS subtree. The leaf is **not a CA**, has
serverAuth EKU, digitalSignature usage, and at most 90 days validity (also capped
by CA expiration). Its **IP subjectAltName** contains `192.168.1.7` and loopback;
identity does not rely on CN or wildcards. The reserved leaf CN is also within the
CA's DNS constraints for Windows compatibility. Issuance validates the chain with
Windows before publishing the identity. The public CA's SHA-256 fingerprint is printed.

## 2. Run HTTPS

After provisioning at the default runtime path:

```powershell
cargo run -- --lan 192.168.1.7 --port 8765 --allow-clipboard-write
```

POST remains opt-in. Omit `--allow-clipboard-write` for read-only HTTPS. It requires
explicit private LAN binding and cannot be combined with the diagnostic flag.
Non-default runtime paths use:

```powershell
cargo run -- --lan 192.168.1.7 --port 8765 --tls-cert 'D:\WiniOS-runtime\server.cert.pem' --tls-key 'D:\WiniOS-runtime\server.key.pem' --allow-clipboard-write
```

The URL is `https://192.168.1.7:8765/v1/health`. Keep the terminal running; Ctrl+C
stops both subsystems. A private certificate's trust is not automatic. An untrusted
browser warning is a failed test, not an instruction to click through.

## 3. Review and trust only the public CA on iPhone

Installing a root adds an explicit trust anchor to the device, potentially affecting
**other apps and sites**, not just this Shortcut. The constraints reduce scope on
clients that enforce them, but do not assume every trust engine enforces constraints
on trust anchors identically. CA-key compromise can therefore be serious. Use this
CA only for WiniOS, only on devices you control, never as a public/global or shared
organization CA. The script never changes Windows or iOS trust stores.

1. On Windows, record the printed SHA-256 CA fingerprint. Verify it independently:

```powershell
$runtimeTls = Join-Path $env:LOCALAPPDATA 'WiniOS\tls'
certutil -hashfile (Join-Path $runtimeTls 'winios-ca.cer') SHA256
```

2. Transfer **only** `winios-ca.cer` using an existing trusted attachment/file-transfer
   channel. For example, the built-in iPhone Mail app can open a public `.cer`
   attachment sent from Windows. WiniOS implements no mail/cloud transfer service.
   Do not download and trust an unauthenticated substituted root. Verify the fingerprint
   out of band from the Windows screen; inspect iOS certificate details/fingerprints
   where available. If the UI cannot show the fingerprint, establish file authenticity
   through your trusted transfer channel before trusting it; do not waive review.
3. Tap the public certificate. In Settings, review **Profile Downloaded**, or
   **General → VPN & Device Management**. Review the WiniOS CA identity, dates and
   certificate details, then explicitly install it using the device passcode.
4. Go to **Settings → General → About → Certificate Trust Settings** and explicitly
   enable full trust for this WiniOS root. Review the warning. Installation alone
   does not grant TLS trust. [Apple's manual trust instructions](https://support.apple.com/en-us/102390).
5. In Safari, open **https://192.168.1.7:8765/v1/health**. It must load fixed
   non-sensitive JSON without any certificate warning. If it fails, stop here and
   correct trust/IP/date/chain problems. Never ignore certificate errors or disable
   TLS verification.

Never send `ca.key.encrypted.pem`, `server.key.pem`, their passwords, a private-key
PEM, or a PFX/PKCS#12 to iPhone. The public `.cer` contains no private key. No PKCS#12
export/import is used by this workflow.

## 4. iPhone Shortcuts

Install only the two standalone Smart Shortcuts using [Mission 018](docs/mission-018.md).

## API behavior remains unchanged

| Request / condition | Status / body |
| --- | --- |
| GET `/v1/health`, no auth | 200 fixed liveness JSON, no clipboard/token/status details |
| GET/POST clipboard missing or invalid bearer | 401 empty + Bearer challenge |
| GET available, native sequence still current | 200 exact UTF-8 `text/plain; charset=utf-8` |
| GET empty / stale / unavailable | 204 / 409 / 503, no cached text |
| Oversized text | 413 |
| POST not enabled or peer forbidden | 403 |
| POST malformed JSON / wrong content type / invalid schema or NUL | 400 / 415 / 422 |
| POST accepted by Windows | 200 `{"status":"ok","operation":N,"sequence":S}` |
| POST body timeout / ack timeout / busy-or-stopped / native failure | 408 / 504 / 503 / 500 |

All responses retain `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.
Tokens, clipboard text, bodies and private keys are not service logs. The legacy
`--debug-clipboard` name enables only metadata/error diagnostics in read-only mode.

POST accepts only `{"text":"..."}`. UTF-8 text and UTF-16 source are each capped at
1 MiB, preserving the previous limit (ASCII therefore fits at most 524,288 characters).
A bounded 6 MiB + 64 byte HTTP body allows JSON escape expansion before deserialization.
Malformed Unicode and embedded NUL are rejected; LF, CRLF and other valid Unicode
are preserved. Authentication occurs before body parsing. Body read timeout is
five seconds; the four-slot command queue plus one active native command is unchanged.
Native writes still run only on the Win32 message thread with ownership-transfer
RAII and bounded timer retries. Expired/disconnected queued commands are skipped.
A 504/lost connection can be an **uncertain** outcome if native commit was already
underway: inspect Windows before retrying. There is no durable replay/idempotency
protocol. If EmptyClipboard succeeded before a native SetClipboardData failure,
previous formats may have been lost; the state is invalidated, never falsely served.

## Bearer credential lifecycle

The existing 256-bit token remains in per-user Windows Credential Manager as
`WiniOS/Agent/Bearer/v1`. HTTPS does not change or rotate it. To explicitly reveal
it for initial Shortcut setup, use the separate export command (never redirect it
to a log or file):

```powershell
.\target\debug\winios-agent.exe --show-token
```

It starts no watcher/server. To rotate: stop the agent, remove only this Generic
Credential through Credential Manager, restart, and replace both Shortcut headers.
Rotate after any exposure during earlier plaintext development or suspected MITM.
Other processes running as your Windows user can read this credential.

## Renewal, IP changes, revocation and compromise

- Check `ipconfig` and certificate expiry before testing. A DHCP reservation for
  `192.168.1.7` reduces churn but does not replace certificate identity validation.
- For routine leaf renewal, stop the agent, reconnect the protected offline CA,
  and run the provisioning script with the **same IP and CA directory** plus
  `-Renew`. Enter the CA password privately. It creates a new leaf key/certificate
  under the same CA, preserving the CA fingerprint. Restart the agent; return the
  signing material offline. Renew before the 90-day leaf deadline.
- The script refuses renewal with under 30 days CA life remaining. Rotate the CA
  in new, separate directories, issue a new leaf, explicitly review/install/trust
  the new public CA on iPhone, and remove the old profile/root trust. Never overwrite
  the only offline backup before migration succeeds.
- IP changes require a **new scoped CA and leaf**, because the existing CA is
  constrained to the old IP. Use new CA/runtime directories (or move the previous
  identities securely aside), provision for the new private IP, replace iPhone root
  trust, restart with new paths/address, update both HTTPS URLs and any narrowly
  scoped firewall rule, and remove old trust. Never change URLs to evade mismatch.
- This offline CA has no OCSP/CRL infrastructure. Native test clients intentionally
  use no online revocation fetch; signature, date, EKU, constraints and hostname
  validation remain enabled. Revocation is manual removal of the trusted root/profile
  and replacement of identities. If a **leaf key** is compromised, issuing another
  leaf under the same CA does not immediately revoke the stolen one: rotate the CA
  and remove old trust from every client. Do the same after CA-key compromise, and
  rotate bearer tokens. Short validity limits exposure but is not revocation.
- On iPhone, disable this root in Certificate Trust Settings and remove its WiniOS
  profile under VPN & Device Management. Do not remove unrelated profiles. If you
  explicitly imported it into a Windows user root store, remove only its verified
  thumbprint there too. Provisioning/test scripts do not install any root for you.

## Firewall and troubleshooting

Keep the existing **Private-profile, program/port, LocalSubnet** firewall boundary.
HTTPS uses the same TCP port 8765 and executable path; normally no new rule is needed.
Do not disable the firewall, add Public-profile rules, forward ports, add cloud
relays/tunnels, or expose wildcard listeners. No rules are changed automatically.

For timeouts check PC sleep, printed IP/port, matching Wi-Fi, guest/AP isolation,
VPN routing, iOS local-network permission, Private profile and existing executable
path in the firewall rule. Certificate errors are different from connectivity:
check the exact URL IP against the IP SAN, device clocks, leaf/CA expiration, correct
root fingerprint and explicit iOS full-trust switch. A 401 means HTTPS connected but
the bearer header is wrong. A 403 POST means the write flag is absent. 409/503 means
retry once the clipboard settles. Never solve a TLS failure with a verification
bypass, a CN-only certificate, or automatic downgrade.

## Explicit legacy development mode only

Normal LAN commands no longer expose plaintext. To intentionally run the old
transport for controlled disposable-text development only:

```powershell
cargo run -- --insecure-http --lan 192.168.1.7 --port 8765 --allow-insecure-clipboard-write
```

Both plaintext mode and plaintext write permission must be explicit. TLS identity
flags cannot accompany `--insecure-http`; the secure write flag cannot authorize
plaintext writes. This is a separate listener mode, not fallback from HTTPS. Stop
one agent before starting another. Never use legacy mode to work around a bad
certificate. The integration script also requires explicit `-AllowInsecureHttp`
for that mode; its normal path requires HTTPS with full validation.

## Design and limits

- `src/watcher.rs` owns a message-only window, `AddClipboardFormatListener`, and
  a `GetMessageW` loop handling `WM_CLIPBOARDUPDATE`. The listener is registered
  before the initial read; changes during startup remain queued. No clipboard polling.
- Clipboard open/lock guards close and unlock on every return path. Windows owns
  the data handle; the agent copies text while locked and never frees that handle.
  See [Microsoft's clipboard ownership contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboarddata).
- Failed access attempts use one-shot-style Win32 timers: 20, 40, 80, 160, 320,
  and 640 ms (seven total attempts). No retry sleeps block the message loop.
  A new clipboard sequence cancels the previous retry and starts a new observation.
  Repeated notifications for a known pending sequence do not reset the retry budget.
  After exhaustion, state is `Unavailable(RetriesExhausted)` until another notification.
  Stale queued timer messages cannot run a retry before its current deadline.
- Individual Win32 calls are synchronous. In particular, Windows may ask the
  clipboard owner to render delayed data inside `GetClipboardData`; retry timers
  do not impose a timeout on that OS call. This watcher tracks the latest value,
  not a guaranteed history of every rapid intermediate copy.
- Source text is capped at 1 MiB of UTF-16 excluding NUL, and stored text at
  1 MiB of UTF-8. Reads inspect at most 524,289 UTF-16 code units, even if the
  clipboard allocation is larger. Unterminated, malformed, or oversized text is
  skipped without logging its contents. Invalid UTF-16 is rejected.
- Consecutive successful identical text reads suppress duplicate update logs.
  The private last-successful-text baseline survives temporary access failures,
  pending reads, unknown sequences, and sequence-change restarts. Recovering the
  same text returns `Update::Duplicate`, suppressing the successful `clipboard updated`
  log even after multiple retries. Snapshot state, timestamps, versions, and sequence
  metadata still update; deduplication suppresses logging, not state publication.
  Genuine text changes return `Update::Changed` and replace the baseline.
  Clearing, unsupported formats, malformed/oversized data, and exhausted retries
  clear the baseline; copying the former text afterward counts as a new update.
  The baseline is never a readable fallback.
- `src/clipboard.rs` owns a private `Arc<RwLock<Inner>>`. Consumers clone the
  store handle and call `snapshot()` for an owned snapshot. `begin_observation()`
  immediately removes readable text, and `complete()` validates all outcomes.
  Store-issued tokens prevent superseded operations from publishing *or invalidating*
  newer observations, including when Windows sequence numbers wrap.
- No clipboard contents are persisted. The store privately retains at most one
  bounded text value for comparison, separately from the readable state. Neither
  `snapshot()` nor observation tokens expose this baseline; superseded operations
  cannot modify it. Previously returned snapshots are independent copies, so
  future consumers must obtain a fresh snapshot per request, not cache old text.
- Ctrl+C/Ctrl+Break posts a shutdown message from the console callback. The watcher
  thread cancels the timer, removes the listener, destroys the window, unregisters
  its class, and removes the console handler. Forced process termination cannot
  run Rust cleanup.

## State and version semantics

| State | Meaning | Readable text |
| --- | --- | --- |
| `Empty` | Clipboard opened successfully and has zero formats | None |
| `Available` | Valid, bounded `CF_UNICODETEXT`, including a zero-length string | Current observed text |
| `Image` | Images explicitly enabled and a validated, bounded bitmap/PNG observed | Current PNG through the separate image API; never returned by the text endpoint |
| `Unavailable(reason)` | Not read yet, pending, failed, unsupported, malformed, oversized, unknown sequence, changed during read, or retries exhausted | None |

A pending read and the first temporary access failure invalidate readable text
immediately. They never leave the previous text available while waiting for retries.
The private comparison baseline can remain in memory during uncertainty, including
while snapshots are `Unavailable`; it is not part of the snapshot or any future
network response. Retry exhaustion discards it as a conservative terminal policy.
An empty Unicode string is `Available` with zero characters; a clipboard with no
formats is `Empty`. Image-only clipboards are `Unavailable` while images are
disabled, or `Image` after successful enabled conversion. Unsupported formats
(including file-only clipboards) remain `Unavailable`.

Every snapshot includes `updated_at`, a monotonically increasing local `version`,
the observed Windows `sequence` (or `None` for zero/unknown), `origin`, and an
`operation_id`. The operation ID combines a process session (start time and PID)
with a local observation counter. One observation keeps its operation ID across
retries; each published state transition advances the snapshot version. IDs are
process-session scoped design hooks, not durable device IDs or a sync wire protocol.
Local observations use `WindowsClipboard` origin; accepted API writes use `Iphone`.

Windows sequence numbers are compared for equality, never numerical ordering.
They are checked before reading, while the clipboard is still open after reading,
and after closing it. If they change, the result is discarded and a fresh
observation is scheduled. Delayed rendering can advance the sequence during a
read. Zero is treated conservatively as unknown, with no text published.
See [Microsoft's sequence-number contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber).

Snapshots describe the last processed observation, not a transaction with the OS.
A change after the final sequence check is invalidated by its queued notification;
there is necessarily a notification-delivery interval. Future serving code must
respect `Unavailable`; the HTTP API also checks sequence freshness before serving.

## Verification and exact manual acceptance test

```powershell
cargo fmt --check
cargo check
cargo test
```

Mission 004A results: **38 passed, 0 failed**. Coverage includes validated HTTPS
health and bearer GET/POST, real TCP peer/IP preservation, missing bearer,
untrusted/wrong-identity/expired certificates, malformed TLS and old plaintext
requests, missing/malformed PEM and mismatched keys, and bounded shutdown with
an incomplete TLS handshake. Test CA/leaf keys exist only in memory. Existing
clipboard/auth/queue/dedup/Unicode/origin tests continue to pass. TLS router POST
tests use a test command consumer; native behavior is tested separately below.

The Windows provisioning script's new issuance and same-CA renewal were tested,
including Windows chain validation. The native integration passed **over HTTPS**:
Unicode + LF/CRLF readback, authenticated writes/GET, repeated operation IDs,
500 ms contention recovery and 1.8 s contention exhaustion without modifying old
text. Remote writes emitted no local-change notifications; a later deliberate
local copy did. Windows also accepted validated HTTPS health and rejected the
uninstalled CA and wrong server identity. No OS roots/firewall rules were changed.
Physical iPhone HTTPS Safari/Shortcut tests have **not** been performed by the agent.

After completing certificate provisioning and starting exactly one HTTPS agent:

```powershell
$runtimeTls = Join-Path $env:LOCALAPPDATA 'WiniOS\tls'
.\scripts\Test-ClipboardPost.ps1 -BaseUri https://192.168.1.7:8765 -CaCertificate (Join-Path $runtimeTls 'winios-ca.cer')
```

The script requires PowerShell 7.4+, the same Windows credential user, and opt-in
POST. It intentionally overwrites the Windows clipboard with disposable test text
and starts no additional agent. `-CaCertificate` configures a **per-client custom
root trust store**, not a certificate-validation callback: Windows still validates
chain/signature/date/EKU/constraints and URL IP. It installs no root or trust bypass.
If omitted, ordinary Windows OS trust is used. Redirects and proxies are disabled.
The script prints only status, never credentials or clipboard bodies.

Exact device acceptance sequence:

1. Provision for `192.168.1.7`, record the public CA fingerprint, and start HTTPS
   with `--allow-clipboard-write`. Do not keep a legacy HTTP listener running.
2. Review/install only the public CA on iPhone and enable TLS root trust as above.
   Safari must load `https://192.168.1.7:8765/v1/health` with no warning.
3. Update both Shortcut URLs to HTTPS; keep the existing bearer/JSON/actions.
   Copy disposable Unicode/multiline text on Windows, run GET, and paste in Notes.
   Copy different disposable text on iPhone, run POST, and Ctrl+V into Notepad.
   Verify exact characters/newlines and repeat both directions.
4. Remove/alter the bearer header: requests must still get 401 and POST must not
   write. Restore the existing token. Disabling the POST flag must yield 403.
5. Disable/remove the test CA trust on the phone: HTTPS must fail, not show an old
   cached success. Restore trust only after reviewing the correct root again.
   Wrong-IP and expired certificate rejection are automated TLS tests; never
   accept a warning in Safari/Shortcuts to force them through.
6. An old `http://192.168.1.7:8765/v1/clipboard` request to the **secure** listener
   must fail at the TLS boundary, not return clipboard text or redirect to HTTP.
   Afterward valid HTTPS must still work. Do not include a real bearer in this
   intentional plaintext test; the Rust test already covers that protocol boundary.
7. Run the native integration script above. Avoid concurrent manual copies while
   it checks exact bytes. Confirm remote writes produce no local update logs and
   the final deliberate local copy produces one.
8. Press Ctrl+C: watcher and API must stop, queued requests must fail/finish safely,
   and the HTTPS URL must stop responding. Return the CA key offline. Remove
   development root trust from devices when no longer needed.

Remaining platform constraints: actual iOS trust UI, root-name-constraint behavior,
Shortcuts permissions and physical paste behavior need device verification. No
pairing, mutual TLS, automated device enrollment or revocation infrastructure is
part of Stage A.
## Manual Windows test

1. Copy disposable text **before** running `cargo run`. Confirm it is observed
   immediately, without a new copy, and the program stays running without a visible
   window. Repeat while copying rapidly during startup; the final copy must win.
2. In Notepad, copy `Mission 001`. Expect one timestamped update with 11
   characters and no content in the log. Repeat the same copy: expect no successful
   `clipboard updated` log, even if `TemporaryFailure` appears before recovery.
   Temporary-unavailability logs are still allowed.
3. Copy different text from VS Code, then copy `Mission 001` again. Expect one
   update for each change. Try Unicode text such as `A😀é` (3 characters).
4. With images disabled, copy an image from Paint or a file from Explorer. Expect
   `Unavailable(UnsupportedFormat)` and no text in a fresh store snapshot. With
   images enabled, a supported image instead enters `Image`; the text API still
   returns no stale text. See Mission 008 for the separate image endpoint.
   Clear the clipboard using a native helper's `OpenClipboard` → `EmptyClipboard`
   → `CloseClipboard`; expect `Empty`. Copy the earlier text again; expect a new
   update. A helper publishing a valid zero-length Unicode string should instead
   yield `Available` with zero characters.
5. In a second PowerShell terminal, run `Set-Clipboard -Value ('a' * 524288)`.
   Expect an update at the UTF-16 limit. Repeat with `524289`: expect
   `Unavailable(Oversized)` and no readable text. Then copy a short text to recover.
6. To check contention, use a small native helper (or PowerShell `Add-Type`
   P/Invoke) to `OpenClipboard`, `EmptyClipboard`, and `SetClipboardData` with
   test Unicode text, then keep the clipboard open after publishing. Closing
   it within about a second should allow a retry to succeed. During contention
   expect `Unavailable(TemporaryFailure)` immediately, with no readable previous text.
   First retry this with the same previously accepted text: recovery within the
   retry window must not emit another successful `clipboard updated` log. Then
   repeat with different text: expect one successful update after recovery.
   Holding it beyond the retry window should produce `Unavailable(RetriesExhausted)`;
   a subsequent copy must still work. Replace the held test data with a different
   value before releasing it: confirm the latest sequence/text wins and no old
   retry restores the earlier value. Press Ctrl+C during retries to check responsiveness.
   **This modifies the clipboard; use disposable test data.**
7. Press Ctrl+C in the watcher terminal. Confirm the stopped message, prompt
   return, and successful restart. Repeat with Ctrl+Break if available.
8. Optionally run read-only mode with `--debug-clipboard` and confirm that only
   metadata/access errors are logged, never clipboard contents.
9. With a native test helper, publish unterminated/invalid UTF-16 test data.
   Expect `MalformedData` and no readable text. Then copy a valid value to recover.

The CLI reports state transitions and metadata. The authenticated HTTP endpoint
tests readable state without exposing internal baseline/operation metadata.
Source unit tests exercise the complete snapshot API directly.

Notepad/VS Code copy delivery, real ownership contention, and interactive
terminal Ctrl+C require validation on a Windows desktop.
