//! Modeless native setup UI. Only the owning UI thread touches HWNDs.
//! Read-only network/process checks run on a bounded, cancellable worker.
use crate::{
    iphone::{self, Export},
    native::AppError,
    setup::{self, Installation, Model, Page},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};
use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{Dwm::*, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::*, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::EnableWindow,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

pub const SHOW: u32 = WM_APP + 44;
pub const WAKE: u32 = WM_APP + 45;
const NEXT: usize = 1;
const CANCEL: usize = 2;
const BACK: usize = 1201;
const CHECK: usize = 1202;
const CONSENT: usize = 1203;
const LAYOUT: usize = 1204;
const OPEN_SEND: usize = 1205;
const OPEN_RECEIVE: usize = 1206;
const COPY_ENDPOINT: usize = 1207;
const COPY_TOKEN: usize = 1208;
const BG: COLORREF = COLORREF(0x0024100c); // Shared midnight brand background.
const FG: COLORREF = COLORREF(0x00fff6f1);
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

struct View {
    parent: HWND,
    session: usize,
    brush: HBRUSH,
    actions: mpsc::Sender<(usize, usize)>,
}
impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.brush.into());
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const View;
        if !ptr.is_null() {
            let view = &*ptr;
            let post = |action| {
                let _ = view.actions.send((action, view.session));
                let _ = PostMessageW(Some(view.parent), WAKE, WPARAM(0), LPARAM(0));
            };
            match message {
                WM_COMMAND => {post(wp.0 & 0xffff);return LRESULT(0);},
                WM_CLOSE => {
                    // Quit must also dismiss a token confirmation's nested loop.
                    let popup=GetLastActivePopup(hwnd);
                    if popup!=hwnd && !popup.0.is_null(){let _=PostMessageW(Some(popup),WM_CLOSE,WPARAM(0),LPARAM(0));}
                    post(CANCEL);return LRESULT(0);
                },
                WM_SIZE => {post(LAYOUT);return LRESULT(0);},
                WM_DPICHANGED => {
                    let rect=&*(lp.0 as *const RECT);
                    let _=SetWindowPos(hwnd,None,rect.left,rect.top,rect.right-rect.left,rect.bottom-rect.top,SWP_NOZORDER|SWP_NOACTIVATE);
                    post(LAYOUT);return LRESULT(0);
                },
                WM_GETMINMAXINFO => {
                    let info=&mut *(lp.0 as *mut MINMAXINFO);
                    let dpi=GetDpiForWindow(hwnd).max(96) as i32;
                    info.ptMinTrackSize.x=600*dpi/96;info.ptMinTrackSize.y=600*dpi/96;
                    return LRESULT(0);
                },
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
                    let dc=HDC(wp.0 as *mut _);SetTextColor(dc,FG);SetBkColor(dc,BG);
                    return LRESULT(view.brush.0 as isize);
                },
                WM_ERASEBKGND => {
                    let mut rect=RECT::default();let _=GetClientRect(hwnd,&mut rect);
                    FillRect(HDC(wp.0 as *mut _),&rect,view.brush);return LRESULT(1);
                },
                // IsDialogMessage uses the documented dialog-manager contract
                // for Tab, Shift+Tab, arrows, Escape and default Enter button.
                0x0400 /* DM_GETDEFID */ => return LRESULT((0x534b << 16) | NEXT as isize),
                WM_NCDESTROY => {SetWindowLongPtrW(hwnd,GWLP_USERDATA,0);},
                _=>{},
            }
        }
        DefWindowProcW(hwnd, message, wp, lp)
    }
}

