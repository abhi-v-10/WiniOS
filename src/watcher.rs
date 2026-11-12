//! All Win32 calls and clipboard-owned pointers stay on the message thread.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
const IMAGE_READY: u32 = WM_APP + 3;
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;
const BACKOFF_MS: [u32; 6] = [20, 40, 80, 160, 320, 640];
static MESSAGE_THREAD: AtomicU32 = AtomicU32::new(0);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn console_handler(event: u32) -> BOOL {
    if event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT {
        SHUTTING_DOWN.store(true, Ordering::SeqCst);
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
        if let Err(error) = unsafe { SetConsoleCtrlHandler(Some(console_handler), true) } {
            return Err(error);
        }
        // Launchers can pass down an inherited "ignore Ctrl+C" attribute.
        // Installing a callback alone does not clear it. Enable delivery only
        // after our handler is ready.
        if let Err(error) = unsafe { SetConsoleCtrlHandler(None, false) } {
            unsafe {
                let _ = SetConsoleCtrlHandler(Some(console_handler), false);
            }
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
    }
}

struct MessageThread;
impl MessageThread {
    fn new() -> Self {
        SHUTTING_DOWN.store(false, Ordering::SeqCst);
        MESSAGE_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
        Self
    }
}
impl Drop for MessageThread {
    fn drop(&mut self) {
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

#[derive(Debug)]
enum ReadError {
    Temporary(Error),
    Invalid(TextError),
    InvalidAllocation,
    SequenceUnknown,
    Changed,
    ImageBusy,
    ImageTooLarge,
}
enum NativeValue {
    Value(ReadValue),
    Image(crate::image::Source),
}
fn preferred_native_dib() -> Option<u32> {
    // EnumClipboardFormats lists a published format before its synthesized
    // conversions. Preserve explicit DIBV5 alpha. Bitmap-first publishers use
    // GetDIBits: cross-process synthesized DIB handles are not always available.
    let mut format = 0;
    for _ in 0..256 {
        format = unsafe { EnumClipboardFormats(format) };
        match format {
            0 => break,
            2 => return Some(2),
            CF_DIB | CF_DIBV5 => return Some(format),
            _ => {}
        }
    }
    None
}

/// The bitmap belongs to Windows. Never GlobalLock, select or delete its handle.
/// Ask GDI for bounded opaque RGB pixels, with a scoped common screen DC.
fn copy_bitmap(handle: HANDLE) -> std::result::Result<Vec<u8>, ReadError> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let bitmap = HBITMAP(handle.0);
        let mut object = BITMAP::default();
        let got = GetObjectW(
            bitmap.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&mut object as *mut BITMAP).cast()),
        );
        if got != std::mem::size_of::<BITMAP>() as i32 {
            return Err(ReadError::InvalidAllocation);
        }
        if object.bmWidth <= 0 || object.bmHeight <= 0 {
            return Err(ReadError::InvalidAllocation);
        }
        let bytes = crate::image::dimensions(object.bmWidth as u32, object.bmHeight as u32)
            .map_err(|_| ReadError::ImageTooLarge)?;
        let dc = GetDC(None);
        if dc.is_invalid() {
            return Err(ReadError::Temporary(Error::from_thread()));
        }
        struct Dc(HDC);
        impl Drop for Dc {
            fn drop(&mut self) {
                unsafe {
                    ReleaseDC(None, self.0);
                }
            }
        }
        let _dc = Dc(dc);
        #[repr(C)]
        struct RgbInfo {
            header: BITMAPINFOHEADER,
            colors: [RGBQUAD; 256],
        }
        let mut info = RgbInfo {
            header: BITMAPINFOHEADER {
                biSize: 40,
                biWidth: object.bmWidth,
                biHeight: object.bmHeight,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            colors: [RGBQUAD::default(); 256],
        };
        let mut output = vec![0u8; 40 + bytes];
        if GetDIBits(
            dc,
            bitmap,
            0,
            object.bmHeight as u32,
            Some(output.as_mut_ptr().add(40).cast()),
            (&mut info as *mut RgbInfo).cast(),
            DIB_RGB_COLORS,
        ) != object.bmHeight
        {
            return Err(ReadError::Temporary(Error::from_thread()));
        }
        if info.header.biWidth != object.bmWidth
            || info.header.biHeight != object.bmHeight
            || info.header.biBitCount != 32
            || info.header.biCompression != BI_RGB.0
            || info.header.biPlanes != 1
            || info.header.biClrUsed != 0
        {
            return Err(ReadError::InvalidAllocation);
        }
        for (offset, value) in [
            (0, 40u32),
            (4, object.bmWidth as u32),
            (8, object.bmHeight as u32),
            (20, bytes as u32),
        ] {
            output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        output[12..14].copy_from_slice(&1u16.to_le_bytes());
        output[14..16].copy_from_slice(&32u16.to_le_bytes());
        Ok(output)
    }
}

pub fn sequence() -> Option<u32> {
    match unsafe { GetClipboardSequenceNumber() } {
        0 => None,
        value => Some(value),
    }
}

fn private_format() -> Result<u32> {
    let format =
        unsafe { RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")) };
    if format == 0 {
        Err(Error::from_thread())
    } else {
        Ok(format)
    }
}

fn read_native(
    hwnd: HWND,
    expected: Option<u32>,
    options: crate::image::Options,
    image_available: bool,
) -> std::result::Result<NativeValue, ReadError> {
    // SAFETY: clipboard is held open until after the global lock guard drops.
    // Its handle belongs to Windows: never GlobalFree it or retain its pointer.
    unsafe {
        OpenClipboard(Some(hwnd)).map_err(ReadError::Temporary)?;
        let _clipboard = OpenClipboardGuard;
        let before = sequence().ok_or(ReadError::SequenceUnknown)?;
        if Some(before) != expected {
            return Err(ReadError::Changed);
        }
        let private = private_format().map_err(ReadError::Temporary)?;
        let value = if IsClipboardFormatAvailable(private).is_ok() {
            // Honor the privacy marker even after an agent restart. Never lock
            // or decode private CF_UNICODETEXT into the shared store/baseline.
            ReadValue::Unavailable(UnavailableReason::PrivateContent)
        } else if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            let png = RegisterClipboardFormatW(w!("PNG"));
            let format = if options.enabled && png != 0 && IsClipboardFormatAvailable(png).is_ok() {
                Some((png, true))
            } else if options.enabled {
                preferred_native_dib().map(|format| (format, false))
            } else {
                None
            };
            if let Some((format, is_png)) = format {
                if !image_available {
                    return Err(ReadError::ImageBusy);
                }
                if format == 2 {
                    let bitmap = GetClipboardData(format).map_err(ReadError::Temporary)?;
                    let bytes = copy_bitmap(bitmap)?;
                    if sequence() != Some(before) {
                        return Err(ReadError::Changed);
                    }
                    return Ok(NativeValue::Image(crate::image::Source::Dib(bytes)));
                }
                let handle = HGLOBAL(GetClipboardData(format).map_err(ReadError::Temporary)?.0);
                let size = GlobalSize(handle);
                let limit = if is_png {
                    options.max_bytes() + 4096
                } else {
                    crate::image::MAX_DIB_BYTES
                };
                if size == 0 {
                    return Err(ReadError::InvalidAllocation);
                }
                if size > limit {
                    return Err(ReadError::ImageTooLarge);
                }
                let pointer = GlobalLock(handle);
                if pointer.is_null() {
                    return Err(ReadError::Temporary(Error::from_thread()));
                }
                let bytes = {
                    let _lock = LockedGlobal(handle);
                    let source = std::slice::from_raw_parts(pointer.cast::<u8>(), size);
                    let count = if is_png {
                        crate::image::native_png_length(source)
                            .map_err(|_| ReadError::InvalidAllocation)?
                    } else {
                        size
                    };
                    if is_png && count > options.max_bytes() {
                        return Err(ReadError::ImageTooLarge);
                    }
                    source[..count].to_vec()
                };
                if sequence() != Some(before) {
                    return Err(ReadError::Changed);
                }
                return Ok(NativeValue::Image(if is_png {
                    crate::image::Source::Png(bytes)
                } else {
                    crate::image::Source::Dib(bytes)
                }));
            }
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
        Ok(NativeValue::Value(value))
    }
}
#[cfg(test)]
fn read_text(hwnd: HWND, expected: Option<u32>) -> std::result::Result<ReadValue, ReadError> {
    match read_native(hwnd, expected, crate::image::Options::default(), false)? {
        NativeValue::Value(value) => Ok(value),
        NativeValue::Image(_) => unreachable!(),
    }
}

struct ImageWork {
    source: crate::image::Source,
    observation: Observation,
    budget: crate::image::Budget,
}
struct ImageResult {
    observation: Observation,
    result:
        std::result::Result<std::sync::Arc<crate::image::ClipboardImage>, crate::image::ImageError>,
}
struct ImageWorker {
    sender: Option<std::sync::mpsc::SyncSender<ImageWork>>,
    results: std::sync::mpsc::Receiver<ImageResult>,
    handle: Option<std::thread::JoinHandle<()>>,
    budget: Option<crate::image::Budget>,
}
impl ImageWorker {
    fn new(options: crate::image::Options) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<ImageWork>(1);
        let (result_tx, results) = std::sync::mpsc::sync_channel(1);
        let thread = unsafe { GetCurrentThreadId() };
        let handle = std::thread::Builder::new()
            .name("winios-image-codec".into())
            .spawn(move || {
                while let Ok(work) = rx.recv() {
                    let result = crate::image::convert(work.source, options, &work.budget);
                    if result_tx
                        .send(ImageResult {
                            observation: work.observation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    unsafe {
                        let _ = PostThreadMessageW(thread, IMAGE_READY, WPARAM(0), LPARAM(0));
                    }
                }
            })
            .map_err(|_| Error::from_thread())?;
        Ok(Self {
            sender: Some(tx),
            results,
            handle: Some(handle),
            budget: None,
        })
    }
    fn submit(&mut self, source: crate::image::Source, observation: Observation) -> bool {
        if self.budget.is_some() {
            return false;
        }
        let budget = crate::image::Budget::new();
        if self.sender.as_ref().is_some_and(|sender| {
            sender
                .try_send(ImageWork {
                    source,
                    observation,
                    budget: budget.clone(),
                })
                .is_ok()
        }) {
            self.budget = Some(budget);
            true
        } else {
            false
        }
    }
}
impl Drop for ImageWorker {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            budget.cancel();
        }
        self.sender.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct Capture {
    store: ClipboardStore,
    observation: Observation,
    retries: usize,
    deadline: Option<Instant>,
    debug: bool,
    images: Option<ImageWorker>,
    waiting_image: bool,
}

impl Capture {
    fn new(store: ClipboardStore, debug: bool) -> Result<Self> {
        let images = if store.image_options().enabled {
            Some(ImageWorker::new(store.image_options())?)
        } else {
            None
        };
        let observation = store.begin_observation(sequence());
        Ok(Self {
            store,
            observation,
            retries: 0,
            deadline: None,
            debug,
            images,
            waiting_image: false,
        })
    }

    fn restart(&mut self, sequence: Option<u32>) {
        if let Some(budget) = self
            .images
            .as_ref()
            .and_then(|worker| worker.budget.as_ref())
        {
            budget.cancel();
        }
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
                    ClipboardState::Image(image) => format!(
                        "clipboard image updated | {} x {} | {} PNG bytes",
                        image.width,
                        image.height,
                        image.len()
                    ),
                    ClipboardState::Empty => "clipboard empty".into(),
                    ClipboardState::Unavailable(reason) => {
                        format!("clipboard unavailable | {reason:?}")
                    }
                };
                crate::diagnostics::info!(
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
            Err(error) => crate::diagnostics::error!("Skipped clipboard: {error}"),
        }
    }

    fn retry(&mut self, window: &Window) -> Result<()> {
        if let Some(&delay) = BACKOFF_MS.get(self.retries) {
            window.retry_after(delay)?;
            self.deadline = Some(Instant::now() + Duration::from_millis(delay.into()));
            self.retries += 1;
        } else {
            self.waiting_image = false;
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
        self.waiting_image = false;
        let result = read_native(
            window.hwnd,
            self.observation.sequence(),
            self.store.image_options(),
            self.images
                .as_ref()
                .is_some_and(|worker| worker.budget.is_none()),
        );
        let after = sequence();
        // Covers changes both while the clipboard was open and after it closed.
        if after != self.observation.sequence() || matches!(result, Err(ReadError::Changed)) {
            self.restart(after);
            self.publish(ReadValue::Unavailable(UnavailableReason::ChangedDuringRead));
            return self.retry(window);
        }
        match result {
            Ok(NativeValue::Value(value)) => self.publish(value),
            Ok(NativeValue::Image(source)) => {
                if !self
                    .images
                    .as_mut()
                    .is_some_and(|worker| worker.submit(source, self.observation.clone()))
                {
                    self.waiting_image = true;
                    self.publish(ReadValue::Unavailable(UnavailableReason::TemporaryFailure));
                    self.retry(window)?;
                }
            }
            Err(ReadError::ImageBusy) => {
                self.waiting_image = true;
                self.publish(ReadValue::Unavailable(UnavailableReason::TemporaryFailure));
                self.retry(window)?;
            }
            Err(ReadError::ImageTooLarge) => {
                self.publish(ReadValue::Unavailable(UnavailableReason::Oversized))
            }
            Err(ReadError::Temporary(error)) => {
                self.publish(ReadValue::Unavailable(UnavailableReason::TemporaryFailure));
                if self.debug {
                    crate::diagnostics::error!("Clipboard access failed: {error}");
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

    fn image_ready(&mut self, window: &Window) -> Result<()> {
        let result = self.images.as_mut().and_then(|worker| {
            let result = worker.results.try_recv().ok()?;
            worker.budget = None;
            Some(result)
        });
        if let Some(result) = result {
            if result.observation == self.observation
                && result.observation.sequence() == sequence()
                && !SHUTTING_DOWN.load(Ordering::SeqCst)
            {
                self.publish(match result.result {
                    Ok(image) => ReadValue::Image(image),
                    Err(crate::image::ImageError::TooLarge) => {
                        ReadValue::Unavailable(UnavailableReason::Oversized)
                    }
                    Err(crate::image::ImageError::Invalid) => {
                        ReadValue::Unavailable(UnavailableReason::MalformedData)
                    }
                    Err(crate::image::ImageError::Timeout) => {
                        ReadValue::Unavailable(UnavailableReason::RetriesExhausted)
                    }
                });
            }
            if self.waiting_image {
                self.attempt(window)?;
            }
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
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
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

    // External clipboard monitors can briefly take the OS lock even with the
    // agent stopped. Retry access errors only; never retry a privacy mismatch.
    fn fixture_read(hwnd: HWND) -> std::result::Result<ReadValue, ReadError> {
        for attempt in 0..=BACKOFF_MS.len() {
            match read_text(hwnd, sequence()) {
                Err(error @ (ReadError::Temporary(_) | ReadError::Changed))
                    if attempt < BACKOFF_MS.len() =>
                {
                    eprintln!("Disposable fixture access retry: {error:?}");
                    std::thread::sleep(Duration::from_millis(BACKOFF_MS[attempt].into()));
                }
                result => return result,
            }
        }
        unreachable!()
    }
    fn fixture_write(
        window: &Window,
        capture: &mut Capture,
        command: &WriteCommand,
    ) -> std::result::Result<WriteReceipt, WriteError> {
        for attempt in 0..=BACKOFF_MS.len() {
            match write_text(window, capture, command) {
                Err(WriteError::Busy) if attempt < BACKOFF_MS.len() => {
                    std::thread::sleep(Duration::from_millis(BACKOFF_MS[attempt].into()))
                }
                result => return result,
            }
        }
        unreachable!()
    }

    #[test]
    fn retry_budget_exhaustion_invalidates_and_resets_dedup_without_sleep_or_clipboard_access() {
        let window = Window::new().unwrap();
        let store = ClipboardStore::default();
        let mut original = store.begin_observation(Some(7));
        store
            .complete(
                &mut original,
                ReadValue::Text("disposable retry baseline".into()),
            )
            .unwrap();
        let mut capture = Capture::new(store.clone(), false).unwrap();
        for expected in 1..=BACKOFF_MS.len() {
            window.cancel_retry();
            capture.deadline = None;
            capture.retry(&window).unwrap();
            assert_eq!(capture.retries, expected);
            assert!(capture.deadline.is_some());
        }
        window.cancel_retry();
        capture.deadline = None;
        capture.retry(&window).unwrap();
        assert!(capture.deadline.is_none());
        assert_eq!(
            store.snapshot().state,
            ClipboardState::Unavailable(UnavailableReason::RetriesExhausted)
        );
        assert_eq!(
            store
                .complete(
                    &mut capture.observation,
                    ReadValue::Text("disposable retry baseline".into())
                )
                .unwrap(),
            Update::Changed
        );
    }

    #[test]
    #[ignore = "Writes disposable clipboard text; stop the installed agent first. Never reads the real bearer credential."]
    fn native_private_setup_export_is_unreadable_and_survives_owner_restart() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (sender,mut receiver)=mpsc::channel(QUEUE_CAPACITY);
            let client=WriteClient::new(sender,||true);
            let store=ClipboardStore::default();
            let mut old=store.begin_observation(Some(1));store.complete(&mut old,ReadValue::Text("old disposable public text".into())).unwrap();
            let window=Window::new().unwrap();let mut capture=Capture::new(store.clone(),false).unwrap();
            let task=tokio::spawn({let client=client.clone();async move{client.copy_private("disposable private setup fixture".into()).await}});
            let command=receiver.recv().await.unwrap();let receipt=fixture_write(&window,&mut capture,&command).unwrap();
            assert_eq!(store.snapshot().origin,crate::clipboard::Origin::Setup);
            assert_eq!(store.snapshot().state,ClipboardState::Unavailable(UnavailableReason::PrivateContent));
            assert!(matches!(fixture_read(window.hwnd),Ok(ReadValue::Unavailable(UnavailableReason::PrivateContent))));
            command.ack.send(Ok(receipt)).ok();assert!(task.await.unwrap().is_ok());
            drop(window);
            let window=Window::new().unwrap();
            // The marker is owned by Windows, not a transient process flag.
            assert!(matches!(fixture_read(window.hwnd),Ok(ReadValue::Unavailable(UnavailableReason::PrivateContent))));
            let task=tokio::spawn(async move{client.write("disposable public recovery".into()).await});
            let command=receiver.recv().await.unwrap();let receipt=fixture_write(&window,&mut capture,&command).unwrap();command.ack.send(Ok(receipt)).ok();assert!(task.await.unwrap().is_ok());
            assert!(matches!(fixture_read(window.hwnd),Ok(ReadValue::Text(text)) if text=="disposable public recovery"));
            assert!(matches!(store.snapshot().state,ClipboardState::Available(_)));
        });
    }

    #[test]
    #[ignore = "Writes disposable image/text fixtures. Stop the installed agent first; no real bearer token or user image is read."]
    fn native_image_startup_formats_remote_suppression_and_stale_conversion() {
        let options = crate::image::Options {
            enabled: true,
            ..Default::default()
        };
        let budget = crate::image::Budget::new();
        let fixture = crate::image::fixture();
        let dib = crate::image::dib(&fixture, true, &budget).unwrap();
        let window = Window::new().unwrap();
        let set_native = |format, data: &[u8]| {
            let mut memory = image_allocation(data).unwrap();
            unsafe {
                OpenClipboard(Some(window.hwnd)).unwrap();
                let _clip = OpenClipboardGuard;
                EmptyClipboard().unwrap();
                SetClipboardData(format, Some(HANDLE(memory.0.unwrap().0))).unwrap();
                memory.0 = None;
            }
        };
        set_native(CF_DIBV5, &dib);
        let store = ClipboardStore::with_images(options);
        let mut capture = Capture::new(store.clone(), false).unwrap();
        let wait_image = |capture: &mut Capture| {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                if capture
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    capture.attempt(&window).unwrap();
                }
                capture.image_ready(&window).unwrap();
                if let ClipboardState::Image(image) = store.snapshot().state {
                    return image;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            panic!("native image fixture did not become available");
        };
        capture.attempt(&window).unwrap();
        let image = wait_image(&mut capture);
        assert_eq!(
            crate::image::decode_png(&image.png(), options, &budget)
                .unwrap()
                .rgba,
            fixture.rgba
        );
        assert_eq!(
            store.snapshot().origin,
            crate::clipboard::Origin::WindowsClipboard
        );
        let mut padded = image.png().to_vec();
        padded.extend_from_slice(&[0; 32]);
        let png_format = unsafe { RegisterClipboardFormatW(w!("PNG")) };
        set_native(png_format, &padded);
        capture.notification(&window).unwrap();
        wait_image(&mut capture);
        // Bitmap-only publishers rely on Windows' synthesized packed DIB.
        unsafe {
            let bitmap = windows::Win32::Graphics::Gdi::CreateBitmap(
                2,
                2,
                1,
                32,
                Some([0u8; 16].as_ptr().cast()),
            );
            assert!(!bitmap.is_invalid());
            OpenClipboard(Some(window.hwnd)).unwrap();
            let _clip = OpenClipboardGuard;
            EmptyClipboard().unwrap();
            if SetClipboardData(2, Some(HANDLE(bitmap.0))).is_err() {
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(bitmap.into());
                panic!("Bitmap fixture ownership transfer failed");
            }
        }
        for _ in 0..3 {
            match read_native(window.hwnd, sequence(), options, true) {
                Ok(NativeValue::Image(crate::image::Source::Dib(bytes))) => {
                    assert!(
                        crate::image::decode_dib(&bytes, &crate::image::Budget::new()).is_ok(),
                        "Synthesized DIB rejected: bytes={} header={} bpp={} compression={} declared={} colors={}",
                        bytes.len(),
                        u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                        u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
                        u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
                        u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
                        u32::from_le_bytes(bytes[32..36].try_into().unwrap())
                    );
                    break;
                }
                Err(ReadError::Changed) => continue,
                _ => panic!("Bitmap fixture was not captured as DIB"),
            }
        }
        capture.notification(&window).unwrap();
        wait_image(&mut capture);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, mut receiver) = mpsc::channel(crate::writes::QUEUE_CAPACITY);
        let client = crate::writes::WriteClient::new(sender, || true);
        runtime.block_on(async {
            let prepared=crate::image::prepare_png(&image.png(),options,&crate::image::Budget::new()).unwrap();
            let lease=store.image_uploads.clone().try_acquire_owned().unwrap();
            let task=tokio::spawn({let client=client.clone();async move{client.write_image(prepared,lease).await}});
            let command=receiver.recv().await.unwrap();let receipt=write_text(&window,&mut capture,&command).unwrap();command.ack.send(Ok(receipt)).ok();assert!(task.await.unwrap().is_ok());
            assert_eq!(store.snapshot().origin,crate::clipboard::Origin::Iphone);
            for format in [CF_DIBV5,CF_DIB,png_format]{assert!(unsafe{IsClipboardFormatAvailable(format)}.is_ok());}
            let operation=store.snapshot().operation_id;capture.notification(&window).unwrap();assert_eq!(store.snapshot().operation_id,operation);
            // Start an old conversion, then supersede it with an accepted text write.
            set_native(CF_DIBV5,&dib);capture.notification(&window).unwrap();
            let task=tokio::spawn({let client=client.clone();async move{client.write("Unicode 😀 café\r\n".into()).await}});
            let command=receiver.recv().await.unwrap();let receipt=write_text(&window,&mut capture,&command).unwrap();command.ack.send(Ok(receipt)).ok();assert!(task.await.unwrap().is_ok());
            std::thread::sleep(Duration::from_millis(30));capture.image_ready(&window).unwrap();
            assert!(matches!(store.snapshot().state,ClipboardState::Available(value) if value.text=="Unicode 😀 café\r\n"));
        });
        drop(capture);
        drop(window);
    }

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
        let image_client = client.clone();
        let options = crate::image::Options {
            enabled: true,
            ..Default::default()
        };
        let png = crate::image::encode_png(
            &crate::image::fixture(),
            options,
            &crate::image::Budget::new(),
        )
        .unwrap();
        let prepared =
            crate::image::prepare_png(&png.png(), options, &crate::image::Budget::new()).unwrap();
        let lease = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let permit = lease.clone().try_acquire_owned().unwrap();
        let image_queued =
            tokio::spawn(async move { image_client.write_image(prepared, permit).await });
        let queued = tokio::spawn(async move { client.write("disposable queued".into()).await });
        tokio::task::yield_now().await;
        assert_eq!(pump.receiver.len(), 2);
        drop(pump);
        assert!(matches!(
            active.await.unwrap(),
            Err(WriteError::Unavailable)
        ));
        assert!(matches!(
            queued.await.unwrap(),
            Err(WriteError::Unavailable)
        ));
        assert!(matches!(
            image_queued.await.unwrap(),
            Err(WriteError::Unavailable)
        ));
        assert_eq!(lease.available_permits(), 1);
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
    if let Some(image) = &command.image {
        return write_image(window, capture, command, image);
    }
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
        // Prepare every allocation BEFORE clearing. Publish the exclusion marker
        // before text, so a marker failure can never leave an exposed token.
        let marker = if command.private {
            let format = private_format().map_err(|_| WriteError::Failed)?;
            let handle = GlobalAlloc(GMEM_MOVEABLE, 4).map_err(|_| WriteError::Failed)?;
            let memory = OwnedGlobal(Some(handle));
            let pointer = GlobalLock(handle);
            if pointer.is_null() {
                return Err(WriteError::Failed);
            }
            {
                let _lock = LockedGlobal(handle);
                pointer.cast::<u32>().write(1);
            }
            Some((format, memory))
        } else {
            None
        };
        OpenClipboard(Some(window.hwnd)).map_err(|_| WriteError::Busy)?;
        let clipboard = OpenClipboardGuard;
        // Last cancellable point before the native commit. Timeouts after this
        // point cannot undo an accepted SetClipboardData operation.
        if command.expired() || SHUTTING_DOWN.load(Ordering::SeqCst) {
            return Err(WriteError::Timeout);
        }
        window.cancel_retry();
        capture.deadline = None;
        capture.restart(sequence());
        if EmptyClipboard().is_err() {
            capture.publish(ReadValue::Unavailable(UnavailableReason::WriteFailed));
            return Err(WriteError::Failed);
        }
        if let Some((format, mut memory)) = marker {
            if SetClipboardData(format, Some(HANDLE(memory.0.unwrap().0))).is_err() {
                capture.publish(ReadValue::Unavailable(UnavailableReason::WriteFailed));
                return Err(WriteError::Failed);
            }
            memory.0 = None; // Windows owns the marker now.
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
        capture.observation = if command.private {
            capture.store.begin_setup_observation(observed)
        } else {
            capture.store.begin_remote_observation(observed)
        };
        let _ = capture.store.complete(
            &mut capture.observation,
            if command.private {
                ReadValue::Unavailable(UnavailableReason::PrivateContent)
            } else {
                ReadValue::Text(command.text.clone())
            },
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

fn image_allocation(bytes: &[u8]) -> std::result::Result<OwnedGlobal, WriteError> {
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|_| WriteError::Failed)?;
        let memory = OwnedGlobal(Some(handle));
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            return Err(WriteError::Failed);
        }
        let _lock = LockedGlobal(handle);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        Ok(memory)
    }
}
fn write_image(
    window: &Window,
    capture: &mut Capture,
    command: &WriteCommand,
    image: &crate::image::Prepared,
) -> std::result::Result<WriteReceipt, WriteError> {
    let options = capture.store.image_options();
    if !options.enabled {
        return Err(WriteError::Unavailable);
    }
    if image.image.len() > options.max_bytes()
        || image.dib_v5.len() > crate::image::MAX_DIB_BYTES
        || image.dib_24.len() > crate::image::MAX_DIB_BYTES
    {
        return Err(WriteError::TooLarge);
    }
    let png = image.image.png();
    let mut allocations = [
        image_allocation(&[0; 4])?,
        image_allocation(&[0; 4])?,
        image_allocation(&png)?,
        image_allocation(&image.dib_v5)?,
        image_allocation(&image.dib_24)?,
    ];
    unsafe {
        let png_format = RegisterClipboardFormatW(w!("PNG"));
        let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        let cloud = RegisterClipboardFormatW(w!("CanUploadToCloudClipboard"));
        if png_format == 0 || history == 0 || cloud == 0 {
            return Err(WriteError::Failed);
        }
        OpenClipboard(Some(window.hwnd)).map_err(|_| WriteError::Busy)?;
        let clipboard = OpenClipboardGuard;
        if command.expired() || SHUTTING_DOWN.load(Ordering::SeqCst) {
            return Err(WriteError::Timeout);
        }
        window.cancel_retry();
        capture.deadline = None;
        capture.restart(sequence());
        if EmptyClipboard().is_err() {
            capture.publish(ReadValue::Unavailable(UnavailableReason::WriteFailed));
            return Err(WriteError::Failed);
        }
        for (format, memory) in [history, cloud, png_format, CF_DIBV5, CF_DIB]
            .into_iter()
            .zip(&mut allocations)
        {
            if SetClipboardData(format, Some(HANDLE(memory.0.unwrap().0))).is_err() {
                capture.publish(ReadValue::Unavailable(UnavailableReason::WriteFailed));
                return Err(WriteError::Failed);
            }
            memory.0 = None; // Windows owns each accepted format independently.
        }
        let accepted = sequence();
        drop(clipboard);
        let current = sequence();
        let still_owner = GetClipboardOwner().is_ok_and(|owner| owner == window.hwnd);
        let observed = if still_owner { current } else { accepted };
        capture.observation = capture.store.begin_remote_observation(observed);
        let _ = capture.store.complete(
            &mut capture.observation,
            ReadValue::Image(image.image.clone()),
        );
        let operation = capture.store.snapshot().operation_id.observation;
        if !still_owner {
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
    console_mode: bool,
    writes: mpsc::Receiver<WriteCommand>,
    ready: impl FnOnce(),
) -> Result<()> {
    let window = Window::new()?;
    let registration = MessageThread::new();
    let console = if console_mode {
        Some(ConsoleHandler::new()?)
    } else {
        None
    };
    crate::diagnostics::info!("WiniOS clipboard watcher started. Press Ctrl+C to stop.");
    if debug {
        crate::diagnostics::error!(
            "Clipboard diagnostics enabled (metadata only; no text or request bodies)."
        );
    }
    // Register BEFORE observing startup state. Changes during this read remain
    // queued; matching sequence notifications are harmless.
    let mut capture = Capture::new(store, debug)?;
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
        if result == 0 || msg.message == SHUTDOWN || SHUTTING_DOWN.load(Ordering::SeqCst) {
            break;
        }
        let update = msg.hwnd == window.hwnd && msg.message == WM_CLIPBOARDUPDATE;
        let timer =
            msg.hwnd == window.hwnd && msg.message == WM_TIMER && msg.wParam.0 == RETRY_TIMER;
        let write_timer =
            msg.hwnd == window.hwnd && msg.message == WM_TIMER && msg.wParam.0 == WRITE_TIMER;
        if msg.message == IMAGE_READY {
            capture.image_ready(&window)?;
        } else if msg.message == WRITE_COMMAND || write_timer {
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
        // Drain a completed conversion after any native event too. A saturated
        // Win32 queue must not strand a result if its dedicated wake was lost.
        capture.image_ready(&window)?;
    }
    drop(writer);
    drop(capture);
    drop(console);
    drop(registration);
    drop(window);
    crate::diagnostics::info!("WiniOS clipboard watcher stopped.");
    Ok(())
}
