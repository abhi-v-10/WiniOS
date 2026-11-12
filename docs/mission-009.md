# Mission 009 — stability and security hardening

Targeted audit on 2026-11-10. Existing text/image APIs, settings, Shortcuts,
credentials, certificate identities, Tailscale Serve and startup preferences are
preserved. No new application feature or UI layout. About adds only the requested
developer/support footer, with existing spacing shortened enough to fit the normal
window: **Developed by Abhishyanth**, **Help or questions:
v.abhishyanth118@gmail.com**. The native render and footer text assertion passed.

## Confirmed findings and fixes

1. **Readiness loss did not cancel an admitted POST.** The old middleware checked
   Tailscale readiness only before the handler. A queued upload could survive a
   disconnect/reconnect and still write. A failing deterministic test reproduced
   this. Readiness now carries a generation and expiry signal. HTTP handlers and
   queued native commands share the original admission guard. Detected loss,
   expiry or pause invalidates that operation; reconnection requires a new request.
   Routine successful readiness refresh does not cancel healthy requests.
2. **Pause depended on async task scheduling to cancel native work.** Previously
   a queued command remained eligible until the HTTP task processed Pause and
   dropped its future. A test paused/resumed without yielding and observed the
   command still eligible. Native expiry/commit checks now inspect the same
   sharing/readiness generations directly, before allocation/commit attempts.
3. **Backpressured downloads retained an old image and capacity until timeout.**
   The old producer checked Pause only between sends, so a stalled send could
   retain the image/permit for ten seconds. The regression reproduced this.
   Clipboard revisions and admission changes now wake the producer. The body
   also checks revision, native sequence, permission and deadline before yielding
   a queued chunk. Invalidated bodies abort rather than deliver queued stale data.
   The revision receiver retains only a number, not the clipboard store/image.
4. **Cancelled DIB work allocated before checking its budget.** Native decode/
   output preparation could allocate the full bounded output before noticing an
   already-cancelled/expired operation. Both now check before allocating.
5. **The old opt-in privacy fixture assumed immediate OS clipboard access.** An
   initial live run failed its immediate read assertion. Test fixtures now retry
   only temporary access/change errors, within the existing bounded delays, and
   report error metadata. A readable-text/privacy mismatch still fails immediately.
   PowerShell acquisition helpers now also tolerate brief contention. A
   shipping-binary run exposed a timing-sensitive fixed-duration exhaustion
   assertion; exhaustion fixtures now hold the lock until the response (with a
   bounded fail-safe), check that it remained held and permit only documented
   503 contention / 504 acknowledgement-timeout responses. Native readback still
   verifies that a rejected text write changed nothing. Production clipboard
   retry behavior was not changed.

No ownership, stride-overflow or authentication-bypass defect was found in the
reviewed native paths. Those paths retain their existing behavior: bounded copy
under OpenClipboard, scoped GlobalLock/CloseClipboard/DC cleanup, preparation
before EmptyClipboard, and relinquishing each accepted handle to Windows exactly
once. Failed/partial writes invalidate readable state. GDI bitmap handles are
never GlobalLocked or deleted by the reader. This is a targeted code review and
regression suite, not a formal proof or exhaustive fuzzing campaign.

## Safety boundaries retained

- PNG input/output cap 2/4/8 MiB, side cap 8192, area cap 8,388,608 pixels, RGBA cap
  32 MiB; strict PNG framing/CRC/type validation before decode; no animation,
  metadata inflation, external profiles, image files or clipboard-content logs.
- One upload/decode/write lease, two image download producers, bounded four-entry
  native command queue. Existing body, codec, acknowledgement and stream deadlines.
- One text/image readable state; no private comparison baseline in API snapshots.
  New observations invalidate prior content; stale conversion/sequence results
  cannot publish over newer text/images, including sequence wraparound.
- 401 remains the result for unauthorized requests, even when paused/disconnected.
  Explicit image/write permissions remain required. Pause and detected readiness
  loss return 503 for authenticated clipboard operations. Rejected/aborted streams
  cannot become successful complete PNG responses.