struct Controls {
    step: HWND,
    heading: HWND,
    body: HWND,
    status: HWND,
    consent: HWND,
    open_send: HWND,
    open_receive: HWND,
    copy_endpoint: HWND,
    copy_token: HWND,
    back: HWND,
    check: HWND,
    next: HWND,
    cancel: HWND,
}
struct Window {
    hwnd: HWND,
    _view: Box<View>,
    controls: Controls,
    fonts: Vec<HFONT>,
    dpi: u32,
}
impl Window {
    fn new(
        parent: HWND,
        session: usize,
        actions: mpsc::Sender<(usize, usize)>,
    ) -> Result<Self, AppError> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None)?.0);
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("WiniOSSetupWindow"),
                lpfnWndProc: Some(procedure),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            // Registration is process-local and intentionally kept for reopening.
            if RegisterClassW(&class) == 0
                && windows::core::Error::from_thread().code()
                    != windows::core::HRESULT::from_win32(1410)
            {
                return Err(windows::core::Error::from_thread().into());
            }
            let brush = CreateSolidBrush(BG);
            if brush.0.is_null() {
                return Err("Could not allocate setup window resources.".into());
            }
            let view = Box::new(View {
                parent,
                session,
                brush,
                actions,
            });
            let hwnd = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.lpszClassName,
                w!("WiniOS · Settings / Setup"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                720,
                740,
                Some(parent),
                None,
                Some(instance),
                Some((&*view as *const View).cast()),
            )?;
            // Own the window before allocating children, so all failure paths
            // destroy it while GWLP_USERDATA still points to a live View.
            let empty = Controls {
                step: HWND::default(),
                heading: HWND::default(),
                body: HWND::default(),
                status: HWND::default(),
                consent: HWND::default(),
                open_send: HWND::default(),
                open_receive: HWND::default(),
                copy_endpoint: HWND::default(),
                copy_token: HWND::default(),
                back: HWND::default(),
                check: HWND::default(),
                next: HWND::default(),
                cancel: HWND::default(),
            };
            let mut window = Self {
                hwnd,
                _view: view,
                controls: empty,
                fonts: Vec::new(),
                dpi: 0,
            };
            let child = |class: PCWSTR,
                         label: PCWSTR,
                         style: WINDOW_STYLE,
                         id: usize|
             -> windows::core::Result<HWND> {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    label,
                    WS_CHILD | WS_VISIBLE | style,
                    0,
                    0,
                    0,
                    0,
                    Some(hwnd),
                    Some(HMENU(id as *mut _)),
                    Some(instance),
                    None,
                )
            };
            window.controls = Controls {
                step: child(w!("STATIC"), w!(""), WINDOW_STYLE::default(), 0)?,
                heading: child(w!("STATIC"), w!(""), WINDOW_STYLE::default(), 0)?,
                body: child(
                    w!("EDIT"),
                    w!(""),
                    WS_TABSTOP | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                    0,
                )?,
                status: child(
                    w!("EDIT"),
                    w!(""),
                    WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                    0,
                )?,
                consent: child(
                    w!("BUTTON"),
                    w!("Allow authenticated iPhone requests to replace my Windows clipboard"),
                    WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | BS_MULTILINE as u32),
                    CONSENT,
                )?,
                open_send: child(
                    w!("BUTTON"),
                    w!("Open &Send"),
                    WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    OPEN_SEND,
                )?,
                open_receive: child(
                    w!("BUTTON"),
                    w!("Open &Receive"),
                    WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    OPEN_RECEIVE,
                )?,
                copy_endpoint: child(
                    w!("BUTTON"),
                    w!("Copy HTTPS &URL"),
                    WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    COPY_ENDPOINT,
                )?,
                copy_token: child(
                    w!("BUTTON"),
                    w!("Copy WiniOS &Token…"),
                    WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    COPY_TOKEN,
                )?,
                back: child(
                    w!("BUTTON"),
                    w!("&Back"),
                    WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    BACK,
                )?,
                check: child(
                    w!("BUTTON"),
                    w!("&Check again"),
                    WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    CHECK,
                )?,
                next: child(
                    w!("BUTTON"),
                    w!("&Next"),
                    WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                    NEXT,
                )?,
                cancel: child(
                    w!("BUTTON"),
                    w!("&Close"),
                    WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    CANCEL,
                )?,
            };
            for button in [
                window.controls.consent,
                window.controls.open_send,
                window.controls.open_receive,
                window.controls.copy_endpoint,
                window.controls.copy_token,
                window.controls.back,
                window.controls.check,
                window.controls.next,
                window.controls.cancel,
            ] {
                let _ = SetWindowTheme(button, w!("DarkMode_Explorer"), PCWSTR::null());
            }
            let dark = 1i32;
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark as *const i32).cast(),
                4,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                4,
            );
            let icon = SendMessageW(parent, WM_GETICON, Some(WPARAM(ICON_BIG as usize)), None);
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(icon.0)),
            );
            window.layout();
            Ok(window)
        }
    }
    fn layout(&mut self) {
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd).max(96);
            if dpi != self.dpi {
                let make = |size: i32, weight: i32| {
                    CreateFontW(
                        -size * dpi as i32 / 96,
                        0,
                        0,
                        0,
                        weight,
                        0,
                        0,
                        0,
                        DEFAULT_CHARSET,
                        OUT_DEFAULT_PRECIS,
                        CLIP_DEFAULT_PRECIS,
                        CLEARTYPE_QUALITY,
                        DEFAULT_PITCH.0 as u32,
                        w!("Segoe UI"),
                    )
                };
                let body = make(16, 400);
                let title = make(27, 600);
                if !body.0.is_null() && !title.0.is_null() {
                    for hwnd in [
                        self.controls.step,
                        self.controls.body,
                        self.controls.status,
                        self.controls.consent,
                        self.controls.open_send,
                        self.controls.open_receive,
                        self.controls.copy_endpoint,
                        self.controls.copy_token,
                        self.controls.back,
                        self.controls.check,
                        self.controls.next,
                        self.controls.cancel,
                    ] {
                        SendMessageW(
                            hwnd,
                            WM_SETFONT,
                            Some(WPARAM(body.0 as usize)),
                            Some(LPARAM(1)),
                        );
                    }
                    SendMessageW(
                        self.controls.heading,
                        WM_SETFONT,
                        Some(WPARAM(title.0 as usize)),
                        Some(LPARAM(1)),
                    );
                    for font in self.fonts.drain(..) {
                        let _ = DeleteObject(font.into());
                    }
                    self.fonts = vec![body, title];
                    self.dpi = dpi;
                } else {
                    if !body.0.is_null() {
                        let _ = DeleteObject(body.into());
                    }
                    if !title.0.is_null() {
                        let _ = DeleteObject(title.into());
                    }
                }
            }
            let mut rect = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rect);
            let scale = |n: i32| n * dpi as i32 / 96;
            let width = rect.right * 96 / dpi as i32;
            let height = rect.bottom * 96 / dpi as i32;
            let place = |hwnd, x, y, w, h| {
                let _ = MoveWindow(hwnd, scale(x), scale(y), scale(w), scale(h), true);
                crate::fluent::update_text_overflow(hwnd);
            };
            place(self.controls.step, 24, 18, width - 48, 24);
            place(self.controls.heading, 24, 48, width - 48, 44);
            place(
                self.controls.body,
                24,
                105,
                width - 48,
                (height - 320).max(120),
            );
            place(self.controls.consent, 24, height - 205, width - 48, 54);
            let half = (width - 60) / 2;
            place(self.controls.open_send, 24, height - 248, half, 32);
            place(
                self.controls.open_receive,
                36 + half,
                height - 248,
                half,
                32,
            );
            place(self.controls.copy_endpoint, 24, height - 208, half, 32);
            place(self.controls.copy_token, 36 + half, height - 208, half, 32);
            place(self.controls.status, 24, height - 143, width - 48, 70);
            place(self.controls.back, 24, height - 52, 82, 32);
            place(self.controls.check, 116, height - 52, 112, 32);
            place(self.controls.next, width - 242, height - 52, 126, 32);
            place(self.controls.cancel, width - 106, height - 52, 82, 32);
        }
    }
    fn render(&self, model: &Model, writer_ready: bool) {
        unsafe {
            for (hwnd, value) in [
                (self.controls.step, model.step()),
                (self.controls.heading, model.heading()),
                (self.controls.body, &model.instructions()),
                (self.controls.status, &model.message),
                (self.controls.next, model.next_label()),
            ] {
                let value = wide(value);
                let _ = SetWindowTextW(hwnd, PCWSTR(value.as_ptr()));
                crate::fluent::update_text_overflow(hwnd);
            }
            let consent = model.page == Page::Consent;
            let iphone = model.page == Page::Iphone;
            let label = wide(if iphone {
                "&Test connection"
            } else {
                "&Check again"
            });
            let _ = SetWindowTextW(self.controls.check, PCWSTR(label.as_ptr()));
            for (hwnd, enabled) in [
                (self.controls.open_send, !model.busy),
                (self.controls.open_receive, !model.busy),
                (
                    self.controls.copy_endpoint,
                    !model.busy && writer_ready && model.endpoint.is_some(),
                ),
                (
                    self.controls.copy_token,
                    !model.busy && writer_ready && model.endpoint.is_some(),
                ),
            ] {
                let _ = ShowWindow(hwnd, if iphone { SW_SHOW } else { SW_HIDE });
                let _ = EnableWindow(hwnd, iphone && enabled);
            }
            // Reserve a second row for iPhone actions; all other steps retain
            // their original native layout. Recompute after resize/DPI changes.
            let mut rect = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rect);
            let dpi = GetDpiForWindow(self.hwnd).max(96) as i32;
            let body_height =
                (rect.bottom - (if iphone { 365 } else { 320 }) * dpi / 96).max(120 * dpi / 96);
            let _ = MoveWindow(
                self.controls.body,
                24 * dpi / 96,
                105 * dpi / 96,
                rect.right - 48 * dpi / 96,
                body_height,
                true,
            );
            let _ = ShowWindow(
                self.controls.consent,
                if consent { SW_SHOW } else { SW_HIDE },
            );
            let enabled = model.is_new() && !model.busy;
            let _ = EnableWindow(self.controls.consent, enabled);
            SendMessageW(
                self.controls.consent,
                BM_SETCHECK,
                Some(WPARAM(if if model.is_new() {
                    model.writes_consent
                } else {
                    model.profile().writes
                } {
                    BST_CHECKED.0
                } else {
                    BST_UNCHECKED.0
                } as usize)),
                None,
            );
            let _ = EnableWindow(self.controls.next, model.can_next());
            let _ = EnableWindow(self.controls.back, model.can_back());
            let _ = EnableWindow(
                self.controls.check,
                !model.busy
                    && matches!(
                        model.page,
                        Page::Client | Page::Serve | Page::Test | Page::Iphone
                    ),
            );
        }
    }
    fn activate(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            for font in self.fonts.drain(..) {
                let _ = DeleteObject(font.into());
            }
        }
    }
}

