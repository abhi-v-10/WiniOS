# WiniOS agent — Mission 002

A Windows clipboard watcher using Win32, with a minimal Axum/Tokio HTTP API for
Windows → iPhone clipboard transfer. No clipboard writer, frontend, cloud service,
or periodic clipboard polling is included.

**Bearer-authenticated HTTP is not encrypted.** This prototype is only for
disposable, non-sensitive text on a trusted LAN. Anyone able to inspect the HTTP
traffic can capture the text and bearer token. HTTPS or another authenticated,
encrypted transport is mandatory before broader everyday use. Never port-forward
this service, publish it through a tunnel/proxy, or allow it on Public networks.

## Run

Requires Windows and a Rust toolchain with the Windows SDK/linker installed.
Install Rust with the MSVC toolchain and Visual Studio Build Tools with the
Desktop development with C++ workload/Windows SDK if they are not already present.
Stop an older watcher before rebuilding: Windows locks a running executable.

```powershell
cd C:\WiniOS
cargo run
```

This starts one agent on `http://127.0.0.1:8765`. A named Windows mutex prevents
another service instance in the same Windows session. The Win32 watcher runs on
one dedicated OS thread; Tokio's main thread runs HTTP. Both share the private-lock
`ClipboardStore`. Ctrl+C/Ctrl+Break stops the watcher, invalidates readable state,
and gracefully drains HTTP for up to five seconds. Server failure requests watcher
shutdown; the watcher thread is joined before normal application exit.

For iPhone access, use `ipconfig` to find your Wi-Fi adapter's private IPv4 address,
then explicitly bind that address (replace the example with your actual address):

```powershell
cargo run -- --lan 192.168.1.42 --port 8765
```

`--lan` requires a specific RFC1918 IPv4 address assigned to this PC; wildcard,
public, IPv6, and link-local binds are rejected. The selected IP/port and URL are
printed at startup. Only loopback/private IPv4 peers are accepted. This does not
replace firewall or router controls; do not forward or proxy the port.

## Token setup

On the first service start, Windows `BCryptGenRandom` generates 32 random bytes
(256 bits), encoded as a 64-character lowercase hex token. The token is saved as
a per-user Generic Credential named `WiniOS/Agent/Bearer/v1` in Windows Credential
Manager, persisting for that user on this computer. It is not stored in the repo,
an ordinary file, a URL, or normal application logs. Token validation uses a
constant-time comparison for equal-length values. See Microsoft's
[credential storage contract](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw).

To explicitly reveal it in your terminal after the first service start:

```powershell
.\target\debug\winios-agent.exe --show-token
```

This command reads the credential and exits; it does **not** start another
watcher or HTTP listener. Its output is intentionally secret: avoid recording,
redirecting, committing, or sharing it. Type it into your iPhone Shortcut's header.
Other processes running as your Windows user can access the credential; it is not
protection from a compromised PC. The Shortcut also contains the secret; do not
share it or treat it as safe for sensitive content.

To rotate the token, stop the agent, remove that Generic Credential through
Control Panel → Credential Manager → Windows Credentials, and restart. Update
the Shortcut with the newly generated token. Never remove it while the agent is
running: the server keeps its startup token in memory until shutdown.

## HTTP endpoints

| Request/outcome | Status | Body |
| --- | --- | --- |
| `GET /v1/health` (no token) | 200 | Fixed non-sensitive JSON liveness status |
| `GET /v1/clipboard`, missing/invalid bearer | 401 | Empty; `WWW-Authenticate: Bearer` |
| Available, current known sequence | 200 | Exact text, `text/plain; charset=utf-8`, at most 1 MiB |
| Empty clipboard (no formats) | 204 | Empty |
| Stale snapshot/changed store revision | 409 | Empty; retry after watcher catches up |
| Pending, inaccessible, unsupported, malformed, stopped, or unknown sequence | 503 | Empty; no previous text fallback |
| Oversized text rejected | 413 | Empty |
| Non-private/non-loopback peer | 403 | Empty |

Only clipboard requests require `Authorization: Bearer <token>`. Duplicate
Authorization headers and malformed credentials are rejected. Every response has
`Cache-Control: no-store` and `X-Content-Type-Options: nosniff`. There is no CORS
permission, request/body/header access logging, or token in query parameters.
Health is liveness only; it does not claim clipboard availability.

Every clipboard request obtains a fresh owned snapshot. After constructing the
response, the handler checks the store revision and calls Windows
`GetClipboardSequenceNumber` immediately before handing the response to Axum.
Unknown/zero or mismatched sequence numbers refuse content. The private dedup
baseline is never part of a response. A clipboard change *after* this final check
cannot retract bytes already handed to the network; the API is a point-in-time
transfer, not a lock held through transmission.

## Apple Shortcuts setup

1. Connect the iPhone and PC to the same trusted Wi-Fi. Start the agent with
   `--lan <PC-Wi-Fi-IPv4>`; `127.0.0.1` on the iPhone refers to the phone itself.
2. In Shortcuts, create a shortcut named **Get Windows Clipboard**. Add a **URL**
   action with `http://192.168.1.42:8765/v1/clipboard` using the printed PC address.
3. Add **Get Contents of URL**, choose **GET**, expand its options, and add a
   header: key **Authorization**, value **Bearer ** followed by the exact token
   (one space; no angle brackets or quotes). Keep the token out of the URL.
