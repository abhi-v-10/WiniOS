//! All Win32 calls and clipboard-owned pointers stay on the message thread.
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Error, Result, w};

use crate::clipboard::{
    ClipboardState, ClipboardStore, MAX_TEXT_BYTES, Observation, ReadValue, TextError,
    UnavailableReason, Update, decode_text,
};
use crate::writes::{WriteCommand, WriteError, WriteReceipt, validate_text};
use tokio::sync::mpsc;

const CF_UNICODETEXT: u32 = 13; // Standard Win32 clipboard format.
const RETRY_TIMER: usize = 1;
const SHUTDOWN: u32 = WM_APP + 1;
const WRITE_COMMAND: u32 = WM_APP + 2;
const WRITE_TIMER: usize = 2;
const BACKOFF_MS: [u32; 6] = [20, 40, 80, 160, 320, 640];
static MESSAGE_THREAD: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn console_handler(event: u32) -> BOOL {
    if event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT {
        let thread = MESSAGE_THREAD.load(Ordering::SeqCst);
        if thread != 0 {
            // No allocation, logging, locks, or cleanup on the console callback thread.
            return unsafe { PostThreadMessageW(thread, SHUTDOWN, WPARAM(0), LPARAM(0)) }
                .is_ok()
                .into();
        }
    }
    false.into()
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Application events are handled in the Rust message loop, outside the FFI callback.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

struct Window {
    hwnd: HWND,
    instance: HINSTANCE,
    listening: bool,
}

impl Window {
    fn new() -> Result<Self> {
        // SAFETY: constant class name and process module remain valid through Drop.
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None)?.0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: w!("WiniOSClipboardWatcher"),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(Error::from_thread());
            }
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!("WiniOS clipboard watcher"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let _ = UnregisterClassW(class.lpszClassName, Some(instance));
                    return Err(error);
                }
            };
            let mut window = Self {
                hwnd,
                instance,
                listening: false,
            };
            AddClipboardFormatListener(hwnd)?;
            window.listening = true;
            Ok(window)
        }
    }

    fn cancel_retry(&self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), RETRY_TIMER);
        }
    }

    fn retry_after(&self, milliseconds: u32) -> Result<()> {
        if unsafe { SetTimer(Some(self.hwnd), RETRY_TIMER, milliseconds, None) } == 0 {
            Err(Error::from_thread())
        } else {
            Ok(())
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            self.cancel_retry();
            let _ = KillTimer(Some(self.hwnd), WRITE_TIMER);
            if self.listening {
                let _ = RemoveClipboardFormatListener(self.hwnd);
            }
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(w!("WiniOSClipboardWatcher"), Some(self.instance));
        }
    }
}

struct ConsoleHandler;

impl ConsoleHandler {
    fn new() -> Result<Self> {
        MESSAGE_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
        if let Err(error) = unsafe { SetConsoleCtrlHandler(Some(console_handler), true) } {
            MESSAGE_THREAD.store(0, Ordering::SeqCst);
            return Err(error);
        }
        // Launchers can pass down an inherited "ignore Ctrl+C" attribute.
        // Installing a callback alone does not clear it. Enable delivery only
        // after our handler is ready.
        if let Err(error) = unsafe { SetConsoleCtrlHandler(None, false) } {
            unsafe {
                let _ = SetConsoleCtrlHandler(Some(console_handler), false);
            }
            MESSAGE_THREAD.store(0, Ordering::SeqCst);
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for ConsoleHandler {
    fn drop(&mut self) {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(console_handler), false);
        }
        MESSAGE_THREAD.store(0, Ordering::SeqCst);
    }
}

struct OpenClipboardGuard;
impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

struct LockedGlobal(HGLOBAL);
impl Drop for LockedGlobal {
    fn drop(&mut self) {
        // GlobalUnlock returning false may simply mean the lock count reached zero.
        unsafe {
            let _ = GlobalUnlock(self.0);
        }
    }
}

enum ReadError {
    Temporary(Error),
    Invalid(TextError),
    InvalidAllocation,
    SequenceUnknown,
    Changed,
}

pub fn sequence() -> Option<u32> {
    match unsafe { GetClipboardSequenceNumber() } {
        0 => None,
        value => Some(value),
    }
}

fn read_text(hwnd: HWND, expected: Option<u32>) -> std::result::Result<ReadValue, ReadError> {
    // SAFETY: clipboard is held open until after the global lock guard drops.
    // Its handle belongs to Windows: never GlobalFree it or retain its pointer.
    unsafe {
        OpenClipboard(Some(hwnd)).map_err(ReadError::Temporary)?;
        let _clipboard = OpenClipboardGuard;
        let before = sequence().ok_or(ReadError::SequenceUnknown)?;
        if Some(before) != expected {
            return Err(ReadError::Changed);
        }
        let value = if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            // Zero can mean either no formats or a failed call.
            windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
            let count = CountClipboardFormats();
            let error = windows::Win32::Foundation::GetLastError();
            if count == 0 && error.0 != 0 {
                return Err(ReadError::Temporary(Error::from_thread()));
            }
            if count == 0 {
                ReadValue::Empty
            } else {
                ReadValue::Unavailable(UnavailableReason::UnsupportedFormat)
            }
        } else {
            let handle = HGLOBAL(
                GetClipboardData(CF_UNICODETEXT)
                    .map_err(ReadError::Temporary)?
                    .0,
            );
            let bytes = GlobalSize(handle);
            if bytes < 2 || bytes % 2 != 0 {
                return Err(ReadError::InvalidAllocation);
            }
            let units = (bytes / 2).min(MAX_TEXT_BYTES / 2 + 1);
            let pointer = GlobalLock(handle);
            if pointer.is_null() {
                return Err(ReadError::Temporary(Error::from_thread()));
            }
            let _lock = LockedGlobal(handle);
            let text = std::slice::from_raw_parts(pointer.cast::<u16>(), units);
            ReadValue::Text(decode_text(text).map_err(ReadError::Invalid)?)
        };
        // Delayed rendering can advance the sequence inside GetClipboardData.
        // Discard this read and reobserve; never label rendered data with the old token.
        if sequence() != Some(before) {
            return Err(ReadError::Changed);
        }
        Ok(value)
    }
}

