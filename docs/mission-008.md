# Mission 008: private image clipboard

Images use a separate PNG API. Existing text Shortcuts and `GET/POST
/v1/clipboard` retain their text contract. The compact native tray, Pause/Resume,
startup choice, singleton, credentials, certificates and Serve configuration are
preserved. No published iCloud Shortcut is edited by Windows.

## Enable and configure

1. Open WiniOS Settings from the tray and select **Images**.
2. Select **Enable image sharing**, review the consent, choose **2, 4 or 8 MiB**,
   then **Apply, restart service**. Images default **OFF**. This is a saved setting;
   the page distinguishes active and saved values. Restart preserves Pause.
3. For iPhone uploads also enable **Clipboard → Allow iPhone to replace Windows
   clipboard**, confirm and apply. Sharing must be resumed and the connection ready.

Secret-free image preferences are atomically saved to `images.json` beside
`settings.json` (normally `%LOCALAPPDATA%\WiniOS`). No existing settings file is
rewritten merely by upgrading or opening the page. Changes reject stale edits.
CLI equivalents: `--enable-images --max-image-mib 8`, together with the existing
secure transport and explicit `--allow-clipboard-write` options. Images are
rejected in the legacy insecure HTTP mode. Tailscale's dedicated same-host HTTP
backend remains loopback-only behind validated private HTTPS Serve.

## API

Use the **existing verified HTTPS hostname** plus `/v1/clipboard/image`. If Copy
Endpoint produces the full text URL ending `/v1/clipboard`, append `/image` once.
Never place the bearer token in a URL. Both methods require
`Authorization: Bearer <your existing token>`.

| Request | Success | Other outcomes |
| --- | --- | --- |
| GET `/v1/clipboard/image` | 200, `image/png`, exact Content-Length | 204 when current clipboard is empty/text; 409 for a stale sequence/version; 503 for pending/unknown/unavailable, pause, contention or disconnected tailnet; 413 for oversized current content |
| POST `/v1/clipboard/image` | 200 JSON acknowledgement with operation/sequence, only after native acceptance | 415 for content type/encoding; 422 for malformed/unsupported PNG; 413 for byte/pixel/dimension limits; 408 for body timeout; 504 for conversion/ack timeout; 503 for contention, full capacity, cancellation or unavailable service; 500 for native failure |

Both return 401 for missing/wrong bearer, including while paused. Disabled image
sharing returns 403; POST also returns 403 without explicit write permission.
Peer/readiness checks and no-store/nosniff headers apply as for text. Health and
diagnostics never contain image bytes, clipboard text or credentials.

POST is **raw PNG bytes**, with exactly `Content-Type: image/png`: no JSON,
multipart form, base64, JPEG, HEIC, APNG, compressed HTTP body or trailing data.
Canonical PNG output strips metadata; its encoded size must also fit the limit.
Headers/response bodies are deliberately not logged.

## Limits and ownership

- Selected encoded limit: 2/4/8 MiB for uploads and downloads, hard maximum 8 MiB.
- Maximum side: 8192 pixels; maximum area: 8,388,608 pixels; RGBA buffer: 32 MiB.
  A 12 MP photo exceeds the area limit: resize it first. A detailed/noisy 4K
  screenshot can fit the pixel limit yet exceed the encoded-byte limit.
- One upload/decode/write lease and two download producers per store; the shared
  native write queue remains bounded at four commands. Body deadline 5 seconds,
  codec budget 3 seconds, native acknowledgement 3 seconds. Download production
  has an absolute 10-second deadline and bounded 64 KiB chunks. Already delivered
  socket bytes cannot be recalled; incomplete Content-Length is an error.
- PNG framing, CRCs and dimensions are checked before decode. Decoder allocations
  and DIB strides/offsets are bounded; animation and unusual native encodings are
  rejected. Codec work runs off the Win32/UI threads with cooperative cancellation.
  Limits bound transient memory too, but process usage can exceed 32 MiB while
  simultaneous source/decoded/native buffers exist. This is not a zero-copy path.
- The watcher holds OpenClipboard only to copy bounded native data. It reads
  registered PNG, explicit CF_DIBV5/CF_DIB, or bounded GetDIBits RGB conversion
  for compatible CF_BITMAP publishers. CF_UNICODETEXT keeps priority if both text and image
  representations are offered. 24/32-bit uncompressed standard RGB layouts are
  supported; palettes, nonstandard masks/compression/profiles fail closed.
- The message thread writes registered PNG, 32-bit CF_DIBV5 (alpha) and 24-bit
  CF_DIB (white-flattened alpha) for paste compatibility. All allocations precede
  EmptyClipboard. Accepted handles belong to Windows; RAII frees only unaccepted
  handles. A partial native failure is an error and invalidates readable state.
- Images and text share one current state/version. New observations immediately
  invalidate readable content. Temporary failures keep only a private comparison
  baseline; clearing/rejection/exhausted retries break deduplication. Conversion
  completion must match its observation and native sequence. Remote operation
  suppression never hides a later deliberate local copy. GET checks current
  version/sequence before serving; image streaming rechecks between chunks.
