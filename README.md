# WiniOS agent — Mission 001.6

A local Windows clipboard watcher using the `windows` crate and Win32. No server,
networking, or frontend is included.

## Run

Requires Windows and a Rust toolchain with the Windows SDK/linker installed.

```powershell
cd C:\WiniOS
cargo run
```

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
- Ctrl+C/Ctrl+Break posts a shutdown message from the console callback. The main
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
Only `WindowsClipboard` origin exists; no clipboard writer or networking is implemented.

Windows sequence numbers are compared for equality, never numerical ordering.
They are checked before reading, while the clipboard is still open after reading,
and after closing it. If they change, the result is discarded and a fresh
observation is scheduled. Delayed rendering can advance the sequence during a
read. Zero is treated conservatively as unknown, with no text published.
See [Microsoft's sequence-number contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber).

Snapshots describe the last processed observation, not a transaction with the OS.
A change after the final sequence check is invalidated by its queued notification;
there is necessarily a notification-delivery interval. Future serving code must
respect `Unavailable` and revalidate freshness if it needs a stronger guarantee.

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

The CLI reports state transitions and metadata. To inspect complete snapshots in
these manual checks, use a Rust debugger at `Capture::publish` after `complete()`;
no read endpoint exists yet. Source unit tests exercise the snapshot API directly.

Notepad/VS Code copy delivery, real ownership contention, and interactive
terminal Ctrl+C require validation on a Windows desktop.
