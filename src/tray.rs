//! Native notification-area UI. All shell/menu calls stay on this UI thread.
use crate::{
    app,
    config::Config,
    diagnostics, fluent, flyout, integration,
    lifecycle::{Event, Status, TrayModel},
    native::{AppError, SingleInstance},
    popup, preferences_window, settings, setup, wizard,
};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    thread::JoinHandle,
};
use tokio::sync::watch;
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

const CALLBACK: u32 = WM_APP + 40;
const STATUS_EVENT: u32 = WM_APP + 41;
const QUIT_EVENT: u32 = WM_APP + 42;
const SHOW_MENU: u32 = WM_APP + 43;
const MANUAL_LAUNCH: u32 = WM_APP + 50;
const QUIT: usize = 1001;
const RETRY: usize = 1002;
const PAUSE: usize = 1003;
const STATUS: usize = 1004;
const STARTUP: usize = 1005;
const SETUP: usize = 1006;
const ONBOARDING: usize = 1007;
const ICON_ID: u32 = 1;
// NIN_KEYSELECT is a C header macro, not projected as a windows-crate constant.
const KEY_SELECT: u32 = NIN_SELECT | NINF_KEY;
static MENU_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TASKBAR_MESSAGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static READD_ICON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SESSION_QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SETUP_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SETTINGS_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAUNCH_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn callback_event(lp: LPARAM) -> Option<u32> {
    let packed = lp.0 as u32;
    ((packed >> 16) == ICON_ID).then_some(packed & 0xffff)
}
fn anchor_point(packed: usize) -> POINT {
    POINT {
        x: packed as u16 as i16 as i32,
        y: (packed >> 16) as u16 as i16 as i32,
    }
}
fn post_menu(hwnd: HWND, coordinates: WPARAM, use_coordinates: bool) {
    use std::sync::atomic::Ordering;
    // No recursive menus if the Shell sends more callbacks in TrackPopupMenu's
    // nested message loop. WndProc only posts; it never blocks on a popup.
    if !MENU_PENDING.swap(true, Ordering::Relaxed) {
        unsafe {
            if PostMessageW(
                Some(hwnd),
                SHOW_MENU,
                coordinates,
                LPARAM(use_coordinates as isize),
            )
            .is_err()
            {
                MENU_PENDING.store(false, Ordering::Relaxed);
            }
        }
    }
}

pub fn show_error(message: &str) {
    let message = wide(message);
    MENU_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("WiniOS"),
            MB_OK | MB_ICONERROR,
        );
    }
    MENU_PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Local process-control command for rollback/automation when the tray is hidden.
