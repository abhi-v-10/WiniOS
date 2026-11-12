//! Shared native controls, tokens and documented DWM backdrops. No render loop.
use crate::{brand, native::AppError};
use std::{cell::Cell, sync::mpsc};
use windows::{
    Win32::{
        Foundation::{COLORREF, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{Dwm::*, Gdi::*},
        System::{LibraryLoader::GetModuleHandleW, Registry::*},
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            Controls::*,
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::*,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};
pub const WAKE: u32 = WM_APP + 48;
pub const CLOSE: usize = 2;
pub const LAYOUT: usize = 0xff01;
pub const THEME: usize = 0xff02;
pub const INACTIVE: usize = 0xff03;
pub const ACTIVATED: usize = 0xff04;
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
pub fn scale(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}
pub fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub background: COLORREF,
    pub surface: COLORREF,
    pub hover: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    pub border: COLORREF,
    pub accent: COLORREF,
    pub violet: COLORREF,
    pub selected: COLORREF,
}
#[derive(Clone, Copy)]
pub struct Appearance {
    pub dark: bool,
    pub contrast: bool,
    pub motion: bool,
    pub colors: Palette,
}
fn preference(name: PCWSTR, default: u32) -> u32 {
    let mut value = default;
    let mut bytes = 4;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut bytes),
        );
    }
    value
}
impl Appearance {
    pub fn detect() -> Self {
        let mut high = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        let known = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                high.cbSize,
                Some((&mut high as *mut HIGHCONTRASTW).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
            )
        }
        .is_ok();
        let contrast = !known || high.dwFlags.contains(HCF_HIGHCONTRASTON);
        let dark = preference(w!("AppsUseLightTheme"), 0) == 0;
        let mut motion = windows::core::BOOL(0);
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&mut motion as *mut windows::core::BOOL).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
            );
        }
        let colors = if contrast {
            unsafe {
                Palette {
                    background: COLORREF(GetSysColor(COLOR_WINDOW)),
                    surface: COLORREF(GetSysColor(COLOR_WINDOW)),
                    hover: COLORREF(GetSysColor(COLOR_HIGHLIGHT)),
                    text: COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
                    muted: COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
                    border: COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
                    accent: COLORREF(GetSysColor(COLOR_HIGHLIGHT)),
                    violet: COLORREF(GetSysColor(COLOR_HIGHLIGHT)),
                    selected: COLORREF(GetSysColor(COLOR_HIGHLIGHT)),
                }
            }
        } else if dark {
            Palette {
                background: rgb(brand::NAVY[0], brand::NAVY[1], brand::NAVY[2]),
                surface: rgb(brand::SURFACE[0], brand::SURFACE[1], brand::SURFACE[2]),
                hover: rgb(32, 44, 78),
                text: rgb(241, 246, 255),
                muted: rgb(164, 180, 210),
                border: rgb(45, 59, 91),
                accent: rgb(brand::CYAN[0], brand::CYAN[1], brand::CYAN[2]),
                violet: rgb(brand::VIOLET[0], brand::VIOLET[1], brand::VIOLET[2]),
                selected: rgb(
                    brand::BLUE[0] / 4 + 12,
                    brand::BLUE[1] / 4 + 16,
                    brand::BLUE[2] / 4 + 20,
                ),
            }
        } else {
            Palette {
                background: rgb(242, 245, 255),
                surface: rgb(255, 255, 255),
                hover: rgb(229, 237, 255),
                text: rgb(21, 30, 57),
                muted: rgb(79, 96, 130),
                border: rgb(210, 220, 240),
                accent: rgb(0, 92, 160),
                violet: rgb(113, 57, 199),
                selected: rgb(221, 235, 255),
            }
        };
        Self {
            dark,
            contrast,
            motion: motion.as_bool() && !contrast,
            colors,
        }
    }
}
// Ordinary GDI STATIC/EDIT/BUTTON controls do not maintain alpha on glass.
// Never extend DWM composition behind them: captures can look correct while
// live scanout loses their text. Opaque content is a deliberate compatibility
// policy, not a runtime heuristic based on screenshots or Windows preferences.
fn gdi_frame_policy() -> (DWM_SYSTEMBACKDROP_TYPE, MARGINS) {
    (DWMSBT_NONE, MARGINS::default())
}
pub fn backdrop(hwnd: HWND, appearance: Appearance, _transient: bool) -> bool {
    unsafe {
        let dark = appearance.dark as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const i32).cast(),
            4,
        );
        let disable_motion = (!appearance.motion) as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            (&disable_motion as *const i32).cast(),
            4,
        );
        let corner = if appearance.contrast {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_ROUND
        };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            4,
        );
        let (kind, margins) = gdi_frame_policy();
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&kind as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
            4,
        );
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        let border = appearance.colors.border;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&border as *const COLORREF).cast(),
            4,
        );
        false
    }
}

