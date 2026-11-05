//! All Win32 calls and clipboard-owned pointers stay on the message thread.
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Error, Result, w};

use crate::clipboard::{
    ClipboardState, ClipboardStore, MAX_TEXT_BYTES, Observation, ReadValue, TextError,
    UnavailableReason, Update, decode_text,
};

const CF_UNICODETEXT: u32 = 13; // Standard Win32 clipboard format.
const RETRY_TIMER: usize = 1;
const SHUTDOWN: u32 = WM_APP + 1;
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

fn sequence() -> Option<u32> {
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
                if self.debug {
                    if let ClipboardState::Available(text) = snapshot.state {
                        eprintln!("clipboard text: {:?}", text.text);
                    }
                }
            }
            Ok(Update::Duplicate | Update::Superseded) => {}
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

pub fn run(store: ClipboardStore, debug: bool) -> Result<()> {
    let window = Window::new()?;
    let console = ConsoleHandler::new()?;
    println!("WiniOS clipboard watcher started. Press Ctrl+C to stop.");
    if debug {
        eprintln!("WARNING: clipboard content logging enabled.");
    }
    // Register BEFORE observing startup state. Changes during this read remain
    // queued; matching sequence notifications are harmless.
    let mut capture = Capture::new(store, debug);
    capture.attempt(&window)?;
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
        if update {
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
    drop(capture);
    drop(console);
    drop(window);
    println!("WiniOS clipboard watcher stopped.");
    Ok(())
}