/// This posts the same graceful UI close request; it never terminates a process.
pub fn request_quit() -> Result<(), AppError> {
    unsafe {
        let hwnd = FindWindowW(w!("WiniOSTrayWindow"), PCWSTR::null())?;
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    use std::sync::atomic::Ordering;
    if let Some(result) = unsafe { popup::handle_message(message, wp, lp) } {
        return result;
    }
    if message == CALLBACK {
        if let Some(event) = callback_event(lp) {
            if event == WM_CONTEXTMENU || event == NIN_SELECT || event == KEY_SELECT {
                // V4: LOWORD(lParam)=event, HIWORD(lParam)=icon ID. Coordinates
                // are documented for NIN_SELECT/KEYSELECT; for WM_CONTEXTMENU
                // use the icon rectangle instead of undefined wParam contents.
                post_menu(hwnd, wp, event != WM_CONTEXTMENU);
            }
        }
        return LRESULT(0);
    }
    if message == wizard::SHOW {
        SETUP_PENDING.store(true, Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(Some(hwnd), STATUS_EVENT, WPARAM(0), LPARAM(0));
        }
        return LRESULT(0);
    }
    if message == preferences_window::SHOW {
        SETTINGS_PENDING.store(true, Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(Some(hwnd), STATUS_EVENT, WPARAM(0), LPARAM(0));
        }
        return LRESULT(0);
    }
    if message == MANUAL_LAUNCH {
        LAUNCH_PENDING.store(true, Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(Some(hwnd), STATUS_EVENT, WPARAM(0), LPARAM(0));
        }
        return LRESULT(0);
    }
    if message == WM_CONTEXTMENU {
        let point = anchor_point(lp.0 as usize);
        post_menu(hwnd, WPARAM(lp.0 as usize), point.x != -1 || point.y != -1);
        return LRESULT(0);
    }
    if message != 0 && message == TASKBAR_MESSAGE.load(Ordering::Relaxed) {
        READD_ICON.store(true, Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(Some(hwnd), STATUS_EVENT, WPARAM(0), LPARAM(0));
        }
        return LRESULT(0);
    }
    if message == WM_CLOSE
        || message == WM_QUERYENDSESSION
        || (message == WM_ENDSESSION && wp.0 != 0)
    {
        SESSION_QUIT.store(true, Ordering::Relaxed);
        unsafe {
            // Close any owned status dialog and native menu so an external
            // graceful Quit is not stranded in their nested message loops.
            let _ = EndMenu();
            let popup = GetLastActivePopup(hwnd);
            if popup != hwnd && !popup.0.is_null() {
                let _ = PostMessageW(Some(popup), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            let _ = PostMessageW(Some(hwnd), QUIT_EVENT, WPARAM(0), LPARAM(0));
        }
        return LRESULT(1);
    }
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}

fn create_icon() -> windows::core::Result<HICON> {
    unsafe {
        if let Ok(icon) = LoadImageW(
            Some(HINSTANCE(GetModuleHandleW(None)?.0)),
            PCWSTR(1usize as *const u16),
            IMAGE_ICON,
            32,
            32,
            LR_DEFAULTCOLOR,
        ) {
            return Ok(HICON(icon.0));
        }
    }
    let mut pixels = crate::brand::pixels(32);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2); // CreateIcon expects BGRA, resource fallback stays RGBA.
    }
    let mask = [0u8; 128];
    unsafe { CreateIcon(None, 32, 32, 1, 32, mask.as_ptr(), pixels.as_ptr()) }
}

struct TrayWindow {
    hwnd: HWND,
    instance: HINSTANCE,
    icon: HICON,
    added: bool,
    taskbar_created: u32,
}
impl TrayWindow {
    fn anchor(&self, anchor: Option<POINT>) -> Result<POINT, AppError> {
        if let Some(point) = anchor.filter(|p| p.x != -1 || p.y != -1) {
            return Ok(point);
        }
        let identity = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        unsafe {
            if let Ok(rect) = Shell_NotifyIconGetRect(&identity) {
                Ok(POINT {
                    x: rect.left,
                    y: rect.bottom,
                })
            } else {
                let mut point = POINT::default();
                GetCursorPos(&mut point)?;
                Ok(point)
            }
        }
    }
    fn new() -> Result<Self, AppError> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None)?.0);
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("WiniOSTrayWindow"),
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(windows::core::Error::from_thread().into());
            }
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!("WiniOS"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let _ = UnregisterClassW(class.lpszClassName, Some(instance));
                    return Err(error.into());
                }
            };
            let icon = match create_icon() {
                Ok(icon) => icon,
                Err(error) => {
                    let _ = DestroyWindow(hwnd);
                    let _ = UnregisterClassW(class.lpszClassName, Some(instance));
                    return Err(error.into());
                }
            };
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(icon.0 as isize)),
            );
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(icon.0 as isize)),
            );
            let mut window = Self {
                hwnd,
                instance,
                icon,
                added: false,
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            };
            if window.taskbar_created == 0 {
                return Err(windows::core::Error::from_thread().into());
            }
            TASKBAR_MESSAGE.store(window.taskbar_created, std::sync::atomic::Ordering::Relaxed);
            READD_ICON.store(false, std::sync::atomic::Ordering::Relaxed);
            SESSION_QUIT.store(false, std::sync::atomic::Ordering::Relaxed);
            MENU_PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
            window.add_icon()?;
            Ok(window)
        }
    }
    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: CALLBACK,
            hIcon: self.icon,
            ..Default::default()
        }
    }
    fn add_icon(&mut self) -> Result<(), AppError> {
        let mut data = self.data();
        for (slot, unit) in data
            .szTip
            .iter_mut()
            .zip("WiniOS — Starting".encode_utf16())
        {
            *slot = unit;
        }
        unsafe {
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                return Err("Windows could not add the WiniOS tray icon.".into());
            }
            self.added = true;
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            if !Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool() {
                return Err("Windows could not configure the tray callback.".into());
            }
        }
        Ok(())
    }
    fn update(&self, model: &TrayModel) {
        let mut data = self.data();
        let tip = format!("WiniOS — {}", model.status_text());
        for (slot, unit) in data.szTip.iter_mut().take(127).zip(tip.encode_utf16()) {
            *slot = unit;
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
    fn update_notice(&self) {
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        for (dst, ch) in data
            .szInfoTitle
            .iter_mut()
            .take(63)
            .zip("WiniOS update available".encode_utf16())
        {
            *dst = ch;
        }
        for (dst, ch) in data
            .szInfo
            .iter_mut()
            .take(255)
            .zip("Open Settings → About to review the release.".encode_utf16())
        {
            *dst = ch;
        }
        data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
    fn menu(
        &self,
        model: &TrayModel,
        anchor: Option<POINT>,
        path: &std::path::Path,
        native: bool,
    ) -> Result<usize, AppError> {
        unsafe {
            let point = match anchor {
                Some(point) if point.x != -1 || point.y != -1 => point,
                _ => {
                    let identity = NOTIFYICONIDENTIFIER {
                        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                        hWnd: self.hwnd,
                        uID: ICON_ID,
                        ..Default::default()
                    };
                    match Shell_NotifyIconGetRect(&identity) {
                        Ok(rect) => POINT {
                            x: rect.left,
                            y: rect.bottom,
                        },
                        Err(_) => {
                            let mut point = POINT::default();
                            GetCursorPos(&mut point)?;
                            point
                        }
                    }
                }
            };
            let startup = integration::startup_info(path);
            let mut header = popup::Row::action(STATUS, "WiniOS", "", false);
            header.kind = popup::Kind::Header;
            header.secondary = model.status_text();
            let mut endpoint = popup::Row::action(1101, "HTTPS endpoint", "", true);
            endpoint.kind = popup::Kind::Endpoint;
            endpoint.secondary = model
                .endpoint
                .clone()
                .unwrap_or_else(|| "Waiting for a secure connection".into());
            let mut startup_row = popup::Row::action(
                STARTUP,
                "Start with &Windows",
                "",
                !startup.can_toggle || model.quitting,
            );
            startup_row.kind = popup::Kind::Checkbox;
            startup_row.checked = startup.registered;
            let mut footer = popup::Row::action(1103, "Local-first. Yours to control.", "", true);
            footer.kind = popup::Kind::Footer;
            if startup.registered || !startup.can_toggle {
                footer.label = startup.note.into();
            }
            let rows = vec![
                header,
                endpoint,
                popup::Row::separator(1102),
                popup::Row::action(
                    PAUSE,
                    if model.sharing.is_paused() {
                        "&Resume Clipboard Sharing"
                    } else {
                        "&Pause Clipboard Sharing"
                    },
                    if model.sharing.is_paused() {
                        "▷"
                    } else {
                        "Ⅱ"
                    },
                    model.quitting,
                ),
                popup::Row::action(
                    RETRY,
                    "Re&try Connection",
                    "↻",
                    model.retry_pending || model.quitting,
                ),
                startup_row,
                popup::Row::action(SETUP, "Open &Settings", "⚙", model.quitting),
                popup::Row::action(ONBOARDING, "iPhone Set&up…", "", model.quitting),
                popup::Row::separator(1104),
                popup::Row::action(QUIT, "&Quit WiniOS", "×", model.quitting),
                footer,
            ];
            let menu = popup::Popup::new(self.hwnd, point, rows, self.icon, native)?;
            let _ = SetForegroundWindow(self.hwnd);
            let selection = TrackPopupMenu(
                menu.menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            )
            .0 as usize;
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = Shell_NotifyIconW(NIM_SETFOCUS, &self.data());
            Ok(selection)
        }
    }
}
impl Drop for TrayWindow {
    fn drop(&mut self) {
        unsafe {
            if self.added {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyIcon(self.icon);
            let _ = UnregisterClassW(w!("WiniOSTrayWindow"), Some(self.instance));
        }
    }
}

fn notify(sender: &mpsc::Sender<Event>, hwnd: isize, event: Event) {
    let _ = sender.send(event);
    unsafe {
        let _ = PostMessageW(
            Some(HWND(hwnd as *mut _)),
            STATUS_EVENT,
            WPARAM(0),
            LPARAM(0),
        );
    }
}
fn fluent_rows(model: &TrayModel, startup: Option<integration::StartupInfo>) -> Vec<popup::Row> {
    let mut header = popup::Row::action(STATUS, "WiniOS", "", false);
    header.kind = popup::Kind::Header;
    header.secondary = model.status_text();
    let mut endpoint = popup::Row::action(1101, "PRIVATE HTTPS", "", true);
    endpoint.kind = popup::Kind::Endpoint;
    endpoint.secondary = model
        .endpoint
        .clone()
        .unwrap_or_else(|| "No verified endpoint".into());
    let mut startup_row = popup::Row::action(
        STARTUP,
        if startup.is_some() {
            "Start with &Windows"
        } else {
            "Checking startup registration…"
        },
        "",
        !startup.as_ref().is_some_and(|s| s.can_toggle) || model.quitting,
    );
    startup_row.kind = popup::Kind::Checkbox;
    startup_row.checked = startup.is_some_and(|s| s.registered);
    vec![
        header,
        endpoint,
        popup::Row::separator(1102),
        popup::Row::action(
            PAUSE,
            if model.sharing.is_paused() {
                "&Resume clipboard sharing"
            } else {
                "&Pause clipboard sharing"
            },
            "",
            model.quitting,
        ),
        popup::Row::action(
            RETRY,
            "&Retry connection",
            "",
            model.retry_pending || model.quitting,
        ),
        popup::Row::action(SETUP, "Open &Settings", "", model.quitting),
        startup_row,
        popup::Row::separator(1104),
        popup::Row::action(QUIT, "&Quit WiniOS", "", model.quitting),
    ]
}
fn spawn_service(
    path: PathBuf,
    sender: mpsc::Sender<Event>,
    hwnd: isize,
    shutdown: watch::Receiver<bool>,
    network_stop: watch::Sender<bool>,
    sharing: crate::sharing::SharingControl,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("winios-api".into())
        .spawn(move || {
            let result: Result<(), AppError> = (|| {
                let config = settings::load(&path)?;
                let address_monitor = if config.tailnet_backend {
                    None
                } else {
                    Some(crate::network::AddressMonitor::new(
                        *config.bind.ip(),
                        network_stop,
                    )?)
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let running_sender = sender.clone();
                let result = runtime.block_on(app::run_managed(
                    config,
                    false,
                    shutdown,
                    Arc::new(move |event| notify(&running_sender, hwnd, event)),
                    sharing,
                ));
                if address_monitor
                    .as_ref()
                    .is_some_and(|monitor| monitor.removed())
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AddrNotAvailable,
                        "Configured LAN IP was removed.",
                    )
                    .into());
                }
                result
            })(); // Runtime (including TLS/background work) is dropped before Finished.
            let result = result.map_err(|error| {
                let status = if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::AddrNotAvailable)
                {
                    Status::NetworkUnavailable
                } else {
                    Status::Error
                };
                let message = if status == Status::NetworkUnavailable {
                    "Configured LAN IP is unavailable. No other interface was selected.".into()
                } else {
                    error.to_string()
                };
                (status, message)
            });
            notify(&sender, hwnd, Event::Finished(result));
        })
}

