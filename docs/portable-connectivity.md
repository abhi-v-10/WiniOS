# Portable connectivity (Mission 005B)

## Decision and phases

Use Tailscale **Serve**, a private HTTPS reverse proxy, as a roaming adapter.
The clipboard library, Win32 message thread, queue acknowledgements, retry timers,
sequence validation, origin tracking and Credential Manager token are unchanged.
No clipboard text is persisted. No autostart is registered.

| Option | Reach | Identity / cost | Decision |
|---|---|---|---|
| Bonjour/mDNS DNS-SD | Same multicast LAN | Needs a stable collision-aware hostname, restricted firewall rules and matching trusted certificate; guest isolation can block it | Separate later milestone |
| Tailscale Serve | Same LAN or different networks | Node's tailnet DNS hostname, managed HTTPS certificate, local reverse proxy; Tailscale installed on both devices | Recommended adapter |
| Direct Tailscale IP binding | Same tailnet | Overlay IP stability helps routing, but IP-scoped TLS still needs provisioning/trust; CGNAT addresses are intentionally rejected by the existing LAN mode | Not implemented |
| Existing direct-LAN HTTPS | Approved private LAN only | Existing IP SAN and dedicated constrained CA; configured IP must remain assigned | Preserved optional mode |

Phase A implements the adapter, settings, background readiness gate and diagnostics.
Phase B is research only. Phase C includes automated tests and the hardware matrix
below. Phase D documents a one-time setup followed by terminal-free tray operation.