4. Add **If** the request result **has any value**, then **Copy to Clipboard**
   using the result. In **Otherwise**, show a notification such as “Windows
   clipboard is empty or unavailable.” This avoids clearing the phone clipboard
   for empty responses. A valid empty Unicode string also leaves the phone
   clipboard unchanged with this simple Shortcut. Do not copy HTTP error text.
5. Allow local-network access if iOS asks. Run the shortcut after copying
   disposable text on Windows, then paste into Notes to verify it.

Action labels/options can vary by iOS version. Transport/authentication errors may
stop the Shortcut and show an error; do not continue copying a previous action's
result. Apple documents the GET action in
[Request your first API](https://support.apple.com/guide/shortcuts/request-your-first-api-apd58d46713f/ios).

## Windows Firewall and connectivity

First test locally: `curl.exe -i http://127.0.0.1:8765/v1/health` for default mode,
or use the selected LAN address in LAN mode. A successful health response proves
connectivity, not authorization. An unauthenticated clipboard request must get 401.

On a trusted home network, use the **Private** Windows network profile and allow
the actual built executable on **Private networks only** if Windows prompts.
Never disable the firewall. If a manual rule is needed, run this yourself in an
Administrator PowerShell after substituting your address and executable path:

```powershell
New-NetFirewallRule -DisplayName 'WiniOS prototype' -Direction Inbound -Action Allow -Protocol TCP -LocalPort 8765 -LocalAddress 192.168.1.42 -RemoteAddress LocalSubnet -Profile Private -Program (Resolve-Path .\target\debug\winios-agent.exe).Path
```

No firewall configuration is changed automatically. To remove your test rule:
`Remove-NetFirewallRule -DisplayName 'WiniOS prototype'`. If using `--release`,
update the rule to the release executable path. Review/remove broader pre-existing
rules yourself; the prototype cannot override router/firewall policy.

For timeouts, check the printed address, port, PC sleep state, Private profile,
program path in the firewall rule, and iOS local-network permission. Guest Wi-Fi,
AP/client isolation, VPNs, and different subnets can prevent peer access. A 401
means connectivity works but the token/header is wrong; 409/503 usually means
retry after copying valid text and waiting briefly. DHCP address changes require
restarting with the new address and updating the Shortcut. A bind error means
the selected address is not assigned locally or the port is occupied.

Keep the terminal running. Initial and new Unicode text produce a local timestamp
and a Unicode scalar-value count (an emoji sequence can contain multiple characters).
Clipboard contents are never printed by default. Stop with **Ctrl+C**; expect
`WiniOS clipboard watcher stopped.` and the terminal prompt to return.

Explicit, sensitive-content debugging:

```powershell
cargo run -- --debug-clipboard
```

Debug output quotes/escapes text, including terminal control characters. Do not
enable this while copying secrets.

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
Only `WindowsClipboard` origin exists; no Windows clipboard writer is implemented.

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

## Automated checks

```powershell
cargo fmt --check
cargo check
cargo test
```

Unit tests cover consecutive deduplication, shared latest state, empty text,
Unicode counting, UTF-16 validity/termination, exact UTF-16 size boundaries,
UTF-8 expansion limits, explicit invalidation after rejection/failure, startup state,
version metadata, and rejection of superseded retry results. Regression tests cover
identical text after one, three, and six temporary failures; repeated observation
restarts; changed text after retries; and clearing followed by the same text.
HTTP tests cover authentication failures/success, exact UTF-8 and 1 MiB boundaries,
empty/unavailable states, retained-baseline non-disclosure, stale/unknown sequence
rejection, health, and public-peer refusal. Tests use disposable in-memory text
and injected sequence values; they do not start another Windows watcher or write
to the real clipboard. The native RNG test does not persist test credentials.

## Manual end-to-end test

1. Ensure older agents are stopped, then run the single agent with `--lan` and
   configure the token/header and firewall as above. Check health from iPhone
   Safari using the printed address; its response must contain no secrets.
2. Copy `WiniOS Mission 002 😀` from Notepad or VS Code. Run the Shortcut, paste
   into Notes, and verify exact Unicode and multiline text. Copy the same text
   repeatedly and verify transfer still works without duplicate successful logs.
3. Remove/change the Shortcut token: it must refuse the request and must not
   replace the phone clipboard with an old Windows value. Restore the header.
4. Clear/replace Windows clipboard contents with an image or invalid/oversized
   test data. Verify the status codes above and that old text is never returned.
   Exercise contention and rapid copies: 409/503 is safe; retry once settled.
5. Press Ctrl+C in the PC terminal. Both watcher/server stopped messages should
   appear, the process should exit, and the old URL should stop responding.

Physical iPhone execution, Wi-Fi routing, firewall prompts, and iOS permissions
require manual validation; automated handler tests cannot establish those.

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
8. Optionally run with `--debug-clipboard` using disposable text and confirm
   content is printed only in this mode.
9. With a native test helper, publish unterminated/invalid UTF-16 test data.
   Expect `MalformedData` and no readable text. Then copy a valid value to recover.

The CLI reports state transitions and metadata. The authenticated HTTP endpoint
tests readable state without exposing internal baseline/operation metadata.
Source unit tests exercise the complete snapshot API directly.

Notepad/VS Code copy delivery, real ownership contention, and interactive
terminal Ctrl+C require validation on a Windows desktop.
