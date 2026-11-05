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

## 4. Existing Shortcuts: change URLs only

After root installation and trust, change the URL in both existing Shortcuts to
**https://192.168.1.7:8765/v1/clipboard**. Keep the same bearer header, actions,
JSON schema and result handling. No TLS bypass/custom validation action is needed.

- GET: **Get Contents of URL → GET**, header `Authorization: Bearer <token>`;
  if the result has a value, **Copy to Clipboard**. Empty/error responses must not
  copy an old action result. Valid zero-length text still yields 200 with no bytes.
- POST: **Get Clipboard → Get Contents of URL → POST**, same Authorization header,
  **Request Body: JSON**, Text field `text` using the Get Clipboard magic variable.
  Let Shortcuts encode the JSON. Only show success when response dictionary `status`
  equals `ok`. HTTP errors/transport failures must not show success.
- Allow iOS local-network permission when prompted. No additional iPhone app is
  necessary. The bearer token remains the initial application authentication, not
  device pairing or mutual TLS. Do not share Shortcuts containing that secret.

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
| `Unavailable(reason)` | Not read yet, pending, failed, unsupported, malformed, oversized, unknown sequence, changed during read, or retries exhausted | None |

A pending read and the first temporary access failure invalidate readable text
immediately. They never leave the previous text available while waiting for retries.
The private comparison baseline can remain in memory during uncertainty, including
while snapshots are `Unavailable`; it is not part of the snapshot or any future
network response. Retry exhaustion discards it as a conservative terminal policy.
An empty Unicode string is `Available` with zero characters; a clipboard with no
formats is `Empty`. Unsupported image/file-only clipboards are `Unavailable`.

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
4. Copy an image from Paint or a file from Explorer. Expect
   `Unavailable(UnsupportedFormat)` and no text in a fresh store snapshot.
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