struct Capture {
    store: ClipboardStore,
    observation: Observation,
    retries: usize,
    deadline: Option<Instant>,
    debug: bool,
}

impl Capture {
    fn new(store: ClipboardStore, debug: bool) -> Self {
        let observation = store.begin_observation(sequence());
        Self {
            store,
            observation,
            retries: 0,
            deadline: None,
            debug,
        }
    }

    fn restart(&mut self, sequence: Option<u32>) {
        self.observation = self.store.begin_observation(sequence);
        self.retries = 0;
    }

    fn publish(&mut self, value: ReadValue) {
        match self.store.complete(&mut self.observation, value) {
            Ok(Update::Changed) => {
                let snapshot = self.store.snapshot();
                let time = unsafe { GetLocalTime() };
                let description = match &snapshot.state {
                    ClipboardState::Available(text) => {
                        format!("clipboard updated | {} characters", text.character_count)
                    }
                    ClipboardState::Empty => "clipboard empty".into(),
                    ClipboardState::Unavailable(reason) => {
                        format!("clipboard unavailable | {reason:?}")
                    }
                };
                println!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} local | {description} | version={} sequence={:?} origin={:?} operation={}",
                    time.wYear,
                    time.wMonth,
                    time.wDay,
                    time.wHour,
                    time.wMinute,
                    time.wSecond,
                    time.wMilliseconds,
                    snapshot.version,
                    snapshot.sequence,
                    snapshot.origin,
                    snapshot.operation_id.observation,
                );
            }
            Ok(Update::Duplicate | Update::Superseded | Update::Remote) => {}
            Err(error) => eprintln!("Skipped clipboard: {error}"),
        }
    }

    fn retry(&mut self, window: &Window) -> Result<()> {
        if let Some(&delay) = BACKOFF_MS.get(self.retries) {
            window.retry_after(delay)?;
            self.deadline = Some(Instant::now() + Duration::from_millis(delay.into()));
            self.retries += 1;
        } else {
            self.publish(ReadValue::Unavailable(UnavailableReason::RetriesExhausted));
        }
        Ok(())
    }

    fn attempt(&mut self, window: &Window) -> Result<()> {
        window.cancel_retry();
        self.deadline = None;
        let current = sequence();
        if current != self.observation.sequence() {
            self.restart(current);
        }
        let result = read_text(window.hwnd, self.observation.sequence());
        let after = sequence();
        // Covers changes both while the clipboard was open and after it closed.
        if after != self.observation.sequence() || matches!(result, Err(ReadError::Changed)) {
            self.restart(after);
            self.publish(ReadValue::Unavailable(UnavailableReason::ChangedDuringRead));
            return self.retry(window);
        }
        match result {
            Ok(value) => self.publish(value),
            Err(ReadError::Temporary(error)) => {
                self.publish(ReadValue::Unavailable(UnavailableReason::TemporaryFailure));
                if self.debug {
                    eprintln!("Clipboard access failed: {error}");
                }
                self.retry(window)?;
            }
            Err(ReadError::SequenceUnknown) => {
                self.publish(ReadValue::Unavailable(UnavailableReason::SequenceUnknown));
                self.retry(window)?;
            }
            Err(ReadError::Invalid(error)) => {
                let reason = if error == TextError::TooLarge {
                    UnavailableReason::Oversized
                } else {
                    UnavailableReason::MalformedData
                };
                self.publish(ReadValue::Unavailable(reason));
            }
            Err(ReadError::InvalidAllocation) => {
                self.publish(ReadValue::Unavailable(UnavailableReason::MalformedData))
            }
            Err(ReadError::Changed) => unreachable!("handled above"),
        }
        Ok(())
    }

    fn notification(&mut self, window: &Window) -> Result<()> {
        let current = sequence();
        // Only suppress our own accepted write, identified by sequence AND
        // owner, never by a lasting text comparison. Future local copies win.
        if self.store.is_remote_sequence(current)
            && unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == window.hwnd)
        {
            return Ok(());
        }
        // Repeated notifications must not reset a pending version's retry budget.
        // Re-read settled observations even at the same sequence: the native
        // number is a change hint, not a unique or globally ordered operation ID.
        if current.is_some() && current == self.observation.sequence() && self.deadline.is_some() {
            return Ok(());
        }
        self.restart(current);
        self.attempt(window)
    }
}