struct Backend {
    stop: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Default GUI mode. Never attaches or allocates a console.
pub fn run(path: Option<PathBuf>) -> Result<(), AppError> {
    run_with_options(path, false, false)
}
pub fn run_with_options(
    path: Option<PathBuf>,
    native_menu: bool,
    startup_launch: bool,
) -> Result<(), AppError> {
    run_with_setup(path, native_menu, startup_launch, false)
}
pub fn run_with_setup(
    path: Option<PathBuf>,
    native_menu: bool,
    startup_launch: bool,
    open_setup: bool,
) -> Result<(), AppError> {
    run_with_ui(path, native_menu, startup_launch, open_setup, false)
}
pub fn run_with_ui(
    path: Option<PathBuf>,
    native_menu: bool,
    startup_launch: bool,
    open_setup: bool,
    open_settings: bool,
) -> Result<(), AppError> {
    use std::sync::atomic::Ordering;
    diagnostics::set_console(false);
    let _instance = match SingleInstance::acquire() {
        Ok(instance) => instance,
        Err(error) => {
            // Search/Start/pinned launches activate the existing tray instead of
            // creating a second watcher/server. Startup duplicates stay quiet.
            unsafe {
                if let Ok(existing) = FindWindowW(w!("WiniOSTrayWindow"), PCWSTR::null()) {
                    if !startup_launch {
                        // Pass the user-initiated launch's foreground right only
                        // to our existing singleton, never to arbitrary processes.
                        let mut process_id = 0;
                        GetWindowThreadProcessId(existing, Some(&mut process_id));
                        if process_id != 0 {
                            let _ = AllowSetForegroundWindow(process_id);
                        }
                        PostMessageW(
                            Some(existing),
                            if open_setup {
                                wizard::SHOW
                            } else if open_settings {
                                preferences_window::SHOW
                            } else {
                                MANUAL_LAUNCH
                            },
                            WPARAM(0),
                            LPARAM(-1),
                        )?;
                    }
                    return Ok(());
                }
            }
            return Err(error);
        }
    };
    let path = match path {
        Some(path) => path,
        None => settings::default_path()?,
    };
    settings::validate_local_absolute(&path)?;
    let mut window = TrayWindow::new()?;
    let (events_tx, events_rx) = mpsc::channel();
    let (stop, shutdown) = watch::channel(false);
    let mut model = TrayModel::default();
    let first_run = setup::needs_first_run(&path);
    if first_run {
        model.status = Status::NotConfigured;
        model.detail = "Complete Settings / Setup to create local preferences. No network settings were changed.".into();
    } else {
        model.begin();
    }
    let hwnd = window.hwnd.0 as isize;
    let worker = if first_run {
        None
    } else {
        Some(spawn_service(
            path.clone(),
            events_tx.clone(),
            hwnd,
            shutdown,
            stop.clone(),
            model.sharing.clone(),
        )?)
    };
    let mut backend = Backend { stop, worker };
    let mut setup_ui = wizard::Controller::new(window.hwnd, path.clone());
    let mut settings_ui = preferences_window::Controller::new(
        window.hwnd,
        path.clone(),
        !startup_launch && !first_run && !open_setup,
    );
    let mut tray_popup = flyout::Controller::new(window.hwnd);
    settings_ui.refresh();
    if open_settings {
        settings_ui.show()?;
    }
    if first_run || open_setup {
        setup_ui.show()?;
    }
    window.update(&model);
    let mut msg = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
        if result == -1 {
            return Err(windows::core::Error::from_thread().into());
        }
        let mut selection = 0;
        let mut needs_update = false;
        if result == 0
            || msg.message == QUIT_EVENT
            || (msg.hwnd == window.hwnd && msg.message == WM_CLOSE)
        {
            selection = QUIT;
        } else if msg.message == wizard::SHOW {
            if !model.quitting {
                setup_ui.show()?;
            }
        } else if msg.message == preferences_window::SHOW {
            if !model.quitting {
                settings_ui.show()?;
            }
        } else if msg.message == MANUAL_LAUNCH {
            LAUNCH_PENDING.store(true, Ordering::Relaxed);
        } else if msg.message == wizard::WAKE || msg.message == fluent::WAKE {
            // Queued setup actions/results are drained below, including wakes
            // consumed by the native menu's nested message loop.
        } else if msg.message == SHOW_MENU {
            if !model.quitting {
                let anchor = (msg.lParam.0 != 0).then(|| anchor_point(msg.wParam.0));
                if !native_menu && !fluent::Appearance::detect().contrast {
                    let point = window.anchor(anchor)?;
                    settings_ui.refresh();
                    if tray_popup
                        .open(point, fluent_rows(&model, settings_ui.startup()))
                        .is_err()
                    {
                        selection = window.menu(&model, anchor, &path, true)?;
                    }
                } else {
                    selection = window.menu(&model, anchor, &path, true)?;
                }
            }
            MENU_PENDING.store(false, Ordering::Relaxed);
        } else if msg.message == window.taskbar_created {
            READD_ICON.store(true, Ordering::Relaxed);
        } else if msg.message != STATUS_EVENT
            && !tray_popup.dialog(&msg)
            && !settings_ui.dialog(&msg)
            && !setup_ui.dialog_message(&msg)
        {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if let Some(action) = tray_popup.drain() {
            selection = action;
        }
        for request in settings_ui.drain(&model) {
            match request {
                preferences_window::Request::Retry => selection = RETRY,
                preferences_window::Request::Iphone => selection = ONBOARDING,
                preferences_window::Request::UpdateNotice => {
                    if !model.quitting {
                        window.update_notice();
                    }
                }
            }
        }
        if selection == QUIT || SESSION_QUIT.load(Ordering::Relaxed) {
            model.quit();
            setup_ui.close();
            settings_ui.close();
            tray_popup.close();
            let _ = backend.stop.send(true);
            needs_update = true;
        }
        if !model.quitting {
            if SETUP_PENDING.swap(false, Ordering::Relaxed) {
                setup_ui.show()?;
            }
            if SETTINGS_PENDING.swap(false, Ordering::Relaxed) {
                settings_ui.show()?;
            }
            if LAUNCH_PENDING.swap(false, Ordering::Relaxed) {
                if settings_ui.prefers_settings() {
                    settings_ui.show()?;
                } else {
                    post_menu(window.hwnd, WPARAM(0), false);
                }
            }
            if selection == PAUSE {
                model.sharing.set_paused(!model.sharing.is_paused());
                needs_update = true;
            } else if selection == SETUP {
                settings_ui.show()?;
            } else if selection == ONBOARDING {
                setup_ui.show()?;
            } else if selection == RETRY && setup::needs_first_run(&path) {
                setup_ui.show()?;
            } else if selection == RETRY && model.retry() {
                let _ = backend.stop.send(true);
                needs_update = true;
            } else if selection == STARTUP {
                settings_ui.toggle_startup();
                needs_update = true;
            } else if selection == STATUS {
                settings_ui.show()?;
            }
        }
        // New settings are applied only by explicit consent. Reopening Setup
        // never restarts or mutates an existing installation.
        if setup_ui.drain() && !model.quitting && !model.worker_active {
            model.retry();
        }
        // Native menus run a nested message loop. Always drain queued state
        // after a menu closes, even if that loop consumed our wake-up message.
        while let Ok(event) = events_rx.try_recv() {
            if let Event::ClipboardWriter(writer) = &event {
                setup_ui.set_writer(Some(writer.clone()));
                settings_ui.set_writer(Some(writer.clone()));
            }
            if let Event::Permissions(enabled) = &event {
                settings_ui.set_active_writes(*enabled);
            }
            if let Event::ImageOptions(options) = &event {
                settings_ui.set_active_images(*options);
            }
            needs_update = true;
            let finished = matches!(event, Event::Finished(_));
            model.apply(event);
            if finished {
                setup_ui.set_writer(None);
                settings_ui.set_writer(None);
                if let Some(handle) = backend.worker.take() {
                    let _ = handle.join();
                }
                if !model.quitting
                    && matches!(model.status, Status::Error | Status::NetworkUnavailable)
                {
                    show_error(&format!(
                        "WiniOS did not start:\n\n{}\n\nFix settings/certificate/network, then choose Retry from the tray. No identity or trust was changed.",
                        model.detail
                    ));
                }
            }
        }
        if model.retry_pending && !model.worker_active && model.begin() {
            needs_update = true;
            let (new_stop, receiver) = watch::channel(false);
            backend.stop = new_stop;
            backend.worker = Some(spawn_service(
                path.clone(),
                events_tx.clone(),
                hwnd,
                receiver,
                backend.stop.clone(),
                model.sharing.clone(),
            )?);
        }
        if READD_ICON.swap(false, Ordering::Relaxed) {
            window.added = false;
            window.add_icon()?;
            needs_update = true;
        }
        if SESSION_QUIT.load(Ordering::Relaxed) && !model.quitting {
            model.quit();
            setup_ui.close();
            settings_ui.close();
            tray_popup.close();
            let _ = backend.stop.send(true);
            needs_update = true;
        }
        if needs_update {
            window.update(&model);
        }
        settings_ui.render(&model);
        if tray_popup.is_open() {
            tray_popup.refresh(fluent_rows(&model, settings_ui.startup()));
        }
        if model.quitting && !model.worker_active && !setup_ui.busy() && !settings_ui.busy() {
            break;
        }
    }
    // UI resources then singleton are removed by RAII after worker completion.
    Ok(())
}
/// Opt-in diagnostics path; default tray launches never execute console APIs.
pub fn run_console(args: Vec<String>) -> Result<(), AppError> {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole};
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            AllocConsole()?;
        }
    }
    diagnostics::set_console(true);
    let config = Config::parse(args)?;
    if config.help {
        crate::diagnostics::info!(
            "Console CLI options: --lan <private-IP> OR --tailnet-backend; --port <port> --allow-clipboard-write --tls-cert <PEM> --tls-key <PEM>. Use winios-agent.exe --help for full CLI usage."
        );
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(app::run(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fluent_menu_tracks_live_sharing_startup_and_shutdown() {
        let mut model = TrayModel::default();
        let startup = integration::StartupInfo {
            registered: true,
            can_toggle: true,
            note: "Test-only registration",
        };
        let rows = fluent_rows(&model, Some(startup.clone()));
        assert!(rows.iter().find(|r| r.id == STARTUP).unwrap().checked);
        for id in [PAUSE, RETRY, SETUP, STARTUP, QUIT] {
            assert_eq!(rows.iter().filter(|r| r.id == id).count(), 1);
            assert!(!rows.iter().find(|r| r.id == id).unwrap().disabled);
        }
        assert!(
            rows.iter()
                .find(|r| r.id == PAUSE)
                .unwrap()
                .label
                .contains("Pause")
        );
        model.sharing.set_paused(true);
        let rows = fluent_rows(&model, None);
        assert!(
            rows.iter()
                .find(|r| r.id == PAUSE)
                .unwrap()
                .label
                .contains("Resume")
        );
        assert!(rows.iter().find(|r| r.id == STARTUP).unwrap().disabled);
        assert!(!rows.iter().find(|r| r.id == STARTUP).unwrap().checked);
        model.quit();
        let rows = fluent_rows(&model, Some(startup));
        assert!(
            [PAUSE, RETRY, SETUP, STARTUP, QUIT].iter().all(|id| rows
                .iter()
                .find(|r| r.id == *id)
                .unwrap()
                .disabled)
        );
    }
    #[test]
    fn v4_callback_decodes_icon_id_and_signed_coordinates() {
        assert_eq!(
            callback_event(LPARAM(((ICON_ID << 16) | WM_CONTEXTMENU) as isize)),
            Some(WM_CONTEXTMENU)
        );
        assert_eq!(
            callback_event(LPARAM(((ICON_ID << 16) | KEY_SELECT) as isize)),
            Some(KEY_SELECT)
        );
        assert_eq!(
            callback_event(LPARAM(((2 << 16) | WM_CONTEXTMENU) as isize)),
            None
        );
        let point = anchor_point(((42u16 as usize) << 16) | (-150i16 as u16 as usize));
        assert_eq!((point.x, point.y), (-150, 42));
        let point = anchor_point(u32::MAX as usize);
        assert_eq!((point.x, point.y), (-1, -1));
    }

    #[test]
    fn sent_shell_v4_context_callback_posts_a_menu_request() {
        // Shell callbacks can be sent directly to WndProc instead of returned
        // by GetMessage. No tray icon, foreground window or agent is started.
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("WiniOSTrayCallbackTest"),
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!("Callback test"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )
            .unwrap();
            SendMessageW(
                hwnd,
                CALLBACK,
                Some(WPARAM((80 << 16) | 40)),
                Some(LPARAM((1 << 16) | WM_CONTEXTMENU as isize)),
            );
            let mut message = MSG::default();
            let posted = PeekMessageW(
                &mut message,
                Some(hwnd),
                WM_APP + 43,
                WM_APP + 43,
                PM_REMOVE,
            )
            .as_bool();
            let _ = DestroyWindow(hwnd);
            let _ = UnregisterClassW(class.lpszClassName, Some(instance));
            MENU_PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
            assert!(
                posted,
                "sent version-4 context callback must wake the UI menu loop"
            );
            assert_eq!(message.wParam.0, (80 << 16) | 40);
        }
    }
}