Tailscale documents [Serve](https://tailscale.com/docs/reference/tailscale-cli/serve),
[HTTPS prerequisites](https://tailscale.com/docs/how-to/set-up-https-certificates),
and [connection types](https://tailscale.com/docs/reference/connection-types).
Serve is private to the tailnet. **Funnel is a different, public feature and must
not be enabled.** WiniOS never invokes a configuration-changing Tailscale command.

## Threat model and trust boundary

```text
iPhone Shortcuts (Tailscale VPN connected)
    -- HTTPS, normal chain + hostname verification, bearer token -->
Tailscale Serve on this Windows PC (tailnet hostname, TCP 443)
    -- same-host HTTP --> 127.0.0.1:8766
WiniOS API -- existing bounded command queue --> dedicated Win32 clipboard thread
```

- Untrusted Wi-Fi participants and the public internet get no WiniOS listener in
  tailnet mode. The backend cannot bind a LAN, wildcard, IPv6 or Tailscale address.
  No port forwarding, public relay, firewall change or TLS downgrade is provided.
- Tailnet membership alone does not authorize clipboard access. Both GET and POST
  require the existing bearer token. Writes additionally require explicit opt-in.
  Limit tailnet grants/ACLs to your approved iPhone/user and this node's TCP 443;
  the tailnet's default policy may permit every member to contact the port.
  Remove unneeded sharing and Funnel capability. WiniOS does not edit that policy.
- Tailscale's local daemon terminates TLS and can see the HTTP request/token.
  The same-host daemon, Windows host and authorized iPhone are trusted endpoints.
  Host compromise, an administrator, or a compromised bearer credential are outside
  this adapter's protection. Keep Windows/iOS/Tailscale patched and accounts secured.
- **Loopback is not process-level authentication.** Another local process may
  connect to the backend; it still needs the bearer token. Local administrators or
  code running as this Windows user may also access that user's Credential Manager.
- The actual accepted TCP peer must be exactly IPv4 `127.0.0.1`. `Host`,
  `X-Forwarded-For`, Tailscale identity/capability headers and other injected headers
  never authorize a request or establish the phone's identity/IP. Proxy headers
  cannot make a direct LAN peer trusted.
- Status comes from the installed official CLI under Windows Program Files, not
  the current directory/PATH. WiniOS checks sign-in/connection, MagicDNS, the local
  node's DNS name and permitted certificate domains, exact HTTPS root proxy target,
  absence of Funnel/alternate routes, and DNS answers matching the node addresses.
  Renaming/changing tailnet identity after connection requires a WiniOS restart.
- Unknown or conflicting Serve configurations fail closed. This intentionally
  supports a dedicated node Serve profile, not arbitrary shared Serve routes or
  Tailscale Services. A future CLI schema change may require a reviewed update.
- Readiness checks are asynchronous and bounded: CLI queries have 4-second
  deadlines and 1 MiB output caps, DNS has a 3-second wait deadline. Checks run
  every 30 seconds when connected, every 5 seconds otherwise. A positive result
  expires after 45 seconds. Failures close the gate; stale clipboard text is never
  substituted. Changes are detected within this check interval, not instantly.
  In-flight writes already accepted may complete under the normal bounded rules.
- `Connected` means local prerequisites/configuration/DNS were verified. It is
  not proof that a remote phone is reachable or that a fresh external certificate
  handshake has succeeded. The integration test and Safari verify that separately.
- Raw CLI output, login URLs, identities, tokens and request bodies are not logged.
  Diagnostics show only readiness, endpoint and existing metadata. No disk log.

Tailscale uses coordination infrastructure and may relay encrypted traffic through
DERP/peer relays. It provides no cloud clipboard storage here. It cannot guarantee
operation on networks blocking VPN/connectivity infrastructure. HTTPS remains
validated even on the private overlay. Publicly issued certificates publish node
DNS names in certificate-transparency logs: choose a non-sensitive machine name.

## One-time Windows and iPhone setup

1. Keep the working direct-LAN settings and `%LOCALAPPDATA%\WiniOS\tls` identity.
   Do not regenerate, move, replace or broaden the dedicated Windows CA. Record
   your existing settings in a separate local backup before switching modes.
2. Install the official [Windows Tailscale client](https://tailscale.com/download/windows)
   and [iPhone client](https://tailscale.com/download/ios). Sign in on both devices
   to the same approved tailnet and connect the VPN. Account sign-in and device
   approval are user actions; never paste account credentials/auth keys into WiniOS.
3. In the Tailscale admin console, review/enable MagicDNS and HTTPS certificates.
   Review the certificate-transparency notice. Review grants/ACLs so only your
   approved devices/users can reach this node's HTTPS port. Do not enable Funnel,
   public sharing, an exit node or a subnet router for this setup.
4. In PowerShell, inspect locally (these commands change nothing):

   ```powershell
   $ts = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
   $node = & $ts status --json --peers=false | ConvertFrom-Json
   $node.BackendState
   $name = $node.Self.DNSName.TrimEnd('.')
   "https://$name"
   & $ts serve status
   ```

   Do not post raw status JSON or credentials in bug reports. Review existing Serve
   usage first: WiniOS requires a dedicated HTTPS root route on 443 and rejects
   other active routes/listeners/Services/foreground profiles. Do not reset someone
   else's Serve configuration. Resolve conflicts with its owner.
5. **User approval is required before this configuration change.** After reviewing
   the exact target and scope, run this command yourself or explicitly authorize it:

   ```powershell
   & $ts serve --bg --https=443 http://127.0.0.1:8766
   & $ts serve status
   ```

   This is **Serve**, never Funnel. If HTTPS prerequisites need approval, review
   the official Tailscale prompt/admin link; do not bypass it. Do not expose a LAN
   target or reuse `--insecure-http`/`--allow-insecure-clipboard-write`.
6. Build and check the GUI binary (Quit any WiniOS instance before replacement):

   ```powershell
   cd C:\WiniOS
   cargo fmt --check
   cargo check
   cargo test
   cargo build --release
   .\scripts\Test-DesktopBinary.ps1
   ```

   Install `target\release\winios.exe` and `winios-agent.exe` together under your
   per-user `%LOCALAPPDATA%\Programs\WiniOS` directory as described in the main
   README. Keep only one agent running. No admin account or Windows Service is
   needed for WiniOS's normal operation; Tailscale installs its own system service.
7. Quit WiniOS. Back up the existing settings and use this secret-free config in
   `%LOCALAPPDATA%\WiniOS\settings.json`:

   ```json
   {
     "mode": "tailnet-backend",
     "port": 8766,
     "allow_clipboard_write": true
   }
   ```

   The checked-in `settings.tailnet.json.example` defaults writes to **false**.
   Set true only if you want the POST Shortcut. Omit `lan_ip`, `tls_cert` and
   `tls_key`; they are rejected in this mode, as are legacy insecure settings.
   The port may be customized but must not be 8765; the approved Serve target must
   exactly match it. No secrets or account identifiers belong in this file.
8. Double-click `winios.exe`. Tray status should progress to **Connected** and
   display the discovered `https://<this-node>.<tailnet>.ts.net` URL. No console
   window should appear, including during background Tailscale status checks.
   Missing/invalid config is an error; missing Serve/sign-in is **Not configured**;
   disconnected VPN is **Tailscale disconnected**; network/DNS/health problems show
   **Waiting for network**. Errors do not fall back to direct LAN or plaintext LAN.
   Connectivity can recover automatically; settings changes require Quit/relaunch.
9. On iPhone, connect Tailscale. Open `https://<discovered-name>/v1/health` in Safari.
   It must load without a certificate warning. A normal publicly trusted Serve
   certificate requires no new WiniOS CA installation. Never ignore trust errors.
10. Update the **URL only** in each existing Shortcut once:

    - Windows → iPhone: `https://<discovered-name>/v1/clipboard`, Get Contents of
      URL **GET**, `Authorization: Bearer <existing-token>`, Copy result to Clipboard.
    - iPhone → Windows: Get Clipboard, same HTTPS URL, Get Contents of URL **POST**,
      JSON field `text` = clipboard variable, same Authorization header, then success
      notification. Notify success only when the HTTP request succeeds. On Windows,
      paste with Ctrl+V.

    Keep the existing token private; do not put it in the URL, screenshots, terminal
    history, diagnostics or shared Shortcuts. Transfer it using the existing explicit
    Credential Manager export workflow if needed. Existing HTTP status/error and
    1 MiB text limits, no-store headers, acknowledgements and origin rules still apply.

The laptop must be **awake, connected, Tailscale connected, and WiniOS running**.
The iPhone VPN must also be connected, including on cellular. Sleep, expired node
authorization, DNS failures, account policy or blocked VPN networks can interrupt
access. Router IP changes do not require changing this URL or the old LAN certificate.
Renaming/re-enrolling the Tailscale node or changing its tailnet DNS suffix does
change the URL: review the new identity and update both Shortcuts explicitly.

## Console diagnostics and automated integration

The console mode remains available independently of tray setup:

```powershell
.\target\release\winios-agent.exe --tailnet-backend --allow-clipboard-write
```

Never run it alongside the tray instance. It prints metadata only. A missing
Tailscale executable/service, unreadable local API, bad Serve configuration or
conflicting route never results in a ready clipboard backend. Fix prerequisites;
WiniOS will not run `tailscale up`, configure Serve, change network/firewall settings
or install trust roots for you.

With one tailnet-mode tray instance already running:

```powershell
# Normal OS certificate/hostname verification; no custom CA or insecure option.
.\scripts\Test-Tailnet.ps1
# Overwrites the Windows clipboard with disposable Unicode test text:
.\scripts\Test-Tailnet.ps1 -NativeClipboard
```

The script discovers the current node, checks the Serve target and DNS, verifies
HTTPS health and unauthenticated GET/POST rejection, then optionally runs the
existing native Unicode, contention, acknowledgement and GET recovery suite.
No `-SkipCertificateCheck`, insecure curl flag, TLS callback bypass or automatic
trust/network change is used. Running on the host tests the real Serve HTTPS path,
but does **not** establish off-host access or iPhone roaming success.

An initial HTTPS probe can time out while first-time Serve setup/certificate work
is still completing. Inspect Tailscale's status, wait briefly and retry with normal
verification. A timeout is a failed test, not proof of a certificate problem or a
reason to use HTTP, disable verification, install a different root or regenerate
the existing LAN CA. Persistent failure needs connectivity/Serve troubleshooting.

Rust tests retain all prior clipboard/HTTPS tests and cover explicit mode validation,
settings compatibility, loopback peer enforcement with spoofed headers, mandatory
auth, readiness invalidation/expiry/recovery, exact Serve route/Funnel rejection,
node identity verification, queue acknowledgement and tray lifecycle states.

## Hardware validation matrix (record actual results)

Use disposable text only. Keep one Windows agent. Do not change adapters, VPN,
firewall, DNS or sleep settings remotely without user approval and a way to recover.
No real roaming test is established merely by changing a mocked IP in a unit test.

| Test | Exact action and expected result |
|---|---|
| Same Wi-Fi | Safari health succeeds; copy Unicode on PC, GET Shortcut → iPhone paste; copy different Unicode on iPhone, POST → Windows Ctrl+V exact |
| Ethernet → Wi-Fi | With permission, unplug Ethernet/connect approved Wi-Fi; wait for Tailscale; same hostname and both Shortcuts recover without certificate/config change |
| Home → mobile hotspot | With permission, join a trusted phone hotspot/new router; same HTTPS URL and both transfers work |
| iPhone cellular → PC | Keep PC online; disable iPhone Wi-Fi, leave its Tailscale VPN connected; both Shortcuts work over cellular |
| Windows Tailscale disconnect | With permission disconnect VPN; local tray/gate becomes disconnected/unavailable within check interval; no clipboard served; reconnect recovers |
| iPhone VPN disabled | Disable iPhone Tailscale; HTTPS is unreachable; no Shortcut success; re-enable and retry |
| Sleep/resume | Sleep PC with permission; phone cannot reach it; resume/reconnect; both transfers recover |
| Internet loss / DNS failure | With permission use controlled interruption; no silent HTTP/LAN fallback; gate reports waiting/error and requests fail; restore normal settings and retry |
| Missing/wrong bearer | HTTPS GET and POST without valid token return 401 and no clipboard text, including for another authorized tailnet device |
| Public/unauthorized device | From a device outside the tailnet, or denied by its policy, HTTPS cannot reach this service; backend port 8766 is unreachable from LAN; never enable Funnel to test |
| Invalid certificate | A client without the correct validated server identity/trust must fail TLS. Existing Rust TLS tests exercise untrusted/wrong-name/expired certs; do not install a bypass to simulate success |
| Old HTTP URL | `http://<tailnet-name>/v1/clipboard` cannot retrieve text or redirect with a token; there is no approved port-80 route. Never send a real bearer token over plaintext to test this |
| Quit | Tray Quit removes icon, closes 127.0.0.1:8766, cancels/completes pending writes and releases singleton; Serve may return 502 until disabled (see below) |
| LAN rollback | Disable approved Serve route, restore original LAN settings, relaunch; existing `https://192.168.1.7:8765` and both old LAN Shortcuts still work, without certificate replacement |

## Serve lifetime, removal and rollback

**Explicit lifecycle boundary:** `--bg` Serve is user-managed and persists across
WiniOS Quit, Tailscale restart and Windows reboot. WiniOS does not pretend to own
or silently remove this configuration. Tray Quit closes the backend; Serve then
normally returns a proxy error, and no WiniOS clipboard service remains listening.
Do not reuse port 8766 for another service while that proxy remains configured:
Serve would expose that new local service to your tailnet. This is a known local
port-reuse limitation, not process-level authentication. Disable the route before
uninstalling, changing backend ports or repurposing the port.

After inspecting the current Serve status and **approving this network change**:

```powershell
$ts = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
& $ts serve status
& $ts serve --https=443 off
& $ts serve status
```

Do not run `serve reset`: it could delete other services. Do not turn Tailscale
off/log out automatically: other applications may need it. Disconnect/remove
devices manually in Tailscale when appropriate, revoke any lost-device access,
and rotate the WiniOS bearer credential using the existing documented workflow.
Also remove the old token from both Shortcuts. Rotating a token does not require
replacing the LAN CA. Node keys and Serve certificate lifecycle belong to Tailscale.

To roll back, Quit WiniOS, disable the reviewed Serve route, restore the backed-up
direct-LAN settings (or `{"lan_ip":"192.168.1.7","port":8765,
"allow_clipboard_write":true}` for the original setup), and double-click the tray
binary. That IP must still be locally assigned and match the original certificate.
The LAN listener still fails closed on IP removal, certificate mismatch or expiry.
Switch the Shortcut URLs back if desired. No CA/trust replacement is involved.
For rollback to the old application version, replace the binaries only after Quit
and restore direct-LAN settings; old versions will reject the new mode field.

## mDNS investigation: deferred

Windows 10+ exposes native asynchronous
[DNS-SD registration](https://learn.microsoft.com/en-us/windows/win32/api/windns/nf-windns-dnsserviceregister).
Apple supports [Bonjour](https://developer.apple.com/bonjour/) and applies
[local-network privacy permissions](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy).
Guest networks, AP client isolation and multicast filtering may prevent discovery.
Apple Shortcuts Get Contents of URL is an HTTP client, not a general DNS-SD service
picker; a known hostname does not create a portable discovery UX by itself.

A later milestone must select a stable explicit hostname/device identity, handle
collisions without silently changing certificate identity, assess Windows/iOS
multicast behavior on hardware, obtain trusted-network approval, and restrict
firewall profiles. A `.local` name does not create TLS trust. The existing WiniOS
CA is constrained to the chosen IPs and the `winios.invalid` DNS subtree, so an
arbitrary `.local` certificate is incompatible. Do not reuse/broaden/regenerate that
CA automatically, weaken validation or advertise on untrusted/public networks.