pub fn request_shutdown() -> Result<()> {
    let thread = MESSAGE_THREAD.load(Ordering::SeqCst);
    if thread != 0 {
        unsafe {
            PostThreadMessageW(thread, SHUTDOWN, WPARAM(0), LPARAM(0))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writes::{QUEUE_CAPACITY, WriteClient};

    #[tokio::test]
    async fn shutdown_fails_active_and_queued_requests_without_native_access() {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let client = WriteClient::new(sender, || true);
        let mut pump = WritePump {
            receiver,
            active: None,
            deadline: None,
        };
        let active = tokio::spawn({
            let client = client.clone();
            async move { client.write("disposable active".into()).await }
        });
        let command = pump.receiver.recv().await.unwrap();
        pump.active = Some(PendingWrite {
            command,
            retries: 0,
        });
        let queued = tokio::spawn(async move { client.write("disposable queued".into()).await });
        tokio::task::yield_now().await;
        assert_eq!(pump.receiver.len(), 1);
        drop(pump);
        assert!(matches!(
            active.await.unwrap(),
            Err(WriteError::Unavailable)
        ));
        assert!(matches!(
            queued.await.unwrap(),
            Err(WriteError::Unavailable)
        ));
    }
}

pub fn wake_writes() -> bool {
    let thread = MESSAGE_THREAD.load(Ordering::SeqCst);
    thread != 0
        && unsafe { PostThreadMessageW(thread, WRITE_COMMAND, WPARAM(0), LPARAM(0)) }.is_ok()
}

struct OwnedGlobal(Option<HGLOBAL>);
impl Drop for OwnedGlobal {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            unsafe {
                let _ = GlobalFree(Some(handle));
            }
        }
    }
}

/// Runs exclusively on the window's message thread. Allocation/locking happens
/// before EmptyClipboard, so preparation failures never destroy old data.
fn write_text(
    window: &Window,
    capture: &mut Capture,
    command: &WriteCommand,
) -> std::result::Result<WriteReceipt, WriteError> {
    validate_text(&command.text)?;
    let units: Vec<u16> = command.text.encode_utf16().chain([0]).collect();
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2).map_err(|_| WriteError::Failed)?;
        let mut allocation = OwnedGlobal(Some(handle));
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            return Err(WriteError::Failed);
        }
        {
            let _lock = LockedGlobal(handle);
            std::ptr::copy_nonoverlapping(units.as_ptr(), pointer.cast::<u16>(), units.len());
        }
        OpenClipboard(Some(window.hwnd)).map_err(|_| WriteError::Busy)?;
        let clipboard = OpenClipboardGuard;
        // Last cancellable point before the native commit. Timeouts after this
        // point cannot undo an accepted SetClipboardData operation.
        if command.expired() {
            return Err(WriteError::Timeout);
        }
        window.cancel_retry();
        capture.deadline = None;
        capture.restart(sequence());
        if EmptyClipboard().is_err() {
            capture.publish(ReadValue::Unavailable(UnavailableReason::WriteFailed));
            return Err(WriteError::Failed);
        }
        if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(handle.0))).is_err() {
            capture.publish(ReadValue::Unavailable(UnavailableReason::WriteFailed));
            return Err(WriteError::Failed);
        }
        // Windows owns it now. Never free it, even if later bookkeeping fails.
        allocation.0 = None;
        let accepted_sequence = sequence();
        drop(clipboard);
        let current = sequence();
        let still_owner = GetClipboardOwner().is_ok_and(|owner| owner == window.hwnd);
        let observed = if still_owner {
            current
        } else {
            accepted_sequence
        };
        capture.observation = capture.store.begin_remote_observation(observed);
        let _ = capture.store.complete(
            &mut capture.observation,
            ReadValue::Text(command.text.clone()),
        );
        let operation = capture.store.snapshot().operation_id.observation;
        if !still_owner {
            // Another application replaced it after acceptance. Never label
            // that newer sequence with our text or suppress its notification.
            capture.restart(current);
        }
        Ok(WriteReceipt {
            operation,
            sequence: observed,
        })
    }
}