enum ResultKind {
    Loaded(Installation),
    Checked(setup::Check),
    Applied(Result<setup::Profile, &'static str>),
    Action(Result<&'static str, &'static str>),
}
struct ResultMessage {
    session: usize,
    kind: ResultKind,
}
enum Job {
    Load,
    Check(Page, setup::Profile),
    Apply(bool),
    Open(bool),
    Copy(setup::Profile, crate::writes::WriteClient, Export),
}
struct Work {
    handle: JoinHandle<()>,
    cancel: Arc<AtomicBool>,
}
pub struct Controller {
    parent: HWND,
    path: PathBuf,
    window: Option<Window>,
    model: Model,
    session: usize,
    tx: mpsc::Sender<ResultMessage>,
    rx: mpsc::Receiver<ResultMessage>,
    work: Option<Work>,
    actions_tx: mpsc::Sender<(usize, usize)>,
    actions_rx: mpsc::Receiver<(usize, usize)>,
    writer: Option<crate::writes::WriteClient>,
}
impl Controller {
    pub fn new(parent: HWND, path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let (actions_tx, actions_rx) = mpsc::channel();
        Self {
            parent,
            path,
            window: None,
            model: Model::default(),
            session: 0,
            tx,
            rx,
            work: None,
            actions_tx,
            actions_rx,
            writer: None,
        }
    }
    pub fn show(&mut self) -> Result<(), AppError> {
        if let Some(window) = &self.window {
            window.activate();
            return Ok(());
        }
        // A just-closed check must finish cancellation before another is opened.
        if self.work.is_some() {
            return Ok(());
        }
        self.session = self.session.wrapping_add(1);
        self.model = Model::default();
        self.window = Some(Window::new(
            self.parent,
            self.session,
            self.actions_tx.clone(),
        )?);
        self.render();
        self.window.as_ref().unwrap().activate();
        self.start(Job::Load);
        Ok(())
    }
    fn render(&self) {
        if let Some(window) = &self.window {
            window.render(&self.model, self.writer.is_some());
        }
    }
    pub fn set_writer(&mut self, writer: Option<crate::writes::WriteClient>) {
        self.writer = writer;
        self.render();
    }
    fn start(&mut self, job: Job) {
        if self.work.is_some() {
            return;
        }
        self.model.busy = true;
        self.model.passed = false;
        self.model.message = "Checking… You can close this window at any time.".into();
        self.render();
        let (tx, path, session, parent) = (
            self.tx.clone(),
            self.path.clone(),
            self.session,
            self.parent.0 as isize,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let handle = std::thread::Builder::new()
            .name("winios-setup".into())
            .spawn(move || {
                let kind = match job {
                    Job::Open(send)=>ResultKind::Action(iphone::open_template(send).map(|_|"Opened the public template link in your default Windows browser. Review/import it in Shortcuts on iPhone; no endpoint or token was included in the link.")),
                    Job::Copy(profile,writer,export)=>{
                        let result=tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|"Could not start the background copy.").and_then(|runtime|runtime.block_on(iphone::copy_export(profile,writer,export,&worker_cancel)));
                        ResultKind::Action(result)
                    },
                    Job::Load => ResultKind::Loaded(setup::inspect_installation(&path)),
                    Job::Apply(writes) => ResultKind::Applied(setup::create_new_preferences(
                        &path,
                        writes,
                        &worker_cancel,
                    )),
                    Job::Check(page, profile) => {
                        let result = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map(|runtime| {
                                runtime.block_on(async {
                                    match page {
                                        Page::Client => setup::check_client(&profile).await,
                                        Page::Serve => setup::check_serve(&profile).await,
                                        _ => setup::test_connection(&profile, &worker_cancel).await,
                                    }
                                })
                            });
                        ResultKind::Checked(result.unwrap_or_else(|_| setup::Check {
                            passed: false,
                            message: "Could not start the background setup check.".into(),
                            endpoint: None,
                        }))
                    }
                };
                let _ = tx.send(ResultMessage { session, kind });
                unsafe {
                    let _ = PostMessageW(Some(HWND(parent as *mut _)), WAKE, WPARAM(0), LPARAM(0));
                }
            });
        match handle {
            Ok(handle) => self.work = Some(Work { handle, cancel }),
            Err(_) => {
                self.model.busy = false;
                self.model.message =
                    "Could not start the background setup check. Close and retry.".into();
                self.render();
            }
        }
    }
    fn check(&mut self) {
        // An old success must not remain visible after a failed network recheck.
        self.model.endpoint = None;
        self.start(Job::Check(self.model.page, self.model.profile()));
    }
    /// Returns true only after a *new* settings file was atomically committed.
    /// The parent starts the normal agent; existing services are never restarted.
    pub fn drain(&mut self) -> bool {
        // Native popup menus may consume wake messages in their nested loop;
        // actions/results stay queued until the outer tray loop drains them.
        while let Ok((action, session)) = self.actions_rx.try_recv() {
            self.action(action, session);
        }
        let mut start_agent = false;
        while let Ok(result) = self.rx.try_recv() {
            if let Some(work) = self.work.take() {
                let _ = work.handle.join();
            }
            if result.session != self.session {
                continue;
            }
            // Apply remains valid if the window was closed just after commit.
            if let ResultKind::Applied(Ok(_)) = &result.kind {
                start_agent = true;
            }
            if self.window.is_none() {
                continue;
            }
            self.model.busy = false;
            match result.kind {
                ResultKind::Loaded(installation) => {
                    self.model.message = match &installation {
                        Installation::New => "New installation. Clipboard writes start OFF.".into(),
                        Installation::Existing(_) => {
                            "Existing settings found. Protected review; nothing will be changed."
                                .into()
                        }
                        Installation::ProtectedError(error) => (*error).into(),
                    };
                    self.model.installation = Some(installation);
                }
                ResultKind::Checked(check) => {
                    self.model.passed = check.passed;
                    self.model.message = check.message;
                    self.model.endpoint = check.endpoint;
                }
                ResultKind::Applied(result) => match result {
                    Ok(profile) => {
                        self.model.applied = true;
                        self.model.installation = Some(Installation::Existing(profile));
                        self.model.page = Page::Test;
                        self.model.message="Preferences applied. Starting WiniOS. Choose Check again when the tray is connected.".into();
                    }
                    Err(error) => self.model.message = error.into(),
                },
                ResultKind::Action(result) => {
                    self.model.message = match result {
                        Ok(message) | Err(message) => message.into(),
                    }
                }
            }
            self.render();
        }
        start_agent
    }
    fn action(&mut self, action: usize, session: usize) {
        if session != self.session || self.window.is_none() {
            return;
        }
        if action == CANCEL {
            self.close();
            return;
        }
        if action == LAYOUT {
            self.window.as_mut().unwrap().layout();
            self.render();
            return;
        }
        if self.model.busy {
            return;
        }
        match action {
            OPEN_SEND | OPEN_RECEIVE if self.model.page == Page::Iphone => {
                self.start(Job::Open(action == OPEN_SEND))
            }
            COPY_ENDPOINT | COPY_TOKEN if self.model.page == Page::Iphone => {
                let Some(writer) = self.writer.clone() else {
                    self.model.message="Clipboard worker is unavailable. Retry Connection before copying setup details.".into();
                    self.render();
                    return;
                };
                if action == COPY_TOKEN {
                    let warning = wide(iphone::TOKEN_WARNING);
                    let confirmed = unsafe {
                        MessageBoxW(
                            Some(self.window.as_ref().unwrap().hwnd),
                            PCWSTR(warning.as_ptr()),
                            w!("Copy bearer token — sensitive credential"),
                            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                        )
                    } == IDYES;
                    if !confirmed {
                        self.model.message = "Bearer token was not read or copied.".into();
                        self.render();
                        return;
                    }
                }
                self.start(Job::Copy(
                    self.model.profile(),
                    writer,
                    if action == COPY_TOKEN {
                        Export::Token
                    } else {
                        Export::SetupBase
                    },
                ));
            }
            CONSENT if self.model.is_new() => {
                let checked = unsafe {
                    SendMessageW(
                        self.window.as_ref().unwrap().controls.consent,
                        BM_GETCHECK,
                        None,
                        None,
                    )
                }
                .0 == BST_CHECKED.0 as isize;
                self.model.writes_consent = checked;
            }
            CHECK
                if matches!(
                    self.model.page,
                    Page::Client | Page::Serve | Page::Test | Page::Iphone
                ) =>
            {
                self.check()
            }
            BACK if self.model.can_back() => {
                self.model.page = match self.model.page {
                    Page::Client => Page::Welcome,
                    Page::Serve => Page::Client,
                    Page::Consent => Page::Serve,
                    Page::Iphone => Page::Test,
                    _ => Page::Consent,
                };
                self.model.passed = false;
                self.model.message = "Review this step, then Check again.".into();
                if matches!(self.model.page, Page::Client | Page::Serve | Page::Test) {
                    self.check();
                }
            }
            NEXT if self.model.can_next() => match self.model.page {
                Page::Welcome => {
                    self.model.page = Page::Client;
                    self.check();
                }
                Page::Client => {
                    self.model.page = Page::Serve;
                    self.check();
                }
                Page::Serve => {
                    self.model.page = Page::Consent;
                    self.model.message="Review the permission choice. Existing permissions cannot be changed here.".into();
                }
                Page::Consent if self.model.is_new() => {
                    self.start(Job::Apply(self.model.writes_consent))
                }
                Page::Consent => {
                    self.model.page = Page::Test;
                    self.check();
                }
                Page::Test => {
                    self.model.page = Page::Iphone;
                    self.model.message="Use your own endpoint and token in both Import Questions. Review every action before entering a credential.".into();
                }
                Page::Iphone => self.close(),
            },
            _ => {}
        }
        self.render();
    }
    pub fn dialog_message(&self, message: &MSG) -> bool {
        self.window.as_ref().is_some_and(|window| unsafe {
            (message.hwnd == window.hwnd || IsChild(window.hwnd, message.hwnd).as_bool())
                && IsDialogMessageW(window.hwnd, message).as_bool()
        })
    }
    pub fn busy(&self) -> bool {
        self.work.is_some()
    }
    pub fn close(&mut self) {
        if let Some(work) = &self.work {
            work.cancel.store(true, Ordering::Release);
        }
        self.window.take();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.close();
        if let Some(work) = self.work.take() {
            let _ = work.handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parent() -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("setup-test-parent"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap()
        }
    }
    fn drain_until_idle(controller: &mut Controller) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut applied = false;
        while controller.busy() {
            applied |= controller.drain();
            assert!(
                std::time::Instant::now() < deadline,
                "setup job must complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        applied
    }
    #[test]
    fn setup_cancel_and_apply_lifecycle_does_not_start_or_overwrite_existing_service() {
        let parent = parent();
        let path = std::env::temp_dir().join(format!(
            "winios-wizard-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut controller = Controller::new(parent, path.clone());
        controller.session = 1;
        controller.window = Some(Window::new(parent, 1, controller.actions_tx.clone()).unwrap());
        controller.start(Job::Load);
        controller.close();
        assert!(!drain_until_idle(&mut controller));
        assert!(!path.exists());
        // A fresh permission choice commits once and notifies the parent once.
        controller.session = 2;
        controller.model = Model {
            page: Page::Consent,
            installation: Some(Installation::New),
            writes_consent: true,
            busy: false,
            ..Model::default()
        };
        controller.window = Some(Window::new(parent, 2, controller.actions_tx.clone()).unwrap());
        controller.action(NEXT, 2);
        assert!(drain_until_idle(&mut controller));
        assert!(!controller.drain());
        assert!(crate::settings::load(&path).unwrap().allow_write);
        assert!(controller.model.applied && !controller.model.can_back());
        let original = std::fs::read(&path).unwrap();
        controller.model.page = Page::Consent;
        // A consent callback on an existing installation cannot change it.
        controller.action(CONSENT, 2);
        controller.close();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        drop(controller);
        std::fs::remove_file(path).unwrap();
        unsafe {
            let _ = DestroyWindow(parent);
        }
    }
    #[test]
    fn iphone_controls_cannot_export_without_worker_and_finish_never_changes_settings() {
        let parent = parent();
        let mut controller =
            Controller::new(parent, PathBuf::from(r"C:\unused-test-settings.json"));
        controller.session = 3;
        controller.window = Some(Window::new(parent, 3, controller.actions_tx.clone()).unwrap());
        controller.model = Model {
            page: Page::Test,
            busy: false,
            passed: true,
            installation: Some(Installation::Existing(setup::Profile::new_tailnet(false))),
            ..Model::default()
        };
        controller.action(NEXT, 3);
        assert_eq!(controller.model.page, Page::Iphone);
        assert!(!controller.work.is_some());
        for action in [COPY_ENDPOINT, COPY_TOKEN] {
            controller.action(action, 3);
            assert!(!controller.busy());
            assert!(controller.model.message.contains("unavailable"));
        }
        controller.action(NEXT, 3);
        assert!(controller.window.is_none());
        assert!(!controller.busy());
        drop(controller);
        unsafe {
            let _ = DestroyWindow(parent);
        }
    }
    #[test]
    fn native_setup_controls_render_and_close_without_changing_settings() {
        unsafe {
            let parent = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("setup-test-parent"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap();
            for existing in [false, true] {
                let (actions_tx, actions_rx) = mpsc::channel();
                let window = Window::new(parent, 7, actions_tx).unwrap();
                let mut model = Model {
                    installation: Some(if existing {
                        Installation::Existing(setup::Profile::new_tailnet(true))
                    } else {
                        Installation::New
                    }),
                    busy: false,
                    ..Model::default()
                };
                for page in [
                    Page::Welcome,
                    Page::Client,
                    Page::Serve,
                    Page::Consent,
                    Page::Test,
                    Page::Iphone,
                ] {
                    model.page = page;
                    window.render(&model, false);
                    if page == Page::Consent {
                        assert_eq!(
                            windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(
                                window.controls.consent
                            )
                            .as_bool(),
                            !existing
                        );
                        assert_eq!(
                            SendMessageW(window.controls.consent, BM_GETCHECK, None, None).0
                                == BST_CHECKED.0 as isize,
                            existing
                        );
                    }
                }
                SendMessageW(window.hwnd, WM_CLOSE, None, None);
                let mut message = MSG::default();
                let mut closed = false;
                while PeekMessageW(&mut message, Some(parent), WAKE, WAKE, PM_REMOVE).as_bool() {
                    let _ = message;
                }
                while let Ok((action, session)) = actions_rx.try_recv() {
                    if action == CANCEL && session == 7 {
                        closed = true;
                    }
                }
                assert!(closed);
                drop(window);
            }
            let _ = DestroyWindow(parent);
        }
    }
}