- Backend remains exclusively 127.0.0.1 behind the existing private HTTPS Serve
  identity. Actual TCP peer checks ignore proxy identity headers. Bearer auth and
  validated HTTPS remain mandatory externally; no TLS bypass/downgrade/Funnel,
  firewall, routing or certificate changes. Other same-host processes are still
  within the loopback trust boundary; loopback is not process authentication.

## Automated and native verification

`cargo fmt --check`, `cargo check --offline`, `cargo test --offline`, and
`cargo build --release --offline` passed: **112 passed, 0 failed, 4 ignored**.
Three opt-in tests were also explicitly run and passed: native image startup /
format / origin / stale conversion, private setup-token isolation across owner
restart, and the live read-only HTTPS setup probe. The remaining ignored test is
an interactive preview, not an acceptance claim.

New/extended deterministic coverage includes:

- 256 interleaved text/image observations, superseded completion/rejection/clear,
  and native sequence wraparound;
- malformed bounded PNG/DIB corpus, valid-CRC dimension/area bombs, expired and
  cancelled codec work, existing truncation/CRC/APNG/byte-limit checks;
- detected network loss plus immediate reconnect, normal freshness refresh,
  simulated long sleep/expired readiness, and requiring fresh requests;
- pause/resume without rescheduling the HTTP handler, queued native expiry,
  backpressured/abandoned image streams, queued-chunk rejection and capacity return;
- active text plus queued text/image requests and image-lease release on Quit;
- deterministic native retry-budget exhaustion without sleeping or opening the
  clipboard. Existing timed body/ack tests and actual contention remain covered.

`Test-ClipboardImages.ps1` and its existing `Test-ClipboardPost.ps1` regression
passed through the real configured Tailscale HTTPS listener with ordinary trust
verification: exact disposable pixels, registered/padded PNG, DIBV5/DIB24/
compatible bitmap capture, upload acknowledgement/auth/limits, contention and
exhaustion, clearing/malformed invalidation, Unicode and local-copy recovery.
Each validation used a single candidate agent with isolated secret-free image
preferences, then restored the original agent. Protected personal file hashes,
credential fingerprint, startup value and Serve digest were unchanged.

The static-CRT per-user installer and explicit eleven-file allowlist checks passed.
The safe upgrade was installed at the existing `%LOCALAPPDATA%\Programs\WiniOS`
path. The **installed shipping binary** also passed the same native text/image
HTTPS script, live setup probe, desktop/Start Menu/GUI-subsystem checks and normal
tailnet HTTPS authentication checks. One normal tray instance was restored, bound
only to 127.0.0.1:8766. Protected settings/token/TLS/startup/Serve were unchanged.

## Measurements

Pre-update installed agent, 30.41-second idle sample, 16 logical processors:
**0.00 ms detected CPU**, mean **49.57 MiB working set**, **32.88 MiB private
commit**. Zero is the observed counter value/resolution, not a promise of no work.

Final installed tray, restored normal preferences, fresh 30.40-second idle sample:
**0.00 ms detected CPU**, **14.97 MiB mean working set**, **2.64 MiB mean private
commit**. Desktop/window/cache state was not controlled between the old and new
samples; do not interpret their difference as a measured hardening speedup.
Raw counters are `target/mission009-idle-before.json` and `-after.json`.

Candidate release benchmark: five POST/GET transfers per synthetic fixture,
through the existing validated Tailscale HTTPS Serve on the **same Windows host**.
POST measures through native acknowledgement. GET includes receiving the PNG.
Every round trip compared decoded disposable pixels exactly. Nearest-rank p95
with five samples is the largest sample; these are indicative measurements.

| Fixture | POST median / p95 | GET median / p95 | Returned PNG |
| --- | --- | --- | --- |
| 1920×1080 pattern | 33.78 / 53.81 ms | 2.01 / 14.38 ms | 50,491 bytes |
| 1920×1080 noise | 109.36 / 125.57 ms | 20.22 / 32.77 ms | 8,296,178 bytes |
| 3840×2160 pattern | 136.19 / 152.22 ms | 2.07 / 3.94 ms | 197,388 bytes |