struct PendingWrite {
    command: WriteCommand,
    retries: usize,
}
struct WritePump {
    receiver: mpsc::Receiver<WriteCommand>,
    active: Option<PendingWrite>,
    deadline: Option<Instant>,
}
impl Drop for WritePump {
    fn drop(&mut self) {
        self.receiver.close();
        if let Some(active) = self.active.take() {
            let _ = active.command.ack.send(Err(WriteError::Unavailable));
        }
        while let Ok(command) = self.receiver.try_recv() {
            let _ = command.ack.send(Err(WriteError::Unavailable));
        }
    }
}
impl WritePump {
    fn step(&mut self, window: &Window, capture: &mut Capture) -> Result<()> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() < deadline)
        {
            return Ok(());
        }
        unsafe {
            let _ = KillTimer(Some(window.hwnd), WRITE_TIMER);
        }
        self.deadline = None;
        let mut pending = match self.active.take() {
            Some(active) => active,
            None => match self.receiver.try_recv() {
                Ok(command) => PendingWrite {
                    command,
                    retries: 0,
                },
                Err(_) => return Ok(()),
            },
        };
        let result = if pending.command.expired() {
            Err(WriteError::Timeout)
        } else {
            write_text(window, capture, &pending.command)
        };
        if matches!(result, Err(WriteError::Busy)) && !pending.command.expired() {
            if let Some(&delay) = BACKOFF_MS.get(pending.retries) {
                if unsafe { SetTimer(Some(window.hwnd), WRITE_TIMER, delay, None) } == 0 {
                    let _ = pending.command.ack.send(Err(WriteError::Unavailable));
                    return Err(Error::from_thread());
                }
                self.deadline = Some(Instant::now() + Duration::from_millis(delay.into()));
                pending.retries += 1;
                self.active = Some(pending);
                return Ok(());
            }
        }
        let _ = pending.command.ack.send(result);
        // Fairness: at most one write attempt per dispatched message.
        if !self.receiver.is_empty() {
            let _ = wake_writes();
        }
        Ok(())
    }
}

pub fn run(
    store: ClipboardStore,
    debug: bool,
    writes: mpsc::Receiver<WriteCommand>,
    ready: impl FnOnce(),
) -> Result<()> {
    let window = Window::new()?;
    let console = ConsoleHandler::new()?;
    println!("WiniOS clipboard watcher started. Press Ctrl+C to stop.");
    if debug {
        eprintln!("Clipboard diagnostics enabled (metadata only; no text or request bodies).");
    }
    // Register BEFORE observing startup state. Changes during this read remain
    // queued; matching sequence notifications are harmless.
    let mut capture = Capture::new(store, debug);
    let mut writer = WritePump {
        receiver: writes,
        active: None,
        deadline: None,
    };
    capture.attempt(&window)?;
    ready();
    let mut msg = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
        if result == -1 {
            return Err(Error::from_thread());
        }
        if result == 0 || msg.message == SHUTDOWN {
            break;
        }
        let update = msg.hwnd == window.hwnd && msg.message == WM_CLIPBOARDUPDATE;
        let timer =
            msg.hwnd == window.hwnd && msg.message == WM_TIMER && msg.wParam.0 == RETRY_TIMER;
        let write_timer =
            msg.hwnd == window.hwnd && msg.message == WM_TIMER && msg.wParam.0 == WRITE_TIMER;
        if msg.message == WRITE_COMMAND || write_timer {
            writer.step(&window, &mut capture)?;
        } else if update {
            capture.notification(&window)?;
        } else if timer {
            // KillTimer does not purge queued WM_TIMER messages. Ignore early
            // stale messages without canceling the current operation's timer.
            if capture
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                capture.attempt(&window)?;
            }
        } else {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    drop(writer);
    drop(capture);
    drop(console);
    drop(window);
    println!("WiniOS clipboard watcher stopped.");
    Ok(())
}