- No WiniOS image files or disk caches are created. Uploaded images request
  exclusion from Windows clipboard history/cloud sync using Microsoft's native
  formats. Other local apps can read the Windows clipboard, and local copies may
  still enter OS/application history independently. Loopback is not process-level
  authentication. Existing bearer and Tailscale access controls remain essential.

PNG is normalized to 8-bit RGBA. ICC/gamma/EXIF metadata is removed, not color
managed: wide-gamut/HDR images can change appearance, and 16-bit channels are
reduced to 8-bit. This prototype transfers one image, not files, animated images,
photo albums or mixed clipboard representations. Pasting requires an image-aware
target such as Paint, Notes or a compatible message editor; Notepad cannot paste
an image. A lost/timeout response can leave an already accepted clipboard write
with an unknown client outcome; inspect before retrying.

Microsoft references: [GetDIBits compatible bitmap conversion](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-getdibits),
[format enumeration and synthesized order](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumclipboardformats),
[clipboard history/cloud exclusion formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats).

## iPhone image transfer

Use only the two standalone Smart Shortcuts described in [Mission 018](mission-018.md).

## Verification commands and manual checks

```powershell
cd C:\WiniOS
cargo fmt --check
cargo check --offline
cargo test --offline
cargo build --release --offline
# Running ready tailnet agent; images and writes enabled; overwrites clipboard
# with disposable fixtures. Does not print bodies/tokens or persist images:
.\scripts\Test-ClipboardImages.ps1
# Existing verified HTTPS/native text regression:
.\scripts\Test-ClipboardPost.ps1 -BaseUri https://<your-node>.ts.net/ -Tailnet
```

The image script discovers the actual local Tailscale hostname, verifies normal
TLS trust, and checks PNG/native DIB/bitmap pixels, acknowledgements, auth, limits,
contention/recovery, clearing, malformed data and Unicode text recovery. It does
not bypass TLS or alter Serve/firewall. It requires **8 MiB** selected (its hard
limit fixture is 8 MiB + 1). It replaces the system clipboard; do not run while
working with important clipboard content.

Two opt-in native tests require stopping WiniOS first, then relaunching the tray:

```powershell
cargo test native_image_startup_formats_remote_suppression_and_stale_conversion -- --ignored --nocapture
cargo test native_private_setup_export_is_unreadable_and_survives_owner_restart -- --ignored --nocapture
```

Manual acceptance: test the two image Shortcuts above; retry Unicode text both
ways; copy text after an image and confirm image GET returns no old image; clear
the clipboard; pause and verify image GET/POST fail while unauthenticated requests
remain 401; resume; toggle image sharing off and apply; verify text still works.
Test image Settings at normal/high DPI, keyboard navigation and confirmation
cancellation. Quit/relaunch and verify one process and saved preferences. Test
Win+Shift+S, Paint paste, transparent PNG, iPhone screenshot/photo (resized),
cellular tailnet and a slow/disconnected transfer separately. No claim of
physical iPhone image or visual UI acceptance follows from automated tests.

## Rollback

Quit WiniOS gracefully. Reinstall the pre-Mission-008 backed-up WiniOS-Setup.exe
(same per-user directory) and launch once. Backups are under
`C:\WiniOS\target\mission008-backup-*`; the validated deployment record names the
exact backup. The extra secret-free `images.json` can remain: the older app does
not use it. Do not delete settings, credentials, TLS or certificates; do not reset
Tailscale. Existing text Shortcuts require no URL changes. Delete/disable only
the two standalone Smart Shortcuts if desired. The user-managed Serve route
is unchanged by upgrade, testing, Quit or rollback.

## Results on this Windows laptop

On 2026-11-06, `cargo fmt --check`, `cargo check --offline`, `cargo test --offline`
and `cargo build --release --offline` passed: **104 passed, 0 failed, 4 ignored**.
Three opt-in tests were also run separately and passed: native image startup /
remote suppression / stale conversion, private setup-export isolation, and the
read-only live HTTPS setup probe. The remaining ignored test is an interactive
preview, not an acceptance result.

`Test-ClipboardImages.ps1` passed over this laptop's existing Tailscale HTTPS Serve
with ordinary certificate validation. It covered PNG, padded native PNG,
CF_DIBV5, CF_DIB24 and compatible CF_BITMAP capture with exact fixture pixels,
acknowledged native uploads, authentication, limits, contention retries/exhaustion,
clearing/malformed invalidation, and the existing Unicode text integration script.
Synthetic bitmap fixtures use a compatible Win32 DDB; private/invalid native GDI
handles fail closed. A System.Drawing GetHbitmap fixture was not a valid
cross-process compatible bitmap in this environment and is not counted as a pass.
Early sandbox-only test attempts were denied access to sockets/registry/atomic
filesystem operations; the actual full suite was rerun with normal OS permissions.

The validation snapshot confirmed personal files, TLS, Credential Manager token,
startup registration and Serve configuration unchanged, and restored one installed
tray process. The pre-update rollback package/binaries are in
`C:\WiniOS\target\mission008-backup-20261106-235033`.
Physical iPhone image Shortcuts, Snipping Tool UI, Paint/Notes paste, roaming and
visual/accessibility acceptance still require the manual checks above. Existing
physical text-transfer acceptance from earlier missions is not image acceptance.