Agent sampled peak: **171.07 MiB private**, **176.44 MiB working set**, sampled
every 5 ms. GDI handles **7 before / 7 after**. This excludes the benchmark client,
Tailscale process, OS clipboard/kernel memory, and possible peaks between samples.
The 32 MiB RGBA cap is not a total-process cap: source, codec, prepared native
buffers and Windows-owned allocations coexist during transfer. No unbounded
growth or GDI leak was observed in this workload; it is not a long-duration soak.
These are not Wi-Fi, DERP, cellular or physical iPhone latency measurements.

A repeat against the **installed static-CRT binary**, after fixture stabilization:

| Fixture | POST median / p95 | GET median / p95 |
| --- | --- | --- |
| 1920×1080 pattern | 42.74 / 56.79 ms | 2.22 / 9.02 ms |
| 1920×1080 noise | 118.19 / 121.65 ms | 25.89 / 40.15 ms |
| 3840×2160 pattern | 144.68 / 150.24 ms | 2.34 / 3.49 ms |

Sampled shipping-agent peak: **171.57 MiB private**, **172.98 MiB working set**;
after replacing the image with disposable text and waiting 500 ms: **4.16 MiB
private**, **16.51 MiB working set**. GDI handles remained **7 → 7**. All fixture
pixels matched. Raw metadata is in `target/mission009-installed-benchmark.json`;
the first candidate run is `target/mission009-benchmark.json`.

Reproduce with a ready, image/write-enabled disposable test configuration (8 MiB):

```powershell
cd C:\WiniOS
.\scripts\Test-ClipboardImages.ps1
.\scripts\Measure-Clipboard.ps1 -Samples 5
```

The benchmark writes generated images to the system clipboard and replaces them
with disposable text afterward. No image files or pixel hashes are saved. Its JSON
report contains only dimensions, sizes, timings, counters and process metadata.
Do not run it with important clipboard contents. It does not change app settings
or Tailscale; do not enable images/writes silently for production benchmarking.

## Remaining limits and manual checks

Real sleep/resume, VPN switching, network loss/roaming and physical iPhone
Shortcuts were **not** exercised by fault-injection tests. The simulated tests
verify WiniOS invalidation/recovery logic, not Windows firmware or router behavior.
Readiness checks remain bounded/periodic (30 seconds while connected, 5 otherwise,
45-second freshness). Cancellation follows detected loss or expiration; it is not
an instantaneous network-disconnect notification. HTTPS still validates normally.

A Win32 write already past its final cancellation check can finish, and socket
bytes already handed off cannot be recalled. A lost response is not proof that
Windows did not accept a write. WiniOS does not undo a clipboard change, since
doing so could overwrite a newer local copy. Inspect before retrying. Wide-gamut/
HDR color management and the earlier image-format limitations are unchanged.

After installation, check About at normal/high DPI and keyboard navigation; verify
the footer is readable. Retest existing text/image Shortcuts, Pause/Resume, clear
then receive, and Quit/relaunch. Optionally sleep longer than 45 seconds with
disposable content, resume and wait for verified Connected; compare fresh content.
Manually disconnect/reconnect Tailscale and confirm authenticated clipboard
operations fail closed until ready; missing bearer must remain 401. Do not bypass
TLS verification or change Serve/firewall/trust to make a test pass.

## Rollback

The verified pre-update package/binaries are backed up at
`C:\WiniOS\target\mission009-backup-20261110-103433`.
The old installer and actual installed binary hashes matched its manifest.
Quit WiniOS, run that folder's `WiniOS-Setup.exe`, and launch once. Do not remove
settings, Credential Manager token, TLS files, or the user-owned Serve route.
No new preference/schema migration was introduced, so the old app can read the
same personal settings. Current installer output is `target\dist\WiniOS-Setup.exe`.

Changed application files: `clipboard.rs`, `http.rs`, `image.rs`, `tailnet.rs`,
`writes.rs`, `watcher.rs` (test-only fixture/retry/shutdown coverage),
`image_tests.rs`, and `preferences_window.rs` (About copy/test only). Supporting
changes: benchmark/native integration fixture scripts, this guide/README, explicit installer documentation
allowlist and its verification count. No dependency additions/updates.
