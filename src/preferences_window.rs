//! Native Settings; mutable preferences use background atomic commits.
use crate::{
    auto_updates,
    fluent::{self, Control, Form},
    integration,
    iphone::{self, Export},
    lifecycle::TrayModel,
    native::AppError,
    settings, setup, updates,
    writes::WriteClient,
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
        Foundation::{HWND, LPARAM, WPARAM},
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};
pub const SHOW: u32 = WM_APP + 49;
const GENERAL: usize = 2101;
const CONNECTION: usize = 2102;
const CLIPBOARD: usize = 2103;
const PRIVACY: usize = 2104;
const ABOUT: usize = 2105;
const STARTUP: usize = 2110;
const LAUNCH: usize = 2111;
const SHARE: usize = 2112;
const WRITES: usize = 2113;
const COPY_ENDPOINT: usize = 2114;
const TEST: usize = 2115;
const RETRY: usize = 2116;
const IPHONE: usize = 2117;
const TOKEN: usize = 2118;
const DIAGNOSTICS: usize = 2119;
const RELOAD: usize = 2120;
const IMAGES: usize = 2106;
const IMAGE_ENABLE: usize = 2121;
const IMAGE_LIMIT: usize = 2122;
const CHECK_UPDATES: usize = 2123;
const OPEN_RELEASE: usize = 2124;
const AUTO_UPDATES: usize = 2125;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    General,
    Connection,
    Clipboard,
    Images,
    Privacy,
    About,
}
impl Page {
    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Connection => "Connection",
            Self::Clipboard => "Clipboard",
            Self::Images => "Images",
            Self::Privacy => "Privacy",
            Self::About => "About",
        }
    }
}
struct Snapshot {
    raw: Vec<u8>,
    writes: bool,
    desktop: settings::DesktopOptions,
    desktop_raw: Option<Vec<u8>>,
    images: crate::image::Options,
    images_raw: Option<Vec<u8>>,
    startup: integration::StartupInfo,
}
fn load(path: &std::path::Path) -> Result<Snapshot, &'static str> {
    settings::load(path)?;
    let raw = settings::read_bounded(path)?;
    let writes = settings::write_permission(&raw)?;
    let (desktop, desktop_raw) = settings::load_desktop(path)?;
    let (images, images_raw) = settings::load_images(path)?;
    Ok(Snapshot {
        raw,
        writes,
        desktop,
        desktop_raw,
        images,
        images_raw,
        startup: integration::startup_info(path),
    })
}
struct Window {
    form: Form,
    nav: [HWND; 6],
    brand: HWND,
    tagline: HWND,
    icon: HWND,
    title: HWND,
    subtitle: HWND,
    body: HWND,
    status: HWND,
    check: [HWND; 2],
    buttons: [HWND; 5],
    checks_shown: usize,
    buttons_shown: usize,
}
impl Window {
    fn new(parent: HWND) -> Result<Self, AppError> {
        let mut form = Form::new(parent, "WiniOS Settings", false, 960, 720)?;
        let brand = form.add(Control::Label, 0, "WiniOS")?;
        let tagline = form.add(Control::Label, 0, "PRIVATE CLIPBOARD")?;
        fluent::muted(tagline);
        let icon = form.add(Control::Icon, 0, "")?;
        let nav = [
            form.add(Control::Button, GENERAL, "&General")?,
            form.add(Control::Button, CONNECTION, "&Connection")?,
            form.add(Control::Button, CLIPBOARD, "C&lipboard")?,
            form.add(Control::Button, IMAGES, "&Images")?,
            form.add(Control::Button, PRIVACY, "&Privacy")?,
            form.add(Control::Button, ABOUT, "&About")?,
        ];
        for (hwnd, icon) in nav
            .iter()
            .zip([0xe713, 0xe968, 0xe77f, 0xe91b, 0xe72e, 0xe946])
        {
            fluent::navigation(*hwnd);
            fluent::glyph(*hwnd, icon);
        }
        let title = form.add(Control::Label, 0, "General")?;
        let subtitle = form.add(Control::Label, 0, "Your desktop. Your clipboard.")?;
        fluent::muted(subtitle);
        let body = form.add(Control::Text, 0, "")?;
        let status = form.add(Control::Text, 0, "Reading preferences…")?;
        let check = [
            form.add(Control::Check, STARTUP, "Start with Windows")?,
            form.add(Control::Check, LAUNCH, "Open Settings on manual launch")?,
        ];
        let buttons = [
            form.add(Control::Button, RELOAD, "Reload")?,
            form.add(Control::Button, RETRY, "Retry connection")?,
            form.add(Control::Button, TEST, "Test connection")?,
            form.add(Control::Button, IPHONE, "iPhone Setup")?,
            form.add(Control::Button, CHECK_UPDATES, "Check for updates")?,
        ];
        let mut window = Self {
            form,
            nav,
            brand,
            tagline,
            icon,
            title,
            subtitle,
            body,
            status,
            check,
            buttons,
            checks_shown: 2,
            buttons_shown: 4,
        };
        window.layout();
        Ok(window)
    }
    fn layout(&mut self) {
        self.form.refresh_fonts();
        let (w, h) = self.form.size();
        self.form.title(self.brand);
        self.form.title(self.title);
        self.form.artwork(self.icon, 48);
        self.form.place(self.icon, 20, 24, 48, 48);
        self.form.place(self.brand, 76, 28, 116, 40);
        self.form.place(self.tagline, 24, 80, 168, 24);
        for (index, hwnd) in self.nav.iter().enumerate() {
            self.form.place(*hwnd, 12, 112 + index as i32 * 40, 180, 36);
        }
        self.form.place(self.title, 232, 30, w - 260, 44);
        self.form.place(self.subtitle, 232, 80, w - 260, 30);
        let compact = h < 560;
        let content_top = if compact { 116 } else { 128 };
        let check_height = if compact { 40 } else { 56 };
        let button_height = if compact { 32 } else { 40 };
        let buttons_top = h - 96 - self.buttons_shown.div_ceil(2) as i32 * (button_height + 8);
        let checks_top = buttons_top
            - if self.checks_shown > 0 {
                12 + self.checks_shown as i32 * check_height
            } else {
                0
            };
        let body_bottom = checks_top - 16;
        self.form.place(
            self.body,
            240,
            content_top + 16,
            w - 280,
            (body_bottom - content_top - 32).max(24),
        );
        let mut panels = vec![windows::Win32::Foundation::RECT {
            left: 224,
            top: content_top,
            right: w - 24,
            bottom: body_bottom,
        }];
        for (index, hwnd) in self.check.iter().enumerate() {
            let top = checks_top + index as i32 * check_height;
            self.form
                .place(*hwnd, 240, top + 6, w - 280, check_height - 20);
            if index < self.checks_shown {
                panels.push(windows::Win32::Foundation::RECT {
                    left: 224,
                    top,
                    right: w - 24,
                    bottom: top + check_height - 8,
                });
            }
        }
        let half = (w - 264) / 2;
        for (index, hwnd) in self.buttons.iter().enumerate() {
            self.form.place(
                *hwnd,
                224 + (index as i32 % 2) * (half + 12),
                buttons_top + (index as i32 / 2) * (button_height + 8),
                half,
                button_height,
            );
        }
        self.form.place(self.status, 32, h - 70, w - 64, 42);
        panels.push(windows::Win32::Foundation::RECT {
            left: 20,
            top: h - 80,
            right: w - 20,
            bottom: h - 16,
        });
        self.form.panels(208, &panels);
    }
}
enum Job {
    Load,
    Startup,
    Launch(bool, Option<Vec<u8>>),
    Writes(bool, Vec<u8>),
    Images(crate::image::Options, Option<Vec<u8>>),
    Test,
    Copy(Export, WriteClient),
    Diagnostic(String, WriteClient),
    Updates,
    OpenRelease(updates::Release),
}
enum ResultKind {
    Loaded(Result<Snapshot, &'static str>),
    Saved(Result<Snapshot, &'static str>, &'static str),
    Check(setup::Check),
    Message(Result<&'static str, &'static str>),
    Updates(Result<Option<updates::Release>, &'static str>),
}
struct Work {
    handle: JoinHandle<()>,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone, Copy)]
pub enum Request {
    Retry,
    Iphone,
    UpdateNotice,
}
pub struct Controller {
    parent: HWND,
    path: PathBuf,
    window: Option<Window>,
    page: Page,
    saved: Option<Snapshot>,
    active_writes: Option<bool>,
    active_images: Option<crate::image::Options>,
    writer: Option<WriteClient>,
    work: Option<Work>,
    tx: mpsc::Sender<ResultKind>,
    rx: mpsc::Receiver<ResultKind>,
    message: String,
    auto_open: bool,
    initial_load: bool,
    last_render: String,
    update_text: String,
    update_release: Option<updates::Release>,
    automatic: auto_updates::Worker,
    automatic_enabled: bool,
    automatic_configured: bool,
}
impl Controller {
    pub fn new(parent: HWND, path: PathBuf, manual_launch: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let automatic = auto_updates::Worker::new(path.clone(), parent);
        Self {
            parent,
            path,
            window: None,
            page: Page::General,
            saved: None,
            active_writes: None,
            active_images: None,
            writer: None,
            work: None,
            tx,
            rx,
            message: "Reading preferences…".into(),
            auto_open: manual_launch,
            initial_load: true,
            last_render: String::new(),
            update_text:
                "Choose Check for updates to check the source. Nothing is downloaded or installed."
                    .into(),
            update_release: None,
            automatic,
            automatic_enabled: false,
            automatic_configured: false,
        }
    }
    pub fn refresh(&mut self) {
        self.automatic.refresh();
        if self.work.is_none() {
            self.start(Job::Load);
        }
    }
    pub fn prefers_settings(&self) -> bool {
        self.saved
            .as_ref()
            .is_some_and(|saved| saved.desktop.open_settings_on_launch)
    }
    pub fn show(&mut self) -> Result<(), AppError> {
        if self.window.is_none() {
            self.window = Some(Window::new(self.parent)?);
            self.last_render.clear();
            self.refresh();
        }
        self.window.as_ref().unwrap().form.show();
        Ok(())
    }
    pub fn startup(&self) -> Option<integration::StartupInfo> {
        self.saved.as_ref().map(|saved| saved.startup.clone())
    }
    pub fn set_writer(&mut self, writer: Option<WriteClient>) {
        if writer.is_none() {
            self.active_writes = None;
            self.active_images = None;
        }
        self.writer = writer;
    }
    pub fn set_active_writes(&mut self, enabled: bool) {
        self.active_writes = Some(enabled);
    }
    pub fn set_active_images(&mut self, options: crate::image::Options) {
        self.active_images = Some(options);
    }
    pub fn busy(&self) -> bool {
        self.work.is_some()
    }
    pub fn close(&mut self) {
        self.auto_open = false;
        self.initial_load = false;
        if let Some(work) = &self.work {
            work.cancel.store(true, Ordering::Release);
        }
        self.window.take();
    }
    pub fn dialog(&self, msg: &MSG) -> bool {
        if let Some(window) = &self.window {
            if msg.message == WM_KEYDOWN
                && [VK_UP.0 as usize, VK_DOWN.0 as usize].contains(&msg.wParam.0)
            {
                if let Some(index) = window.nav.iter().position(|hwnd| *hwnd == msg.hwnd) {
                    let next = if msg.wParam.0 == VK_DOWN.0 as usize {
                        (index + 1) % window.nav.len()
                    } else {
                        (index + window.nav.len() - 1) % window.nav.len()
                    };
                    unsafe {
                        let _ = SetFocus(Some(window.nav[next]));
                        SendMessageW(window.nav[next], BM_CLICK, None, None);
                    }
                    return true;
                }
            }
            if msg.message == WM_KEYDOWN
                && msg.wParam.0 == VK_ESCAPE.0 as usize
                && (msg.hwnd == window.form.hwnd
                    || unsafe { IsChild(window.form.hwnd, msg.hwnd) }.as_bool())
            {
                unsafe {
                    let _ = PostMessageW(Some(window.form.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                return true;
            }
        }
        self.window.as_ref().is_some_and(|w| w.form.dialog(msg))
    }
    pub fn toggle_startup(&mut self) {
        if self.work.is_none() {
            self.start(Job::Startup);
        }
    }
    fn start(&mut self, job: Job) {
        if self.work.is_some() {
            return;
        }
        let (tx, path, parent) = (self.tx.clone(), self.path.clone(), self.parent.0 as isize);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        self.message = "Working in the background…".into();
        let handle=std::thread::Builder::new().name("winios-settings".into()).spawn(move||{
            let runtime=||tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_|"Could not start the settings worker.");
            let result=match job{
                Job::Load=>ResultKind::Loaded(load(&path)),
                Job::Startup=>ResultKind::Saved(integration::toggle_startup(&path).map_err(|_|"Startup could not be changed. Use the installed app with default settings; review conflicts in Windows Startup Apps.").and_then(|_|load(&path)),"Startup registration updated. Windows Startup Apps policy remains in force."),
                Job::Launch(enabled,expected)=>ResultKind::Saved(settings::save_desktop(&path,expected.as_deref(),settings::DesktopOptions{open_settings_on_launch:enabled}).and_then(|_|load(&path)),"Launch behavior saved. Startup launches always stay in the tray."),
                Job::Writes(enabled,expected)=>ResultKind::Saved(settings::save_write_permission(&path,&expected,enabled).and_then(|_|load(&path)),"Write permission saved. Choose Apply & restart to change the active service; Pause is preserved."),
                Job::Images(options,expected)=>ResultKind::Saved(settings::save_images(&path,expected.as_deref(),options).and_then(|_|load(&path)),"Image preferences saved. Apply & restart activates them; Pause is preserved."),
                Job::Test=>{let check=match settings::load(&path){Ok(config)=>match runtime(){Ok(runtime)=>runtime.block_on(setup::test_connection(&setup::Profile::from_config(config),&worker_cancel)),Err(error)=>setup::Check{passed:false,message:error.into(),endpoint:None}},Err(error)=>setup::Check{passed:false,message:error.into(),endpoint:None}};ResultKind::Check(check)},
                Job::Copy(export,writer)=>ResultKind::Message(settings::load(&path).and_then(|config|runtime().and_then(|runtime|runtime.block_on(iphone::copy_export(setup::Profile::from_config(config),writer,export,&worker_cancel))))),
                Job::Diagnostic(text,writer)=>ResultKind::Message(runtime().and_then(|runtime|runtime.block_on(async{let copy=writer.copy_private(text);tokio::pin!(copy);loop{tokio::select!{result=&mut copy=>return result.map(|_|"Non-sensitive diagnostics copied privately.").map_err(|_|"Diagnostics copy failed or timed out."),_=tokio::time::sleep(std::time::Duration::from_millis(25))=>if worker_cancel.load(Ordering::Acquire){return Err("Copy cancelled.");}}}}))),
                Job::Updates=>ResultKind::Updates(updates::check(&path,&worker_cancel)),
                Job::OpenRelease(release)=>ResultKind::Message(updates::open(&path,&release,&worker_cancel)),
            };
            let _=tx.send(result);unsafe{let _=PostMessageW(Some(HWND(parent as *mut _)),fluent::WAKE,WPARAM(0),LPARAM(0));}
        });
        match handle {
            Ok(handle) => self.work = Some(Work { handle, cancel }),
            Err(_) => self.message = "Could not start background work. Retry.".into(),
        }
    }
    fn confirm(&self, message: &str) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        let message = fluent::wide(message);
        unsafe {
            MessageBoxW(
                Some(window.form.hwnd),
                PCWSTR(message.as_ptr()),
                w!("WiniOS — confirm change"),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            ) == IDYES
        }
    }
    pub fn drain(&mut self, model: &TrayModel) -> Vec<Request> {
        let mut requests = Vec::new();
        let mut events = Vec::new();
        if let Some(window) = &self.window {
            while let Some(action) = window.form.poll() {
                events.push(action);
            }
        }
        for event in events {
            if event.id == fluent::CLOSE {
                self.close();
                continue;
            }
            if event.id == fluent::ACTIVATED {
                self.refresh();
                continue;
            }
            if event.id == fluent::LAYOUT {
                if let Some(w) = &mut self.window {
                    w.layout();
                }
                self.last_render.clear();
                continue;
            }
            if event.id == fluent::THEME {
                if let Some(w) = &mut self.window {
                    w.form.refresh_theme();
                    w.layout();
                }
                self.last_render.clear();
                continue;
            }
            if [GENERAL, CONNECTION, CLIPBOARD, IMAGES, PRIVACY, ABOUT].contains(&event.id) {
                self.page = match event.id {
                    CONNECTION => Page::Connection,
                    CLIPBOARD => Page::Clipboard,
                    IMAGES => Page::Images,
                    PRIVACY => Page::Privacy,
                    ABOUT => Page::About,
                    _ => Page::General,
                };
                continue;
            }
            if event.id == SHARE {
                model.sharing.set_paused(!event.checked);
                self.message="Sharing changed immediately for this session. Tailscale and saved write permission are unchanged.".into();
                continue;
            }
            if self.work.is_some() {
                continue;
            }
            match event.id {
                IMAGE_ENABLE | IMAGE_LIMIT => {
                    if let Some(saved) = &self.saved {
                        let mut options = saved.images;
                        let expected = saved.images_raw.clone();
                        if event.id == IMAGE_ENABLE {
                            options.enabled = event.checked;
                        } else {
                            options.max_mib = match options.max_mib {
                                2 => 4,
                                4 => 8,
                                _ => 2,
                            };
                        }
                        if event.id!=IMAGE_ENABLE || !options.enabled || self.confirm("Enable image clipboard sharing?\n\nCopied screenshots and photos can be retrieved by authenticated clients. Image uploads also require the existing clipboard write permission. Images stay in memory.\n\nThe two Smart Shortcuts support the intended text and image flow; verify them on iPhone. This saves the preference; Apply & restart activates it. No network or credential settings change.\n\nSave image sharing permission?") {self.start(Job::Images(options,expected));} else {self.message="Image sharing was not changed.".into();self.last_render.clear();}
                    }
                }
                STARTUP => self.toggle_startup(),
                LAUNCH => {
                    if let Some(saved) = &self.saved {
                        self.start(Job::Launch(event.checked, saved.desktop_raw.clone()));
                    }
                }
                SHARE => {
                    model.sharing.set_paused(!event.checked);
                    self.message="Sharing changed immediately for this session. Tailscale and saved write permission are unchanged.".into();
                }
                WRITES => {
                    if let Some(saved) = &self.saved {
                        let expected = saved.raw.clone();
                        if self.confirm("Save the clipboard write permission?\n\nEnabling it lets authenticated iPhone requests replace the Windows clipboard. Disabling it blocks those writes after the service restarts.\n\nThis saves the preference only. Choose Apply & restart afterward; pending requests may be cancelled. Pause remains in force. No credential, certificate or Tailscale route changes.\n\nSave this permission?"){self.start(Job::Writes(event.checked,expected));} else {self.message="Write permission was not changed.".into();self.last_render.clear();}
                    }
                }
                COPY_ENDPOINT | TOKEN => {
                    if let Some(writer) = self.writer.clone() {
                        if event.id != TOKEN || self.confirm(iphone::TOKEN_WARNING) {
                            self.start(Job::Copy(
                                if event.id == TOKEN {
                                    Export::Token
                                } else {
                                    Export::Endpoint
                                },
                                writer,
                            ));
                        } else {
                            self.message = "Token was not read or copied.".into();
                        }
                    }
                }
                TEST => self.start(Job::Test),
                RETRY => requests.push(Request::Retry),
                IPHONE => requests.push(Request::Iphone),
                DIAGNOSTICS => {
                    if let Some(writer) = self.writer.clone() {
                        self.start(Job::Diagnostic(self.diagnostic(model), writer));
                    }
                }
                RELOAD => self.refresh(),
                CHECK_UPDATES => {
                    self.update_release = None;
                    self.update_text = "Checking the configured release source…".into();
                    self.start(Job::Updates);
                }
                OPEN_RELEASE => {
                    if let Some(release) = self.update_release.clone() {
                        self.start(Job::OpenRelease(release));
                    }
                }
                AUTO_UPDATES => {
                    self.automatic.set(event.checked);
                    self.message = "Saving automatic update preference…".into();
                }
                _ => {}
            }
        }
        while let Some(event) = self.automatic.poll() {
            match event {
                auto_updates::Event::State {
                    enabled,
                    configured,
                } => {
                    self.automatic_enabled = enabled;
                    self.automatic_configured = configured;
                }
                auto_updates::Event::Release(release, notice) => {
                    self.update_text = release.describe();
                    self.update_release = Some(release);
                    if notice {
                        requests.push(Request::UpdateNotice);
                    }
                }
                auto_updates::Event::Error(error) => {
                    self.message = error.into();
                }
            }
        }
        while let Ok(result) = self.rx.try_recv() {
            let cancelled = self
                .work
                .as_ref()
                .is_some_and(|work| work.cancel.load(Ordering::Acquire));
            if let Some(work) = self.work.take() {
                let _ = work.handle.join();
            }
            match result {
                ResultKind::Loaded(result) => {
                    match result {
                        Ok(saved) => {
                            let open = self.initial_load
                                && self.auto_open
                                && saved.desktop.open_settings_on_launch;
                            self.saved = Some(saved);
                            self.message="Preferences loaded. Live controls apply immediately; saved write permission requires restart.".into();
                            if open {
                                let _ = self.show();
                            }
                        }
                        Err(error) => self.message = error.into(),
                    };
                    self.initial_load = false;
                }
                ResultKind::Saved(result, message) => match result {
                    Ok(saved) => {
                        self.saved = Some(saved);
                        self.message = message.into();
                    }
                    Err(error) => self.message = error.into(),
                },
                ResultKind::Check(check) => self.message = check.message,
                ResultKind::Message(result) => {
                    self.message = match result {
                        Ok(message) | Err(message) => message.into(),
                    }
                }
                ResultKind::Updates(result) => {
                    self.update_release = None;
                    self.update_text = if cancelled {
                        "Update check cancelled. Check again when ready.".into()
                    } else {
                        match result {
                            Ok(Some(release)) => {
                                let text = release.describe();
                                self.update_release = Some(release);
                                text
                            }
                            Ok(None) => "Update source not configured. Add updates.json beside settings.json with github_repository set to the trusted publisher's public owner/repository. No request was sent.".into(),
                            Err(error) => error.into(),
                        }
                    };
                    self.message = "Update check finished. See About for the result; nothing was downloaded or installed.".into();
                }
            }
        }
        self.render(model);
        requests
    }
    fn diagnostic(&self, model: &TrayModel) -> String {
        format!(
            "WiniOS {}\r\nNative Win32 / Rustls / {}\r\nStatus: {}\r\nSharing paused: {}\r\nActive write permission: {}\r\nSaved write permission: {}\r\nActive image sharing: {}\r\nHTTPS endpoint: {}\r\n{}\r\nNext step: {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
            model.status_text(),
            model.sharing.is_paused(),
            on_off(self.active_writes),
            on_off(self.saved.as_ref().map(|s| s.writes)),
            on_off(self.active_images.map(|s| s.enabled)),
            model.endpoint.as_deref().unwrap_or("Not verified"),
            model.detail,
            model.guidance()
        )
    }
    pub fn render(&mut self, model: &TrayModel) {
        let signature = format!(
            "{:?}|{}|{:?}|{}|{}|{}|{}|{:?}|{:?}|{}|{}|{}|{}|{}",
            self.page,
            model.status_text(),
            model.endpoint,
            model.detail,
            model.quitting,
            model.retry_pending,
            self.message,
            self.active_writes,
            self.saved.as_ref().map(|s| (
                s.writes,
                s.desktop.open_settings_on_launch,
                s.startup.registered,
                s.startup.can_toggle,
                s.startup.note,
                s.images,
                self.active_images
            )),
            self.busy(),
            self.writer.is_some(),
            self.update_text,
            self.automatic_enabled,
            self.automatic_configured
        );
        if self.last_render == signature {
            return;
        }
        self.last_render = signature;
        let pending = self.saved.as_ref().is_some_and(|saved| {
            self.active_writes
                .is_some_and(|active| active != saved.writes)
                || self
                    .active_images
                    .is_some_and(|active| active != saved.images)
        });
        let busy = self.busy();
        let Some(window) = &mut self.window else {
            return;
        };
        fluent::text(window.title, self.page.name());
        fluent::text(
            window.subtitle,
            &format!(
                "{}   ·   {}",
                model.status_text(),
                if pending {
                    "Saved changes await service restart"
                } else {
                    "Private clipboard continuity"
                }
            ),
        );
        fluent::text(window.status, &self.message);
        for (index, nav) in window.nav.iter().enumerate() {
            unsafe {
                if index == self.page as usize {
                    let _ = SetPropW(
                        *nav,
                        w!("WiniOS.Selected"),
                        Some(windows::Win32::Foundation::HANDLE(1usize as *mut _)),
                    );
                } else {
                    let _ = RemovePropW(*nav, w!("WiniOS.Selected"));
                }
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(*nav), None, false);
            }
        }
        for hwnd in window.check.iter().chain(window.buttons.iter()) {
            fluent::visible(*hwnd, false);
        }
        let mut checks: Vec<(usize, &str, bool, bool)> = Vec::new();
        let mut buttons: Vec<(usize, &str, bool)> = Vec::new();
        let body = match self.page {
            Page::General => {
                let startup = self.saved.as_ref().map(|s| &s.startup);
                checks.push((
                    STARTUP,
                    "Start with Windows",
                    startup.is_some_and(|s| s.registered),
                    startup.is_some_and(|s| s.can_toggle),
                ));
                checks.push((
                    LAUNCH,
                    "Open Settings when I launch WiniOS",
                    self.saved
                        .as_ref()
                        .is_some_and(|s| s.desktop.open_settings_on_launch),
                    self.saved.is_some(),
                ));
                buttons.push((RELOAD, "Reload preferences", true));
                format!(
                    "YOUR DESKTOP, CONNECTED\r\n\r\nKeep WiniOS ready in the notification area. Closing Settings keeps clipboard sharing running.\r\n\r\nStartup: {}\r\n\r\nWindows startup launches always stay quiet. These preferences are saved; Windows Startup Apps policy remains in control.",
                    startup.map_or("Checking registration…", |s| s.note)
                )
            }
            Page::Connection => {
                buttons.extend([
                    (
                        COPY_ENDPOINT,
                        "Copy HTTPS endpoint",
                        self.writer.is_some() && model.endpoint.is_some(),
                    ),
                    (TEST, "Test HTTPS & authentication", true),
                    (RETRY, "Retry connection", !model.retry_pending),
                    (IPHONE, "iPhone Setup", true),
                ]);
                format!(
                    "PRIVATE HTTPS CONNECTION\r\n\r\n{}\r\n{}\r\n\r\n{}\r\n\r\nKeep Tailscale connected on your laptop and iPhone. Test verifies HTTPS and authentication without transferring clipboard text. Retry reconnects WiniOS; your network and certificate setup stay unchanged.",
                    model.status_text(),
                    model
                        .endpoint
                        .as_deref()
                        .unwrap_or("No verified HTTPS endpoint available"),
                    if model.detail.is_empty() {
                        model.guidance().into()
                    } else {
                        format!("{}\r\n{}", model.detail, model.guidance())
                    }
                )
            }
            Page::Clipboard => {
                checks.push((
                    SHARE,
                    "Clipboard sharing · applies immediately",
                    !model.sharing.is_paused(),
                    true,
                ));
                checks.push((
                    WRITES,
                    "Allow iPhone to write to this clipboard",
                    self.saved.as_ref().is_some_and(|s| s.writes),
                    self.saved.is_some(),
                ));
                buttons.push((
                    RETRY,
                    "Apply & restart service",
                    (pending || !model.worker_active)
                        && self.saved.is_some()
                        && !model.retry_pending,
                ));
                buttons.push((RELOAD, "Reload preferences", true));
                format!(
                    "SEAMLESS TEXT, IN BOTH DIRECTIONS\r\n\r\nShare Unicode text up to 1 MiB. Images are configured in the Images page; file transfer is not supported. Unavailable clipboard content is never replaced with stale text.\r\n\r\nWrite permission: {} now · {} saved\r\n{}\r\n\r\nSharing changes immediately for this session. Saved write permission requires confirmation and Apply & restart. Retry preserves Pause.",
                    on_off(self.active_writes),
                    on_off(self.saved.as_ref().map(|s| s.writes)),
                    if pending {
                        "Choose Apply & restart to apply the saved permission."
                    } else if self.active_writes.is_none() {
                        "Active permission is available once the service starts."
                    } else {
                        "Your active and saved permissions are up to date."
                    }
                )
            }
            Page::Images => {
                checks.push((
                    IMAGE_ENABLE,
                    "Share images and screenshots · saved",
                    self.saved.as_ref().is_some_and(|s| s.images.enabled),
                    self.saved.is_some(),
                ));
                buttons.push((
                    IMAGE_LIMIT,
                    "Cycle limit: 2 / 4 / 8 MiB",
                    self.saved.is_some(),
                ));
                buttons.push((
                    RETRY,
                    "Apply & restart service",
                    (pending || !model.worker_active)
                        && self.saved.is_some()
                        && !model.retry_pending,
                ));
                buttons.push((RELOAD, "Reload preferences", true));
                format!(
                    "SCREENSHOTS AND PHOTOS\r\n\r\nActive image sharing: {}\r\nSaved image sharing: {}\r\nPNG maximum: {} MiB active · {} MiB saved\r\n\r\nHard limits: 8192 pixels per dimension and 8 megapixels decoded. PNG only; animation and unsupported native bitmap layouts are rejected. Images remain in memory.\r\n\r\nEnable sharing, choose a limit, then Apply & restart. Image uploads also require Clipboard write permission. Pause blocks both text and images.\r\n\r\nInstall only WiniOS Send and WiniOS Receive from Setup. Verify image transfer on iPhone after enabling this setting.",
                    on_off(self.active_images.map(|o| o.enabled)),
                    on_off(self.saved.as_ref().map(|s| s.images.enabled)),
                    self.active_images.map_or(0, |o| o.max_mib),
                    self.saved.as_ref().map_or(8, |s| s.images.max_mib)
                )
            }
            Page::Privacy => {
                checks.push((
                    SHARE,
                    "Clipboard sharing · applies immediately",
                    !model.sharing.is_paused(),
                    true,
                ));
                buttons.push((
                    TOKEN,
                    "Copy bearer token…",
                    self.writer.is_some() && model.endpoint.is_some(),
                ));
                buttons.push((DIAGNOSTICS, "Copy safe diagnostics", self.writer.is_some()));
                "PRIVATE BY DESIGN\r\n\r\nClipboard contents stay in memory. WiniOS does not save or log them. Transfers require HTTPS and bearer authentication. Localhost alone does not authenticate other apps on this PC.\r\n\r\nYOUR CREDENTIAL, YOUR CHOICE\r\nThe token stays in Windows Credential Manager. Copy requires confirmation and is excluded from WiniOS transfers and Windows history/sync. Other local apps can still read the clipboard; replace the token after setup.\r\n\r\nPause lasts for this session. Quit and relaunch starts sharing again.".into()
            }
            Page::About => {
                checks.push((
                    AUTO_UPDATES,
                    "Automatically check for updates",
                    self.automatic_enabled,
                    self.automatic_configured || self.automatic_enabled,
                ));
                buttons.push((DIAGNOSTICS, "Copy safe diagnostics", self.writer.is_some()));
                buttons.push((IPHONE, "Setup & troubleshooting", true));
                buttons.push((TEST, "Run connection diagnostics", true));
                buttons.push((CHECK_UPDATES, "Check for updates", true));
                buttons.push((OPEN_RELEASE, "View Update", self.update_release.is_some()));
                format!(
                    "WiniOS {}\r\nYour clipboard, across devices.\r\nA lightweight, local-first Windows companion.\r\n\r\n{} · {}\r\nNative Rust / Win32 · Axum / Rustls\r\n\r\nNeed a hand? Keep both devices connected to Tailscale, your laptop awake and WiniOS running. Connection diagnostics verify the current setup without changing it.\r\nSafe diagnostics include version, status and endpoint only.\r\n\r\nUPDATES\r\n{}\r\n{}\r\n\r\nDeveloped by Abhishyanth\r\nHelp or questions: v.abhishyanth118@gmail.com",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::ARCH,
                    if cfg!(debug_assertions) {
                        "Debug build"
                    } else {
                        "Release build"
                    },
                    if self.automatic_configured {
                        "Source configured (public availability is checked on request)."
                    } else {
                        "Source not configured or invalid; configure a trusted public GitHub repository."
                    },
                    self.update_text
                )
            }
        };
        fluent::text(window.body, &body);
        let relayout = window.checks_shown != checks.len() || window.buttons_shown != buttons.len();
        window.checks_shown = checks.len();
        window.buttons_shown = buttons.len();
        for (index, (id, label, value, allowed)) in checks.iter().enumerate() {
            let hwnd = window.check[index];
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_ID, *id as isize);
            }
            fluent::text(hwnd, label);
            fluent::checked(hwnd, *value);
            fluent::enabled(hwnd, *allowed && (!busy || *id == SHARE) && !model.quitting);
            fluent::visible(hwnd, true);
        }
        for (index, (id, label, allowed)) in buttons.iter().enumerate() {
            let hwnd = window.buttons[index];
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_ID, *id as isize);
            }
            fluent::text(hwnd, label);
            fluent::glyph(
                hwnd,
                match *id {
                    RELOAD => 0xe72c,
                    RETRY => 0xe777,
                    TEST => 0xe73e,
                    IPHONE => 0xe8ea,
                    COPY_ENDPOINT => 0xe8c8,
                    TOKEN => 0xe72e,
                    DIAGNOSTICS => 0xe9d9,
                    CHECK_UPDATES => 0xe895,
                    OPEN_RELEASE => 0xe774,
                    _ => 0,
                },
            );
            fluent::enabled(hwnd, *allowed && !busy && !model.quitting);
            fluent::visible(hwnd, true);
        }
        if relayout {
            window.layout();
        }
    }
}
fn on_off(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Enabled",
        Some(false) => "Disabled",
        None => "Not known",
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
                w!("WiniOS UI test"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        }
    }
    fn fixture(parent: HWND) -> Controller {
        let mut controller = Controller::new(
            parent,
            PathBuf::from(r"C:\WiniOS\target\ui-fixture\settings.json"),
            false,
        );
        controller.window = Some(Window::new(parent).unwrap());
        controller.active_writes = Some(false);
        controller.saved = Some(Snapshot {
            raw: br#"{"mode":"tailnet-backend","allow_clipboard_write":false}"#.to_vec(),
            writes: false,
            desktop: settings::DesktopOptions::default(),
            desktop_raw: None,
            images: crate::image::Options::default(),
            images_raw: None,
            startup: integration::StartupInfo {
                registered: false,
                can_toggle: false,
                note: "Preview fixture — no registration changes",
            },
        });
        controller.message = "UI preview fixture. No service or network was started.".into();
        controller
    }
    #[test]
    fn safe_diagnostics_describe_permissions_and_recovery_without_reading_clipboard() {
        let mut controller = Controller::new(
            HWND::default(),
            PathBuf::from(r"C:\WiniOS\target\unused-diagnostic-fixture.json"),
            false,
        );
        let mut model = TrayModel::default();
        model.begin();
        model.apply(crate::lifecycle::Event::Connectivity(
            crate::tailnet::Report {
                state: crate::tailnet::State::Connected,
                endpoint: Some("https://example.example.ts.net".into()),
            },
        ));
        controller.active_writes = Some(false);
        controller.active_images = Some(crate::image::Options {
            enabled: true,
            ..Default::default()
        });
        let text = controller.diagnostic(&model);
        assert!(text.contains("Active write permission: Disabled"));
        assert!(text.contains("Saved write permission: Not known"));
        assert!(text.contains("Active image sharing: Enabled"));
        assert!(text.contains("HTTP 412"));
        assert!(!text.contains("Some("));
        model.sharing.set_paused(true);
        assert!(
            controller
                .diagnostic(&model)
                .contains("Retry does not resume")
        );
        assert!(
            controller.window.is_none() && controller.work.is_none() && controller.writer.is_none()
        );
    }
    #[test]
    fn update_action_uses_background_worker_without_config_changes_or_network() {
        let parent = parent();
        let mut controller = fixture(parent);
        let dir = std::env::temp_dir().join(format!("winios-update-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        controller.path = dir.join("settings.json");
        controller.page = Page::About;
        let model = TrayModel::default();
        controller.render(&model);
        let hwnd = controller.window.as_ref().unwrap().form.hwnd;
        unsafe {
            SendMessageW(hwnd, WM_COMMAND, Some(WPARAM(CHECK_UPDATES)), None);
        }
        assert!(controller.drain(&model).is_empty());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while controller.busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            controller.drain(&model);
        }
        assert!(!controller.busy());
        assert!(controller.update_text.contains("not configured"));
        assert!(controller.update_release.is_none());
        assert!(!dir.join("updates.json").exists() && !controller.path.exists());
        assert!(!model.sharing.is_paused());
        // Late completion of cancelled work must never enable a release action.
        controller.work = Some(Work {
            handle: std::thread::spawn(|| {}),
            cancel: Arc::new(AtomicBool::new(true)),
        });
        controller.tx.send(ResultKind::Updates(Ok(None))).unwrap();
        controller.drain(&model);
        assert!(controller.update_text.contains("cancelled"));
        assert!(controller.update_release.is_none());
        drop(controller);
        unsafe {
            let _ = DestroyWindow(parent);
        }
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn native_settings_navigation_live_pause_close_and_render_cache() {
        let parent = parent();
        let mut controller = fixture(parent);
        let model = TrayModel::default();
        for page in [
            Page::General,
            Page::Connection,
            Page::Clipboard,
            Page::Images,
            Page::Privacy,
            Page::About,
        ] {
            controller.page = page;
            controller.render(&model);
            assert!(!controller.last_render.is_empty());
            let key = controller.last_render.clone();
            controller.render(&model);
            assert_eq!(controller.last_render, key);
        }
        let hwnd = controller.window.as_ref().unwrap().form.hwnd;
        // About remains a native text surface; the contact footer is not a new
        // browser link, credential export, or layout/control change.
        let body = controller.window.as_ref().unwrap().body;
        let mut text = vec![0u16; 4096];
        let len = unsafe { GetWindowTextW(body, &mut text) } as usize;
        let about = String::from_utf16_lossy(&text[..len]);
        assert!(about.ends_with(
            "Developed by Abhishyanth\r\nHelp or questions: v.abhishyanth118@gmail.com"
        ));
        unsafe {
            SendMessageW(hwnd, WM_COMMAND, Some(WPARAM(CLIPBOARD)), None);
        }
        controller.drain(&model);
        assert_eq!(controller.page, Page::Clipboard);
        unsafe {
            SendMessageW(hwnd, WM_COMMAND, Some(WPARAM(SHARE)), None);
        }
        controller.drain(&model);
        assert!(model.sharing.is_paused());
        assert!(!controller.busy());
        unsafe {
            SendMessageW(hwnd, WM_CLOSE, None, None);
        }
        controller.drain(&model);
        assert!(controller.window.is_none());
        drop(controller);
        unsafe {
            let _ = DestroyWindow(parent);
        }
    }
    #[test]
    fn render_opaque_settings_fixtures_without_desktop_interaction() {
        let parent = parent();
        let mut controller = fixture(parent);
        let model = TrayModel::default();
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        for page in [
            Page::General,
            Page::Connection,
            Page::Clipboard,
            Page::Images,
            Page::Privacy,
            Page::About,
        ] {
            controller.page = page;
            controller.render(&model);
            let path = directory.join(format!("fluent-{}.bmp", page.name().to_lowercase()));
            controller
                .window
                .as_ref()
                .unwrap()
                .form
                .render_fixture_bitmap(&path)
                .unwrap();
        }
        drop(controller);
        unsafe {
            let _ = DestroyWindow(parent);
        }
    }
    #[test]
    fn cards_and_controls_fit_small_and_large_native_client_areas() {
        let parent = parent();
        let mut controller = fixture(parent);
        let model = TrayModel::default();
        for (width, height) in [(720, 480), (960, 720), (1440, 900)] {
            let window = controller.window.as_mut().unwrap();
            unsafe {
                SetWindowPos(
                    window.form.hwnd,
                    None,
                    0,
                    0,
                    fluent::scale(width, window.form.dpi),
                    fluent::scale(height, window.form.dpi),
                    SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
                )
                .unwrap();
            }
            window.layout();
            for page in [
                Page::General,
                Page::Connection,
                Page::Clipboard,
                Page::Images,
                Page::Privacy,
                Page::About,
            ] {
                controller.page = page;
                controller.last_render.clear();
                controller.render(&model);
                let window = controller.window.as_ref().unwrap();
                let rect = |hwnd| {
                    let mut value = windows::Win32::Foundation::RECT::default();
                    unsafe {
                        GetWindowRect(hwnd, &mut value).unwrap();
                    }
                    value
                };
                let body = rect(window.body);
                let footer = rect(window.status);
                assert!(body.top < body.bottom && body.left < body.right);
                for (index, hwnd) in window.check.iter().enumerate() {
                    if index < window.checks_shown {
                        let control = rect(*hwnd);
                        assert!(control.top >= body.bottom && control.bottom < footer.top);
                    }
                }
                for (index, hwnd) in window.buttons.iter().enumerate() {
                    if index < window.buttons_shown {
                        let control = rect(*hwnd);
                        assert!(control.top >= body.bottom && control.bottom < footer.top);
                    }
                }
            }
        }
        drop(controller);
        unsafe {
            let _ = DestroyWindow(parent);
        }
    }
    #[test]
    #[ignore = "Interactive, fixture-only UI preview. No clipboard/network service or credential export; closes after two minutes."]
    fn preview_native_settings_surface() {
        let parent = parent();
        let mut controller = fixture(parent);
        let model = TrayModel::default();
        let window = controller.window.as_ref().unwrap();
        fluent::text(window.form.hwnd, "WiniOS Settings — UI fixture preview");
        window.form.show();
        controller.render(&model);
        unsafe {
            let _ = SetTimer(Some(parent), 7701, 120000, None);
        }
        let mut msg = MSG::default();
        while controller.window.is_some() {
            let code = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if code <= 0 || (msg.message == WM_TIMER && msg.hwnd == parent && msg.wParam.0 == 7701)
            {
                break;
            }
            if !controller.dialog(&msg) {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            controller.drain(&model);
        }
        controller.close();
        drop(controller);
        unsafe {
            let _ = KillTimer(Some(parent), 7701);
            let _ = DestroyWindow(parent);
        }
    }
}
