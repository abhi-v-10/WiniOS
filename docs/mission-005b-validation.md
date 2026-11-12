# Mission 005B validation — 2026-11-06

## Automated and laptop-side results

- `cargo fmt --check`: passed.
- `cargo check --offline`: passed.
- `cargo test --offline`: **55 passed, 0 failed**; includes retained clipboard,
  retry/deduplication, auth, write acknowledgement/cancellation, TLS trust/identity,
  malformed/plaintext TLS and graceful shutdown tests.
- `cargo build --release --offline`: passed.
- `scripts/Test-DesktopBinary.ps1`: passed; GUI subsystem for `winios.exe`, console
  subsystem for `winios-agent.exe`.
- New tests cover dedicated loopback mode/settings, legacy flag rejection, tailnet
  status/certificate-domain checks, exact Serve profile and Funnel rejection,
  gate expiry/recovery, mandatory authentication, spoofed proxy headers, real TCP
  peer rejection, tray states, and bounded subprocess output/time/failure handling.
- Before Serve setup, a real tray instance bound only `127.0.0.1:8766`; health was
  503 and unauthenticated clipboard GET was 401. No LAN listener was created.
- Tailscale 1.104.5 was already installed, authenticated and connected. MagicDNS
  and the node's certificate domain were enabled; initial Serve config was empty.
- The exact private HTTPS Serve route to `http://127.0.0.1:8766` was enabled after
  explicit approval. No Funnel, firewall, adapter, public listener or trust change
  was made by this implementation/setup.
- Real `Test-Tailnet.ps1 -NativeClipboard` passed through Tailscale Serve, both
  before installation and from the installed standalone tray application. This
  verifies normal Windows HTTPS trust/hostname validation, health, GET/POST bearer
  enforcement/no-store, native Unicode and LF/CRLF readback, repeated operation
  tracking, temporary contention recovery, retry exhaustion and local-copy GET
  recovery. It deliberately overwrote the clipboard with disposable text only.
- The first HTTPS probe timed out; subsequent normal verified HTTPS health and
  complete native tests passed. No TLS bypass/downgrade was used.
- `winios.exe --quit` (same graceful close path as the menu) exited the agent,
  closed its backend listener and permitted relaunch. User-managed Serve config
  remained exactly unchanged and returned HTTP 502 while the backend was closed.
  This does not claim an interactive click on the tray Quit menu was observed.
- Plaintext port-80 requests could not complete; plaintext to TLS port 443 returned
  HTTP 400. No real bearer token was sent in these probes.
- Installed executable matched the final release executable after relaunch.
- A second console launch was rejected before starting another watcher/server;
  exactly one GUI agent and one `127.0.0.1:8766` listener remained. A five-second
  idle sample recorded 0 ms agent CPU and 14.6 MiB working set. This short sample
  is observational, not a full performance benchmark or aggregate Tailscale cost.

## Installed setup

Standalone GUI: `%LOCALAPPDATA%\Programs\WiniOS\winios.exe`.
Console CLI is beside it. Saved settings select `tailnet-backend`, port 8766,
and explicit `allow_clipboard_write: true`. No autostart was created.

The discovered HTTPS hostname is displayed by the tray and by the read-only
integration script; no machine name or overlay IP is hardcoded into the agent.
The existing LAN certificate, key and public CA files were left unchanged; file
size/timestamp metadata was checked around installation.

Local rollback backups from this installation:

- `%LOCALAPPDATA%\WiniOS\settings.direct-lan.20261106-101125.backup.json`
- `%LOCALAPPDATA%\Programs\WiniOS\backup-20261106-101125\`

**The approved background Serve route persists after Quit.** Disable that exact
route before uninstalling or reusing port 8766; see the
[setup and rollback guide](portable-connectivity.md#serve-lifetime-removal-and-rollback).

## Physical iPhone validation and remaining checks

The user physically verified the new tailnet HTTPS URL on the iPhone: Safari
loaded health without a certificate warning, and both GET/POST Shortcuts copied
and pasted successfully in both directions. This establishes actual device use
of the new endpoint; the specific roaming/network transitions below were not
claimed by that confirmation.

Still unverified:

- Ethernet/Wi-Fi transitions, another router/mobile hotspot, iPhone cellular,
  PC sleep/resume, VPN disable/reconnect, DNS outage or loss of internet.
- Reachability from an independent denied tailnet device or public-internet host.
- Interactive tray visual/menu behavior in this mission (previous Mission 005A
  was physically confirmed by the user; that is not a new tailnet test).
- Direct-LAN native HTTPS retest/rollback on the original network: `192.168.1.7`
  was not assigned to the laptop during this session. No alternate IP was selected,
  and no certificate was replaced to make that test pass. Direct-mode Rust TLS
  tests and backward-compatible settings tests passed.
- Bonjour/mDNS hardware compatibility and secure local discovery are deferred.

Follow the [hardware matrix](portable-connectivity.md#hardware-validation-matrix-record-actual-results)
to record these separately. Laptop-side success is not a claim of roaming success.

## Mission 005B changed files

- `src/tailnet.rs`: read-only CLI/status/Serve/DNS checks, readiness gate and tests.
- `src/config.rs`, `src/settings.rs`: explicit backend CLI/settings mode; old direct
  LAN schema remains valid; incompatible/security-weakening combinations rejected.
- `src/http.rs`: strict actual loopback-peer boundary and fail-closed readiness;
  existing auth, responses and clipboard rules preserved.
- `src/app.rs`, `src/lifecycle.rs`, `src/tray.rs`: background adapter checks,
  shutdown gating, endpoint and connectivity states; no blocking tray work.
- `src/lib.rs`, `src/main.rs`: module registration and CLI help.
- `src/transport_tests.rs`: real TCP peer-enforcement regression test.
- `Cargo.toml`, `Cargo.lock`: Tokio process/I/O support (two small transitive
  packages); no new networking framework.
- `scripts/Test-Tailnet.ps1`, `scripts/Test-ClipboardPost.ps1`: real verified
  tailnet HTTPS/native integration and restricted hostname support.
- `settings.tailnet.json.example`, `README.md`, `docs/portable-connectivity.md`,
  `docs/mission-005b-validation.md`: safe settings, threat model, setup, evidence,
  manual matrix and rollback.

Earlier uncommitted Mission 005A files/changes were preserved. Mission 005B did not
edit `clipboard.rs`, `writes.rs`, `watcher.rs`, `native.rs`, `network.rs`, certificate
provisioning, or the TLS implementation. No repository was initialized or rewritten.
