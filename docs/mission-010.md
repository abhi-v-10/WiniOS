# Mission 010 — Smart Receive

One manually configured iPhone Shortcut can receive either Windows clipboard
text or a PNG, without a selection menu. Existing Send/Receive/image helper
Shortcuts and published iCloud links are unchanged. There is no pairing, UI,
settings migration, credential rotation, certificate or Tailscale change.

## API and race protection

All requests use the existing **validated private HTTPS endpoint** and existing
`Authorization: Bearer <your token>`. The new endpoint is:

```http
GET /v1/clipboard/type
Authorization: Bearer <your token>
```

Successful JSON has exactly two fields:

```json
{"kind":"text","version":"opaque value returned by this request"}
```

`kind` is `text`, `image`, or `empty`. `empty` means no transferable supported
content: a genuinely empty clipboard or a settled unsupported format. With image
sharing disabled, an image-only observation remains unsupported and therefore
reports `empty`. It is never reported as a downloadable image. A pending read,
temporary failure, malformed/oversized content, private setup export or stopped
watcher returns **503 with no contents**, rather than a guessed kind.

For the detected kind, perform the existing GET with this **additional header**:

```http
GET /v1/clipboard
Authorization: Bearer <your token>
X-WiniOS-Clipboard-Version: <exact version string from the JSON>
```

Use `/v1/clipboard/image` for the image branch. Do not put either header in the
URL. The version is an opaque public snapshot identifier, not a credential or a
content hash. Do not parse, edit, quote, persist or reuse it across runs.

Every router lifetime gets a fresh 128-bit namespace from Windows' system RNG,
independent of the bearer token, combined with the store revision. A clipboard
change, invalidation, identical recopy/new observation or server restart makes an
old bound request fail **412 Precondition Failed**, with an empty response body.
Malformed/duplicate version headers fail 400; they never fall back to an unbound
request. If RNG initialization fails, metadata and well-formed bound requests fail
503; existing unbound endpoints remain operational.

Both metadata and fetch recheck the store revision and current native Windows
clipboard sequence immediately before returning. A sequence-only change before
the watcher catches up fails 409; an unknown/zero sequence fails 503. Image bodies
also retain the existing revision/sequence/admission/deadline checks throughout
streaming. A mid-download change aborts the incomplete body. Bytes already handed
off cannot be recalled; clients must reject incomplete downloads. This binds the
fetch to the detected observation without retaining an old clipboard snapshot or
serving historical data. The final native check is the read's validation point;
a later copy cannot retroactively recall an already completed transfer.

Requests **without** `X-WiniOS-Clipboard-Version` retain their existing contracts:
text GET remains text/plain UTF-8, image GET remains image/png, POSTs are unchanged.
There is no implicit content negotiation or silent downgrade. A version alone
never grants access: both calls still require bearer auth and trusted peer /
Tailscale readiness. Pause and detected readiness loss cancel authenticated
requests with 503; unauthorized requests remain 401. All responses use no-store
and nosniff. No contents, tokens, namespaces or request bodies are logged.


## iPhone setup

Install the two standalone Smart Shortcuts using [Mission 018](mission-018.md).