#[derive(Clone, Copy)]
pub struct Action {
    pub id: usize,
    pub checked: bool,
}
struct View {
    parent: HWND,
    tx: mpsc::Sender<Action>,
    appearance: Cell<Appearance>,
    brush: Cell<HBRUSH>,
    background: Cell<HBRUSH>,
    panels: Cell<[RECT; 4]>,
    sidebar: Cell<i32>,
    glass: Cell<bool>,
    popup: bool,
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const View;
        if !ptr.is_null() {
            let view = &*ptr;
            let post = |id, checked| {
                let _ = view.tx.send(Action { id, checked });
                let _ = PostMessageW(Some(view.parent), WAKE, WPARAM(0), LPARAM(0));
            };
            match msg {
                WM_COMMAND => {
                    if wp.0 >> 16 != BN_CLICKED as usize {
                        return LRESULT(0);
                    }
                    let source = HWND(lp.0 as *mut _);
                    post(
                        wp.0 & 0xffff,
                        !source.0.is_null()
                            && SendMessageW(source, BM_GETCHECK, None, None).0
                                == BST_CHECKED.0 as isize,
                    );
                    return LRESULT(0);
                }
                WM_CLOSE => {
                    let popup = GetLastActivePopup(hwnd);
                    if popup != hwnd && !popup.0.is_null() {
                        let _ = PostMessageW(Some(popup), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    post(CLOSE, false);
                    return LRESULT(0);
                }
                WM_ACTIVATE if view.popup && wp.0 & 0xffff == WA_INACTIVE as usize => {
                    post(INACTIVE, false);
                    return LRESULT(0);
                }
                WM_ACTIVATE if !view.popup && wp.0 & 0xffff != WA_INACTIVE as usize => {
                    post(ACTIVATED, false);
                }
                WM_SIZE => {
                    post(LAYOUT, false);
                    return LRESULT(0);
                }
                WM_DPICHANGED => {
                    let r = &*(lp.0 as *const RECT);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    post(LAYOUT, false);
                    return LRESULT(0);
                }
                WM_THEMECHANGED | WM_SETTINGCHANGE | WM_DWMCOMPOSITIONCHANGED => {
                    post(THEME, false);
                }
                WM_GETMINMAXINFO if !view.popup => {
                    let info = &mut *(lp.0 as *mut MINMAXINFO);
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    info.ptMinTrackSize.x = scale(720, dpi);
                    info.ptMinTrackSize.y = scale(480, dpi);
                    let mut monitor = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    if GetMonitorInfoW(
                        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                        &mut monitor,
                    )
                    .as_bool()
                    {
                        info.ptMinTrackSize.x = info
                            .ptMinTrackSize
                            .x
                            .min(monitor.rcWork.right - monitor.rcWork.left - 16);
                        info.ptMinTrackSize.y = info
                            .ptMinTrackSize
                            .y
                            .min(monitor.rcWork.bottom - monitor.rcWork.top - 16);
                    }
                    return LRESULT(0);
                }
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
                    let colors = view.appearance.get().colors;
                    let dc = HDC(wp.0 as *mut _);
                    let child = HWND(lp.0 as *mut _);
                    let background = !GetPropW(child, w!("WiniOS.Background")).0.is_null();
                    let muted = !GetPropW(child, w!("WiniOS.Muted")).0.is_null();
                    SetTextColor(dc, if muted { colors.muted } else { colors.text });
                    SetBkColor(
                        dc,
                        if background {
                            colors.background
                        } else {
                            colors.surface
                        },
                    );
                    return LRESULT(
                        if background {
                            view.background.get()
                        } else {
                            view.brush.get()
                        }
                        .0 as isize,
                    );
                }
                WM_ERASEBKGND => {
                    paint_surface(hwnd, HDC(wp.0 as *mut _), view);
                    return LRESULT(1);
                }
                WM_PAINT => {
                    let mut paint = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut paint);
                    paint_surface(hwnd, dc, view);
                    let _ = EndPaint(hwnd, &paint);
                    return LRESULT(0);
                }
                WM_DRAWITEM if wp.0 != 0 => {
                    paint_button(&*(lp.0 as *const DRAWITEMSTRUCT), view.appearance.get());
                    return LRESULT(1);
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}
fn paint_surface(hwnd: HWND, dc: HDC, view: &View) {
    unsafe {
        let old = SaveDC(dc);
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        FillRect(dc, &rect, view.background.get());
        let appearance = view.appearance.get();
        let colors = appearance.colors;
        let dpi = GetDpiForWindow(hwnd).max(96);
        let pen = CreatePen(PS_SOLID, 1, colors.border);
        SelectObject(dc, pen.into());
        SelectObject(dc, view.brush.get().into());
        for panel in view.panels.get() {
            if panel.right > panel.left && panel.bottom > panel.top {
                let _ = RoundRect(
                    dc,
                    panel.left,
                    panel.top,
                    panel.right,
                    panel.bottom,
                    scale(16, dpi),
                    scale(16, dpi),
                );
            }
        }
        let sidebar = view.sidebar.get();
        if sidebar > 0 {
            let _ = MoveToEx(dc, sidebar, scale(24, dpi), None);
            let _ = LineTo(dc, sidebar, rect.bottom - scale(24, dpi));
        }
        // A quiet, opaque brand accent. No glass, alpha blending or animation.
        if !appearance.contrast && !view.popup {
            let left = if sidebar > 0 {
                sidebar + scale(32, dpi)
            } else {
                scale(18, dpi)
            };
            let right = (left + scale(144, dpi)).min(rect.right - scale(18, dpi));
            let top = scale(if sidebar > 0 { 16 } else { 10 }, dpi);
            for step in 0..32 {
                let blend = |a: u32, b: u32| ((a * (31 - step) + b * step) / 31) as u8;
                let a = colors.accent.0;
                let b = colors.violet.0;
                let brush = CreateSolidBrush(rgb(
                    blend(a & 255, b & 255),
                    blend((a >> 8) & 255, (b >> 8) & 255),
                    blend((a >> 16) & 255, (b >> 16) & 255),
                ));
                let stripe = RECT {
                    left: left + (right - left) * step as i32 / 32,
                    right: left + (right - left) * (step + 1) as i32 / 32,
                    top,
                    bottom: top + scale(2, dpi),
                };
                FillRect(dc, &stripe, brush);
                let _ = DeleteObject(brush.into());
            }
        }
        let _ = RestoreDC(dc, old);
        let _ = DeleteObject(pen.into());
    }
}
unsafe extern "system" fn button_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEMOVE => {
                if GetPropW(hwnd, w!("WiniOS.Hover")).0.is_null() {
                    let _ = SetPropW(hwnd, w!("WiniOS.Hover"), Some(HANDLE(1usize as *mut _)));
                    let mut event = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut event);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            WM_MOUSELEAVE => {
                let _ = RemovePropW(hwnd, w!("WiniOS.Hover"));
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            WM_NCDESTROY => {
                let _ = RemovePropW(hwnd, w!("WiniOS.Hover"));
                let _ = RemoveWindowSubclass(hwnd, Some(button_proc), 1);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
fn paint_button(item: &DRAWITEMSTRUCT, appearance: Appearance) {
    unsafe {
        let dc = item.hDC;
        let old = SaveDC(dc);
        let colors = appearance.colors;
        let compact = !GetPropW(item.hwndItem, w!("WiniOS.Compact")).0.is_null();
        let navigation = !GetPropW(item.hwndItem, w!("WiniOS.Navigation")).0.is_null();
        let active = !GetPropW(item.hwndItem, w!("WiniOS.Selected")).0.is_null();
        let selected = item.itemState.0 & (ODS_SELECTED.0 | ODS_FOCUS.0) != 0
            || !GetPropW(item.hwndItem, w!("WiniOS.Hover")).0.is_null()
            || active;
        let tint = if active {
            colors.selected
        } else if (navigation || compact) && !selected {
            colors.background
        } else if selected {
            colors.hover
        } else {
            colors.surface
        };
        let brush = CreateSolidBrush(tint);
        let outside = CreateSolidBrush(colors.background);
        FillRect(dc, &item.rcItem, outside);
        let _ = DeleteObject(outside.into());
        let pen = CreatePen(
            PS_SOLID,
            1,
            if navigation || !selected {
                tint
            } else {
                colors.border
            },
        );
        SelectObject(dc, pen.into());
        SelectObject(dc, brush.into());
        let dpi = GetDpiForWindow(item.hwndItem).max(96);
        let _ = RoundRect(
            dc,
            item.rcItem.left + 1,
            item.rcItem.top + 1,
            item.rcItem.right - 1,
            item.rcItem.bottom - 1,
            scale(if compact { 6 } else { 12 }, dpi),
            scale(if compact { 6 } else { 12 }, dpi),
        );
        if active {
            let marker = CreateSolidBrush(colors.accent);
            let marker_rect = RECT {
                left: item.rcItem.left + scale(3, dpi),
                right: item.rcItem.left + scale(6, dpi),
                top: item.rcItem.top + scale(12, dpi),
                bottom: item.rcItem.bottom - scale(12, dpi),
            };
            FillRect(dc, &marker_rect, marker);
            let _ = DeleteObject(marker.into());
        }
        let mut rect = item.rcItem;
        rect.left += scale(if compact { 8 } else { 16 }, dpi);
        rect.right -= scale(if compact { 8 } else { 14 }, dpi);
        let mut value = [0u16; 1024];
        let count = GetWindowTextW(item.hwndItem, &mut value).max(0) as usize;
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(
            dc,
            if selected && appearance.contrast {
                COLORREF(GetSysColor(COLOR_HIGHLIGHTTEXT))
            } else if item.itemState.0 & ODS_DISABLED.0 != 0 {
                colors.muted
            } else {
                colors.text
            },
        );
        let font = SendMessageW(item.hwndItem, WM_GETFONT, None, None);
        if font.0 != 0 {
            SelectObject(dc, HGDIOBJ(font.0 as *mut _));
        }
        let brand = GetPropW(item.hwndItem, w!("WiniOS.Brand"));
        let glyph = GetPropW(item.hwndItem, w!("WiniOS.Glyph")).0 as usize;
        if !brand.0.is_null() {
            let size = scale(if compact { 24 } else { 40 }, dpi);
            let _ = DrawIconEx(
                dc,
                rect.left,
                (rect.top + rect.bottom - size) / 2,
                HICON(brand.0),
                size,
                size,
                0,
                None,
                DI_NORMAL,
            );
            rect.left += scale(if compact { 34 } else { 54 }, dpi);
        } else if let Some(symbol) = char::from_u32(glyph as u32).filter(|_| glyph != 0) {
            let icon_font = CreateFontW(
                -scale(if compact { 14 } else { 18 }, dpi),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH.0 as u32,
                w!("Segoe Fluent Icons"),
            );
            if !icon_font.0.is_null() {
                let previous = SelectObject(dc, icon_font.into());
                let mut units = [0u16; 2];
                let text = symbol.encode_utf16(&mut units);
                let mut icon_rect = rect;
                icon_rect.right = icon_rect.left + scale(if compact { 18 } else { 24 }, dpi);
                DrawTextW(
                    dc,
                    text,
                    &mut icon_rect,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
                SelectObject(dc, previous);
                let _ = DeleteObject(icon_font.into());
                rect.left += scale(if compact { 24 } else { 30 }, dpi);
            }
        }
        if count > 0 && !brand.0.is_null() {
            let label = String::from_utf16_lossy(&value[..count]);
            let mut lines = label.lines();
            let mut title = wide(lines.next().unwrap_or("WiniOS"));
            let mut detail = wide(lines.next().unwrap_or(""));
            let heading_font = CreateFontW(
                -scale(if compact { 14 } else { 20 }, dpi),
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH.0 as u32,
                w!("Segoe UI Variable Text"),
            );
            let old_font = SelectObject(dc, heading_font.into());
            let mut title_rect = rect;
            title_rect.top += scale(if compact { 3 } else { 8 }, dpi);
            title_rect.bottom = title_rect.top + scale(if compact { 19 } else { 28 }, dpi);
            let length = title.len() - 1;
            if length > 0 {
                DrawTextW(
                    dc,
                    &mut title[..length],
                    &mut title_rect,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }
            SelectObject(dc, old_font);
            let _ = DeleteObject(heading_font.into());
            if detail.len() > 1 {
                let mut detail_rect = rect;
                detail_rect.top = title_rect.bottom + scale(if compact { 0 } else { 2 }, dpi);
                SetTextColor(dc, colors.muted);
                let length = detail.len() - 1;
                DrawTextW(
                    dc,
                    &mut detail[..length],
                    &mut detail_rect,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }
        } else if count > 0 {
            let hidden = SendMessageW(item.hwndItem, WM_QUERYUISTATE, None, None).0
                & UISF_HIDEACCEL as isize
                != 0;
            DrawTextW(
                dc,
                &mut value[..count],
                &mut rect,
                DT_SINGLELINE
                    | DT_VCENTER
                    | DT_END_ELLIPSIS
                    | if hidden { DT_HIDEPREFIX } else { DT_LEFT },
            );
        }
        if item.itemState.0 & ODS_FOCUS.0 != 0 && item.itemState.0 & ODS_NOFOCUSRECT.0 == 0 {
            let mut focus = item.rcItem;
            let _ = InflateRect(&mut focus, -4, -4);
            let _ = DrawFocusRect(dc, &focus);
        }
        let _ = RestoreDC(dc, old);
        let _ = DeleteObject(pen.into());
        let _ = DeleteObject(brush.into());
    }
}

pub enum Control {
    Label,
    EllipsisLabel,
    Text,
    Button,
    Check,
    Icon,
}
pub struct Form {
    pub hwnd: HWND,
    view: Box<View>,
    rx: mpsc::Receiver<Action>,
    font: HFONT,
    title_font: HFONT,
    children: Vec<HWND>,
    icons: Vec<(HWND, u32, HICON)>,
    pub dpi: u32,
}
impl Form {
    pub fn new(
        parent: HWND,
        title: &str,
        popup: bool,
        width: i32,
        height: i32,
    ) -> Result<Self, AppError> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None)?.0);
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("WiniOSFluentWindow"),
                lpfnWndProc: Some(procedure),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0
                && windows::core::Error::from_thread().code()
                    != windows::core::HRESULT::from_win32(1410)
            {
                return Err(windows::core::Error::from_thread().into());
            }
            let appearance = Appearance::detect();
            let brush = CreateSolidBrush(appearance.colors.surface);
            let background = CreateSolidBrush(appearance.colors.background);
            if brush.0.is_null() || background.0.is_null() {
                let _ = DeleteObject(brush.into());
                let _ = DeleteObject(background.into());
                return Err("Could not create native UI palette.".into());
            }
            let (tx, rx) = mpsc::channel();
            let view = Box::new(View {
                parent,
                tx,
                appearance: Cell::new(appearance),
                brush: Cell::new(brush),
                background: Cell::new(background),
                panels: Cell::new([RECT::default(); 4]),
                sidebar: Cell::new(0),
                glass: Cell::new(false),
                popup,
            });
            let text = wide(title);
            let hwnd = match CreateWindowExW(
                if popup {
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST
                } else {
                    WS_EX_CONTROLPARENT | WS_EX_APPWINDOW
                },
                class.lpszClassName,
                PCWSTR(text.as_ptr()),
                if popup {
                    WS_POPUP | WS_BORDER | WS_CLIPCHILDREN
                } else {
                    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN
                },
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width,
                height,
                Some(parent),
                None,
                Some(instance),
                Some((&*view as *const View).cast()),
            ) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    let _ = DeleteObject(brush.into());
                    let _ = DeleteObject(background.into());
                    return Err(error.into());
                }
            };
            let mut form = Self {
                hwnd,
                view,
                rx,
                font: HFONT::default(),
                title_font: HFONT::default(),
                children: Vec::new(),
                icons: Vec::new(),
                dpi: 0,
            };
            form.refresh_theme();
            form.refresh_fonts();
            if !popup {
                let mut monitor = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(
                    MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                )
                .as_bool()
                {
                    let width =
                        scale(width, form.dpi).min(monitor.rcWork.right - monitor.rcWork.left - 24);
                    let height = scale(height, form.dpi)
                        .min(monitor.rcWork.bottom - monitor.rcWork.top - 24);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        monitor.rcWork.left
                            + (monitor.rcWork.right - monitor.rcWork.left - width) / 2,
                        monitor.rcWork.top
                            + (monitor.rcWork.bottom - monitor.rcWork.top - height) / 2,
                        width,
                        height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            let icon = SendMessageW(parent, WM_GETICON, Some(WPARAM(ICON_BIG as usize)), None);
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(icon.0)),
            );
            Ok(form)
        }
    }
    pub fn appearance(&self) -> Appearance {
        self.view.appearance.get()
    }
    pub fn refresh_theme(&mut self) {
        unsafe {
            let appearance = Appearance::detect();
            let brush = CreateSolidBrush(appearance.colors.surface);
            let background = CreateSolidBrush(appearance.colors.background);
            if !brush.0.is_null() && !background.0.is_null() {
                let old = self.view.brush.replace(brush);
                let old_background = self.view.background.replace(background);
                self.view.appearance.set(appearance);
                self.view
                    .glass
                    .set(backdrop(self.hwnd, appearance, self.view.popup));
                for child in &self.children {
                    let _ = SetWindowTheme(
                        *child,
                        if appearance.dark && !appearance.contrast {
                            w!("DarkMode_Explorer")
                        } else {
                            w!("")
                        },
                        PCWSTR::null(),
                    );
                }
                let _ = InvalidateRect(Some(self.hwnd), None, true);
                let _ = DeleteObject(old.into());
                let _ = DeleteObject(old_background.into());
            } else {
                let _ = DeleteObject(brush.into());
                let _ = DeleteObject(background.into());
            }
        }
    }
    pub fn refresh_fonts(&mut self) {
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd).max(96);
            if self.dpi == dpi {
                return;
            }
            let create = |size, weight| {
                CreateFontW(
                    -scale(size, dpi),
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
                    w!("Segoe UI Variable Text"),
                )
            };
            let body = create(if self.view.popup { 12 } else { 15 }, 400);
            let title = create(if self.view.popup { 14 } else { 27 }, 600);
            if body.0.is_null() || title.0.is_null() {
                if !body.0.is_null() {
                    let _ = DeleteObject(body.into());
                }
                if !title.0.is_null() {
                    let _ = DeleteObject(title.into());
                }
                self.dpi = dpi;
                return;
            }
            for hwnd in &self.children {
                SendMessageW(
                    *hwnd,
                    WM_SETFONT,
                    Some(WPARAM(body.0 as usize)),
                    Some(LPARAM(1)),
                );
                update_text_overflow(*hwnd);
            }
            for font in [self.font, self.title_font] {
                if !font.0.is_null() {
                    let _ = DeleteObject(font.into());
                }
            }
            self.font = body;
            self.title_font = title;
            self.dpi = dpi;
        }
    }
    pub fn add(&mut self, kind: Control, id: usize, label: &str) -> Result<HWND, AppError> {
        unsafe {
            let (class, style) = match kind {
                Control::Label => (w!("STATIC"), WINDOW_STYLE(0)),
                // SS_LEFTNOWORDWRAP | SS_ENDELLIPSIS | SS_NOPREFIX (winuser.h).
                // Native STATIC labels have no scrolling or editable text state.
                Control::EllipsisLabel => (w!("STATIC"), WINDOW_STYLE(0x400c | 0x80)),
                Control::Icon => (w!("STATIC"), WINDOW_STYLE(3)), // SS_ICON (winuser.h)
                Control::Text => (
                    w!("EDIT"),
                    WS_TABSTOP | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                ),
                Control::Button => (w!("BUTTON"), WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32)),
                Control::Check => (
                    w!("BUTTON"),
                    WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32 | BS_MULTILINE as u32),
                ),
            };
            let label = wide(label);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                PCWSTR(label.as_ptr()),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                1,
                1,
                Some(self.hwnd),
                Some(HMENU(id as *mut _)),
                Some(HINSTANCE(GetModuleHandleW(None)?.0)),
                None,
            )?;
            self.children.push(hwnd);
            if self.view.popup
                || matches!(
                    kind,
                    Control::Label | Control::EllipsisLabel | Control::Icon
                )
            {
                let _ = SetPropW(
                    hwnd,
                    w!("WiniOS.Background"),
                    Some(HANDLE(1usize as *mut _)),
                );
            }
            if self.view.popup {
                let _ = SetPropW(hwnd, w!("WiniOS.Compact"), Some(HANDLE(1usize as *mut _)));
            }
            if matches!(kind, Control::Icon) {
                let icon = SendMessageW(
                    self.view.parent,
                    WM_GETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    None,
                );
                SendMessageW(hwnd, STM_SETICON, Some(WPARAM(icon.0 as usize)), None);
            }
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(self.font.0 as usize)),
                Some(LPARAM(0)),
            );
            let _ = SetWindowTheme(
                hwnd,
                if self.appearance().dark && !self.appearance().contrast {
                    w!("DarkMode_Explorer")
                } else {
                    w!("")
                },
                PCWSTR::null(),
            );
            if matches!(kind, Control::Button) {
                let _ = SetWindowSubclass(hwnd, Some(button_proc), 1, 0);
            }
            Ok(hwnd)
        }
    }
    pub fn title(&self, hwnd: HWND) {
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(self.title_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
    pub fn artwork(&mut self, hwnd: HWND, size: i32) {
        if self
            .icons
            .iter()
            .any(|(owner, dpi, _)| *owner == hwnd && *dpi == self.dpi)
        {
            return;
        }
        unsafe {
            let Ok(module) = GetModuleHandleW(None) else {
                return;
            };
            if let Ok(icon) = LoadImageW(
                Some(HINSTANCE(module.0)),
                PCWSTR(1usize as *const u16),
                IMAGE_ICON,
                scale(size, self.dpi),
                scale(size, self.dpi),
                LR_DEFAULTCOLOR,
            ) {
                SendMessageW(hwnd, STM_SETICON, Some(WPARAM(icon.0 as usize)), None);
                if let Some((_, dpi, owned)) =
                    self.icons.iter_mut().find(|(owner, _, _)| *owner == hwnd)
                {
                    let old = std::mem::replace(owned, HICON(icon.0));
                    *dpi = self.dpi;
                    let _ = DestroyIcon(old);
                } else {
                    self.icons.push((hwnd, self.dpi, HICON(icon.0)));
                }
            }
        }
    }
    /// Card rectangles are DIPs, as are child-control layout coordinates.
    pub fn panels(&self, sidebar: i32, panels: &[RECT]) {
        let mut scaled = [RECT::default(); 4];
        for (to, from) in scaled.iter_mut().zip(panels) {
            *to = RECT {
                left: scale(from.left, self.dpi),
                top: scale(from.top, self.dpi),
                right: scale(from.right, self.dpi),
                bottom: scale(from.bottom, self.dpi),
            };
        }
        self.view.panels.set(scaled);
        self.view.sidebar.set(scale(sidebar, self.dpi));
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
    }
    pub fn place(&self, hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            let _ = MoveWindow(
                hwnd,
                scale(x, self.dpi),
                scale(y, self.dpi),
                scale(width, self.dpi),
                scale(height, self.dpi),
                true,
            );
            update_text_overflow(hwnd);
        }
    }
    pub fn size(&self) -> (i32, i32) {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        (
            (rect.right * 96 / self.dpi.max(96) as i32).max(1),
            (rect.bottom * 96 / self.dpi.max(96) as i32).max(1),
        )
    }
    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }
    pub fn poll(&self) -> Option<Action> {
        self.rx.try_recv().ok()
    }
    pub fn dialog(&self, msg: &MSG) -> bool {
        unsafe {
            if msg.hwnd != self.hwnd && !IsChild(self.hwnd, msg.hwnd).as_bool() {
                return false;
            }
            if msg.message == WM_KEYDOWN
                && msg.wParam.0 == VK_RETURN.0 as usize
                && IsWindowEnabled(msg.hwnd).as_bool()
            {
                let mut class = [0u16; 32];
                let count = GetClassNameW(msg.hwnd, &mut class) as usize;
                if String::from_utf16_lossy(&class[..count]).eq_ignore_ascii_case("BUTTON") {
                    SendMessageW(msg.hwnd, BM_CLICK, None, None);
                    return true;
                }
            }
            IsDialogMessageW(self.hwnd, msg).as_bool()
        }
    }
    #[cfg(test)]
    pub fn render_fixture_bitmap(&self, path: &std::path::Path) -> Result<(), AppError> {
        // WM_PRINT renders ONLY this test-owned HWND and its controls. It never
        // captures the desktop or another app, and never activates this window.
        unsafe {
            let appearance = self.appearance();
            self.view.appearance.set(appearance);
            self.view.glass.set(false);
            backdrop(self.hwnd, appearance, self.view.popup);
            let mut rect = RECT::default();
            GetClientRect(self.hwnd, &mut rect)?;
            let width = rect.right;
            let height = rect.bottom;
            let count = (width as usize) * (height as usize) * 4;
            let dc = GetDC(Some(self.hwnd));
            let memory = CreateCompatibleDC(Some(dc));
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: 40,
                    biWidth: width,
                    biHeight: height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(memory);
                    ReleaseDC(Some(self.hwnd), dc);
                    return Err(error.into());
                }
            };
            let old = SelectObject(memory, bitmap.into());
            SendMessageW(
                self.hwnd,
                WM_ERASEBKGND,
                Some(WPARAM(memory.0 as usize)),
                None,
            );
            // Render each owned child at client coordinates. DefWindowProc's
            // PRF_CHILDREN path includes the overlapped frame's origin, which
            // otherwise shifts the fixture by a caption height and hides layout bugs.
            let mut origin = windows::Win32::Foundation::POINT::default();
            let _ = ClientToScreen(self.hwnd, &mut origin);
            for child in &self.children {
                if GetWindowLongW(*child, GWL_STYLE) as u32 & WS_VISIBLE.0 == 0 {
                    continue;
                }
                let mut child_rect = RECT::default();
                GetWindowRect(*child, &mut child_rect)?;
                let saved = SaveDC(memory);
                let _ = SetViewportOrgEx(
                    memory,
                    child_rect.left - origin.x,
                    child_rect.top - origin.y,
                    None,
                );
                IntersectClipRect(
                    memory,
                    0,
                    0,
                    child_rect.right - child_rect.left,
                    child_rect.bottom - child_rect.top,
                );
                SendMessageW(
                    *child,
                    WM_PRINT,
                    Some(WPARAM(memory.0 as usize)),
                    Some(LPARAM(
                        (PRF_CLIENT | PRF_NONCLIENT | PRF_ERASEBKGND) as isize,
                    )),
                );
                let _ = RestoreDC(memory, saved);
            }
            let mut pixels = std::slice::from_raw_parts(bits.cast::<u8>(), count).to_vec();
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(Some(self.hwnd), dc);
            let mut file = vec![0x42, 0x4d];
            file.extend_from_slice(&((54 + count) as u32).to_le_bytes());
            file.extend_from_slice(&[0u8; 4]);
            file.extend_from_slice(&54u32.to_le_bytes());
            file.extend_from_slice(&40u32.to_le_bytes());
            file.extend_from_slice(&width.to_le_bytes());
            file.extend_from_slice(&height.to_le_bytes());
            file.extend_from_slice(&1u16.to_le_bytes());
            file.extend_from_slice(&32u16.to_le_bytes());
            file.extend_from_slice(&[0u8; 24]);
            file.extend(pixels);
            std::fs::write(path, file)?;
            Ok(())
        }
    }
}
impl Drop for Form {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            for (_, _, icon) in &self.icons {
                let _ = DestroyIcon(*icon);
            }
            for font in [self.font, self.title_font] {
                if !font.0.is_null() {
                    let _ = DeleteObject(font.into());
                }
            }
            let _ = DeleteObject(self.view.brush.get().into());
            let _ = DeleteObject(self.view.background.get().into());
        }
    }
}
pub fn text(hwnd: HWND, value: &str) {
    unsafe {
        let value = wide(value);
        let _ = SetWindowTextW(hwnd, PCWSTR(value.as_ptr()));
        update_text_overflow(hwnd);
    }
}
/// Reflow at full width first; reserve a scrollbar only for real vertical overflow.
/// Runs only on native text/size/font updates, never on a polling timer.
pub(crate) fn update_text_overflow(hwnd: HWND) {
    unsafe {
        let mut class = [0u16; 16];
        let count = GetClassNameW(hwnd, &mut class).max(0) as usize;
        if !String::from_utf16_lossy(&class[..count]).eq_ignore_ascii_case("EDIT") {
            return;
        }
        if GetWindowLongW(hwnd, GWL_STYLE) as u32 & ES_MULTILINE as u32 == 0 {
            return;
        }
        let pad = scale(10, GetDpiForWindow(hwnd).max(96));
        let format = || {
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_err() {
                return;
            }
            rect.left += pad;
            rect.top += pad;
            rect.right = (rect.right - pad).max(rect.left + 1);
            rect.bottom = (rect.bottom - pad).max(rect.top + 1);
            SendMessageW(
                hwnd,
                EM_SETRECTNP,
                None,
                Some(LPARAM((&rect as *const RECT) as isize)),
            );
        };
        let _ = ShowScrollBar(hwnd, SB_VERT, false);
        format();
        let mut rect = RECT::default();
        SendMessageW(
            hwnd,
            EM_GETRECT,
            None,
            Some(LPARAM((&mut rect as *mut RECT) as isize)),
        );
        let dc = GetDC(Some(hwnd));
        if dc.0.is_null() {
            return;
        }
        let font = SendMessageW(hwnd, WM_GETFONT, None, None);
        let old = if font.0 != 0 {
            SelectObject(dc, HGDIOBJ(font.0 as *mut _))
        } else {
            HGDIOBJ::default()
        };
        let mut metrics = TEXTMETRICW::default();
        let measured = GetTextMetricsW(dc, &mut metrics).as_bool();
        if font.0 != 0 {
            SelectObject(dc, old);
        }
        ReleaseDC(Some(hwnd), dc);
        let lines = SendMessageW(hwnd, EM_GETLINECOUNT, None, None).0.max(1) as i64;
        let overflow = GetWindowTextLengthW(hwnd) > 0
            && (!measured
                || lines * metrics.tmHeight.max(1) as i64 > (rect.bottom - rect.top).max(0) as i64);
        if overflow {
            let _ = ShowScrollBar(hwnd, SB_VERT, true);
            // GetClientRect already excludes the visible non-client scrollbar.
            // Do not subtract its width a second time.
            format();
        } else {
            // A formerly scrolled control can now fit in full after resizing.
            // Do not hide its scrollbar while leaving the first lines offscreen.
            let first = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0;
            if first > 0 {
                SendMessageW(hwnd, EM_LINESCROLL, None, Some(LPARAM(-first)));
            }
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}
pub fn enabled(hwnd: HWND, value: bool) {
    unsafe {
        let _ = EnableWindow(hwnd, value);
    }
}
pub fn visible(hwnd: HWND, value: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, if value { SW_SHOW } else { SW_HIDE });
    }
}
pub fn checked(hwnd: HWND, value: bool) {
    unsafe {
        SendMessageW(
            hwnd,
            BM_SETCHECK,
            Some(WPARAM(if value {
                BST_CHECKED.0
            } else {
                BST_UNCHECKED.0
            } as usize)),
            None,
        );
    }
}
pub fn glyph(hwnd: HWND, code: u32) {
    unsafe {
        let _ = SetPropW(
            hwnd,
            w!("WiniOS.Glyph"),
            Some(HANDLE(code as usize as *mut _)),
        );
    }
}
pub fn muted(hwnd: HWND) {
    unsafe {
        let _ = SetPropW(hwnd, w!("WiniOS.Muted"), Some(HANDLE(1usize as *mut _)));
    }
}
pub fn navigation(hwnd: HWND) {
    unsafe {
        let _ = SetPropW(
            hwnd,
            w!("WiniOS.Navigation"),
            Some(HANDLE(1usize as *mut _)),
        );
    }
}
pub fn brand(hwnd: HWND, owner: HWND) {
    unsafe {
        let icon = SendMessageW(owner, WM_GETICON, Some(WPARAM(ICON_BIG as usize)), None);
        if icon.0 != 0 {
            let _ = SetPropW(hwnd, w!("WiniOS.Brand"), Some(HANDLE(icon.0 as *mut _)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollbars_follow_real_overflow_text_changes_and_window_size() {
        let mut form = Form::new(HWND::default(), "Scroll fixture", false, 900, 700).unwrap();
        let body = form.add(Control::Text, 1, "").unwrap();
        let has_bar = || unsafe { GetWindowLongW(body, GWL_STYLE) as u32 & WS_VSCROLL.0 != 0 };
        form.place(body, 20, 20, 500, 100);
        text(body, "Short status message.");
        assert!(
            !has_bar(),
            "fitting text must not reserve a scrollbar gutter"
        );
        text(
            body,
            "One\r\nTwo\r\nThree\r\nFour\r\nFive\r\nSix\r\nSeven\r\nEight",
        );
        assert!(has_bar(), "overflow must remain accessible");
        unsafe {
            SendMessageW(body, EM_LINESCROLL, None, Some(LPARAM(3)));
        }
        assert!(unsafe { SendMessageW(body, EM_GETFIRSTVISIBLELINE, None, None) }.0 > 0);
        form.place(body, 20, 20, 500, 300);
        assert!(
            !has_bar(),
            "growing the window must remove the redundant scrollbar"
        );
        assert_eq!(
            unsafe { SendMessageW(body, EM_GETFIRSTVISIBLELINE, None, None) }.0,
            0
        );
        form.place(body, 20, 20, 500, 70);
        assert!(has_bar(), "shrinking the window must restore scrolling");
        text(body, &"Wrapped Unicode text: café 🦀. ".repeat(12));
        form.place(body, 20, 20, 140, 100);
        assert!(has_bar(), "soft-wrapped lines count as overflow too");
        form.place(body, 20, 20, 800, 300);
        assert!(
            !has_bar(),
            "measure at full width before deciding to reserve a gutter"
        );
        text(body, "Short again.");
        assert!(!has_bar());
        text(body, "");
        assert!(!has_bar());
        assert!(!unsafe { IsWindowVisible(form.hwnd) }.as_bool());
    }

    #[test]
    fn gdi_content_never_uses_full_client_glass_even_after_theme_refresh() {
        let (kind, margins) = gdi_frame_policy();
        assert_eq!(kind, DWMSBT_NONE);
        assert_eq!(
            [
                margins.cxLeftWidth,
                margins.cxRightWidth,
                margins.cyTopHeight,
                margins.cyBottomHeight
            ],
            [0; 4]
        );
        let mut form = Form::new(HWND::default(), "Opaque UI fixture", false, 800, 640).unwrap();
        let text = form
            .add(Control::Text, 1, "Always visible native text")
            .unwrap();
        form.place(text, 20, 20, 400, 80);
        for _ in 0..3 {
            form.refresh_theme();
            assert!(!form.view.glass.get());
            let mut actual = DWM_SYSTEMBACKDROP_TYPE::default();
            if unsafe {
                DwmGetWindowAttribute(
                    form.hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE,
                    (&mut actual as *mut DWM_SYSTEMBACKDROP_TYPE).cast(),
                    4,
                )
            }
            .is_ok()
            {
                assert_eq!(actual, DWMSBT_NONE);
            }
        }
        assert!(
            !unsafe { IsWindowVisible(form.hwnd) }.as_bool(),
            "fixture must never activate or capture the desktop"
        );
    }
}
