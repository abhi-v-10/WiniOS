//! Owner-drawn native menu: Windows retains focus, keyboard, dismissal, placement,
//! and accessibility. No custom input loop, networking, timers or web renderer.
use crate::native::AppError;
use std::cell::RefCell;
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW, MSAA_MENU_SIG, MSAAMENUINFO},
            Controls::*,
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Header,
    Endpoint,
    Action,
    Checkbox,
    Separator,
    Footer,
}
#[derive(Clone)]
pub struct Row {
    pub id: usize,
    pub label: String,
    pub secondary: String,
    pub kind: Kind,
    pub disabled: bool,
    pub checked: bool,
    pub glyph: &'static str,
}
impl Row {
    pub fn action(id: usize, label: &str, glyph: &'static str, disabled: bool) -> Self {
        Self {
            id,
            label: label.into(),
            secondary: String::new(),
            kind: Kind::Action,
            disabled,
            checked: false,
            glyph,
        }
    }
    pub fn separator(id: usize) -> Self {
        Self {
            kind: Kind::Separator,
            ..Self::action(id, "", "", true)
        }
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn color(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}
const BG: COLORREF = COLORREF(0x24100c);
const TEXT: COLORREF = COLORREF(0xfff6f1);
const MUTED: COLORREF = COLORREF(0xd2b4a4);
const ACCENT: COLORREF = COLORREF(0xffd936);

struct Font(HFONT);
impl Font {
    fn new(height: i32, bold: bool) -> Result<Self, AppError> {
        let font = unsafe {
            CreateFontW(
                -height,
                0,
                0,
                0,
                if bold { 600 } else { 400 },
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
        if font.0.is_null() {
            return Err("Could not create popup typography.".into());
        }
        Ok(Self(font))
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}
struct Brush(HBRUSH);
impl Brush {
    fn new(value: COLORREF) -> Result<Self, AppError> {
        let brush = unsafe { CreateSolidBrush(value) };
        if brush.0.is_null() {
            return Err("Could not create popup palette.".into());
        }
        Ok(Self(brush))
    }
}
impl Drop for Brush {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}

#[repr(C)]
struct AccessibleRow {
    // Must be first: Windows MSAA recognizes this header via dwItemData.
    info: MSAAMENUINFO,
    _text: Vec<u16>,
    row: Row,
}
impl AccessibleRow {
    fn new(row: Row) -> Box<Self> {
        let name = format!("{} {}", row.label.replace('&', ""), row.secondary);
        let mut text = wide(name.trim());
        let info = MSAAMENUINFO {
            dwMSAASignature: MSAA_MENU_SIG as u32,
            cchWText: (text.len() - 1) as u32,
            pszWText: PWSTR(text.as_mut_ptr()),
        };
        Box::new(Self {
            info,
            _text: text,
            row,
        })
    }
}
struct Theme {
    dpi: u32,
    width: i32,
    title: Font,
    body: Font,
    small: Font,
    bg: Brush,
    hover: Brush,
    line: Brush,
    accent: Brush,
    rows: Vec<Box<AccessibleRow>>,
    icon: HICON,
}
thread_local! { static ACTIVE: RefCell<Option<Theme>> = const { RefCell::new(None) }; }
fn scaled(value: i32, dpi: u32) -> i32 {
    ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32
}
fn row_height(kind: Kind, dpi: u32) -> u32 {
    scaled(
        match kind {
            Kind::Header => 76,
            Kind::Endpoint => 56,
            Kind::Action | Kind::Checkbox => 42,
            Kind::Separator => 12,
            Kind::Footer => 29,
        },
        dpi,
    ) as u32
}
fn popup_width(dpi: u32, work_width: i32) -> i32 {
    scaled(344, dpi).min((work_width - scaled(16, dpi)).max(96))
}

/// RAII scope pins item/accessibility data and GDI handles through TrackPopupMenu.
pub struct Popup {
    pub menu: HMENU,
    styled: bool,
}
impl Drop for Popup {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.menu);
        }
        if self.styled {
            ACTIVE.with(|active| {
                active.borrow_mut().take();
            });
        }
    }
}
impl Popup {
    pub fn new(
        owner: HWND,
        point: POINT,
        rows: Vec<Row>,
        icon: HICON,
        native: bool,
    ) -> Result<Self, AppError> {
        // Moving the hidden owner gives GetDpiForWindow the anchor monitor's DPI.
        unsafe {
            let _ = SetWindowPos(
                owner,
                None,
                point.x,
                point.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        let mut contrast = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        let contrast_known = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
            )
        }
        .is_ok();
        let native = native || !contrast_known || contrast.dwFlags.contains(HCF_HIGHCONTRASTON);
        let mut menu = Self {
            menu: unsafe { CreatePopupMenu()? },
            styled: false,
        };
        if !native {
            if let Ok(theme) = Theme::new(
                owner,
                point,
                rows.iter()
                    .map(|r| Row {
                        id: r.id,
                        label: r.label.clone(),
                        secondary: r.secondary.clone(),
                        kind: r.kind,
                        disabled: r.disabled,
                        checked: r.checked,
                        glyph: r.glyph,
                    })
                    .collect(),
                icon,
            ) {
                ACTIVE.with(|active| *active.borrow_mut() = Some(theme));
                menu.styled = true;
                // If any native setup fails, rebuild a plain menu, not a broken UI.
                if menu.populate_styled().is_ok() {
                    return Ok(menu);
                }
                drop(menu);
                menu = Self {
                    menu: unsafe { CreatePopupMenu()? },
                    styled: false,
                };
            }
        }
        for row in rows {
            unsafe {
                if row.kind == Kind::Separator {
                    AppendMenuW(menu.menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                    continue;
                }
                let text = if row.secondary.is_empty() {
                    row.label
                } else {
                    format!("{} — {}", row.label, row.secondary)
                };
                let mut text: String = text.chars().take(100).collect();
                if row.kind == Kind::Checkbox {
                    text = text.split(" — ").next().unwrap_or(&text).to_string();
                }
                let text = wide(&text);
                let mut flags = MF_STRING;
                if row.disabled {
                    flags |= MF_GRAYED;
                }
                if row.checked {
                    flags |= MF_CHECKED;
                }
                AppendMenuW(menu.menu, flags, row.id, PCWSTR(text.as_ptr()))?;
            }
        }
        Ok(menu)
    }
    fn populate_styled(&self) -> Result<(), AppError> {
        ACTIVE.with(|active| {
            let active = active.borrow();
            let theme = active.as_ref().ok_or("Missing popup theme.")?;
            unsafe {
                let info = MENUINFO {
                    cbSize: std::mem::size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND,
                    hbrBack: theme.bg.0,
                    ..Default::default()
                };
                SetMenuInfo(self.menu, &info)?;
                for row in &theme.rows {
                    let text = wide(&row.row.label);
                    let mut state = MFS_ENABLED;
                    if row.row.disabled {
                        state |= MFS_DISABLED;
                    }
                    if row.row.checked {
                        state |= MFS_CHECKED;
                    }
                    let info = MENUITEMINFOW {
                        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_FTYPE | MIIM_ID | MIIM_DATA | MIIM_STATE | MIIM_STRING,
                        fType: MFT_OWNERDRAW,
                        fState: state,
                        wID: row.row.id as u32,
                        dwItemData: (&**row as *const AccessibleRow) as usize,
                        dwTypeData: PWSTR(text.as_ptr() as *mut u16),
                        cch: (text.len() - 1) as u32,
                        ..Default::default()
                    };
                    InsertMenuItemW(self.menu, u32::MAX, true, &info)?;
                }
            }
            Ok(())
        })
    }
}
impl Theme {
    fn new(owner: HWND, point: POINT, rows: Vec<Row>, icon: HICON) -> Result<Self, AppError> {
        let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return Err("Could not locate popup monitor.".into());
        }
        Ok(Self {
            dpi,
            width: popup_width(dpi, info.rcWork.right - info.rcWork.left),
            title: Font::new(scaled(20, dpi), true)?,
            body: Font::new(scaled(14, dpi), false)?,
            small: Font::new(scaled(12, dpi), false)?,
            bg: Brush::new(BG)?,
            hover: Brush::new(color(32, 44, 78))?,
            line: Brush::new(color(45, 59, 91))?,
            accent: Brush::new(ACCENT)?,
            rows: rows.into_iter().map(AccessibleRow::new).collect(),
            icon,
        })
    }
    fn find(&self, data: usize) -> Option<&AccessibleRow> {
        self.rows
            .iter()
            .find(|row| (&***row as *const AccessibleRow) as usize == data)
            .map(|row| &**row)
    }
    fn text(&self, dc: HDC, value: &str, rect: RECT, font: &Font, tint: COLORREF, hidden: bool) {
        // Empty Vec data is a dangling pointer; native DrawTextW may inspect
        // WCHAR zero even with count=0 (the unchecked-checkbox glyph). Skip it.
        if value.is_empty() {
            return;
        }
        let mut text: Vec<u16> = value.encode_utf16().collect();
        let mut rect = rect;
        unsafe {
            SelectObject(dc, HGDIOBJ(font.0.0));
            SetTextColor(dc, tint);
            let mut format = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
            if hidden {
                format |= DT_HIDEPREFIX;
            }
            DrawTextW(dc, &mut text, &mut rect, format);
        }
    }
    fn draw(&self, item: &DRAWITEMSTRUCT, row: &Row) {
        let dc = item.hDC;
        let rect = item.rcItem;
        let d = |v| scaled(v, self.dpi);
        unsafe {
            let saved = SaveDC(dc);
            SetBkMode(dc, TRANSPARENT);
            FillRect(dc, &rect, self.bg.0);
            if row.kind == Kind::Separator {
                FillRect(
                    dc,
                    &RECT {
                        left: rect.left + d(18),
                        right: rect.right - d(18),
                        top: rect.top + d(5),
                        bottom: rect.top + d(6),
                    },
                    self.line.0,
                );
            } else {
                let highlighted =
                    !row.disabled && item.itemState.0 & (ODS_SELECTED.0 | ODS_HOTLIGHT.0) != 0;
                if highlighted {
                    SelectObject(dc, HGDIOBJ(self.hover.0.0));
                    SelectObject(dc, GetStockObject(NULL_PEN));
                    let _ = RoundRect(
                        dc,
                        rect.left + d(7),
                        rect.top + d(2),
                        rect.right - d(7),
                        rect.bottom - d(2),
                        d(10),
                        d(10),
                    );
                }
                let left = rect.left + d(20);
                let right = rect.right - d(20);
                let hide = item.itemState.0 & ODS_NOACCEL.0 != 0;
                match row.kind {
                    Kind::Header => {
                        let _ = DrawIconEx(
                            dc,
                            left,
                            rect.top + d(20),
                            self.icon,
                            d(34),
                            d(34),
                            0,
                            None,
                            DI_NORMAL,
                        );
                        self.text(
                            dc,
                            &row.label,
                            RECT {
                                left: left + d(48),
                                right,
                                top: rect.top + d(12),
                                bottom: rect.top + d(41),
                            },
                            &self.title,
                            TEXT,
                            hide,
                        );
                        self.text(
                            dc,
                            &row.secondary,
                            RECT {
                                left: left + d(48),
                                right,
                                top: rect.top + d(41),
                                bottom: rect.top + d(63),
                            },
                            &self.small,
                            ACCENT,
                            true,
                        );
                    }
                    Kind::Endpoint => {
                        self.text(
                            dc,
                            &row.label,
                            RECT {
                                left,
                                right,
                                top: rect.top + d(3),
                                bottom: rect.top + d(25),
                            },
                            &self.small,
                            MUTED,
                            true,
                        );
                        self.text(
                            dc,
                            &row.secondary,
                            RECT {
                                left,
                                right,
                                top: rect.top + d(25),
                                bottom: rect.bottom - d(6),
                            },
                            &self.small,
                            TEXT,
                            true,
                        );
                    }
                    Kind::Footer => self.text(
                        dc,
                        &row.label,
                        RECT {
                            left,
                            right,
                            ..rect
                        },
                        &self.small,
                        MUTED,
                        true,
                    ),
                    Kind::Action | Kind::Checkbox => {
                        if row.kind == Kind::Checkbox {
                            let tick = if row.checked { "✓" } else { "" };
                            let box_rect = RECT {
                                left,
                                right: left + d(17),
                                top: rect.top + d(13),
                                bottom: rect.top + d(30),
                            };
                            FrameRect(
                                dc,
                                &box_rect,
                                if row.checked {
                                    self.accent.0
                                } else {
                                    self.line.0
                                },
                            );
                            self.text(dc, tick, box_rect, &self.body, ACCENT, true);
                        } else {
                            self.text(
                                dc,
                                row.glyph,
                                RECT {
                                    left,
                                    right: left + d(23),
                                    ..rect
                                },
                                &self.body,
                                MUTED,
                                true,
                            );
                        }
                        self.text(
                            dc,
                            &row.label,
                            RECT {
                                left: left + d(34),
                                right,
                                ..rect
                            },
                            &self.body,
                            if row.disabled { MUTED } else { TEXT },
                            hide,
                        );
                    }
                    Kind::Separator => {}
                }
                if item.itemState.0 & ODS_FOCUS.0 != 0 && item.itemState.0 & ODS_NOFOCUSRECT.0 == 0
                {
                    let _ = DrawFocusRect(
                        dc,
                        &RECT {
                            left: rect.left + d(9),
                            right: rect.right - d(9),
                            top: rect.top + d(3),
                            bottom: rect.bottom - d(3),
                        },
                    );
                }
            }
            let _ = RestoreDC(dc, saved);
        }
    }
}

/// Sent owner-draw callbacks must be handled in WndProc, not only GetMessage.
pub unsafe fn handle_message(message: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    ACTIVE.with(|active| {
        let active = active.borrow();
        let theme = active.as_ref()?;
        match message {
            WM_MEASUREITEM if lp.0 != 0 => {
                let item = unsafe { &mut *(lp.0 as *mut MEASUREITEMSTRUCT) };
                if item.CtlType != ODT_MENU {
                    return None;
                }
                let row = theme.find(item.itemData)?;
                item.itemWidth = theme.width as u32;
                item.itemHeight = row_height(row.row.kind, theme.dpi);
                Some(LRESULT(1))
            }
            WM_DRAWITEM if lp.0 != 0 => {
                let item = unsafe { &*(lp.0 as *const DRAWITEMSTRUCT) };
                if item.CtlType != ODT_MENU {
                    return None;
                }
                let row = theme.find(item.itemData)?;
                theme.draw(item, &row.row);
                Some(LRESULT(1))
            }
            WM_MENUCHAR => {
                let key = char::from_u32((wp.0 & 0xffff) as u32)?.to_ascii_lowercase();
                let index = theme.rows.iter().position(|row| {
                    !row.row.disabled
                        && row
                            .row
                            .label
                            .split_once('&')
                            .and_then(|(_, tail)| tail.chars().next())
                            .is_some_and(|c| c.to_ascii_lowercase() == key)
                })?;
                Some(LRESULT(((MNC_EXECUTE as usize) << 16 | index) as isize))
            }
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe extern "system" fn track_test_proc(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        if message == WM_TIMER {
            unsafe {
                let _ = EndMenu();
            }
            return LRESULT(0);
        }
        if let Some(result) = unsafe { handle_message(message, wp, lp) } {
            return result;
        }
        unsafe { DefWindowProcW(hwnd, message, wp, lp) }
    }
    #[test]
    fn real_native_popup_dispatch_is_bounded_and_survives_dismissal() {
        unsafe {
            let instance = windows::Win32::Foundation::HINSTANCE(
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    .unwrap()
                    .0,
            );
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("WiniOSPopupDispatchTest"),
                lpfnWndProc: Some(track_test_proc),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!("Popup dispatch test"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )
            .unwrap();
            let icon = LoadIconW(None, IDI_APPLICATION).unwrap();
            for (checked, native) in [(false, false), (true, false), (false, false), (false, true)]
            {
                let rows = vec![
                    Row {
                        kind: Kind::Header,
                        secondary: "Connected".into(),
                        ..Row::action(10, "WiniOS", "", false)
                    },
                    Row::action(1, "&Pause Clipboard Sharing", "Ⅱ", false),
                    Row {
                        kind: Kind::Checkbox,
                        checked,
                        ..Row::action(2, "Start with &Windows", "", false)
                    },
                ];
                let popup = Popup::new(owner, POINT { x: 40, y: 40 }, rows, icon, native).unwrap();
                assert_ne!(SetTimer(Some(owner), 193, 250, None), 0);
                let _ = TrackPopupMenu(
                    popup.menu,
                    TPM_NONOTIFY | TPM_RETURNCMD,
                    40,
                    40,
                    None,
                    owner,
                    None,
                );
                let _ = KillTimer(Some(owner), 193);
                drop(popup);
            }
            let _ = DestroyWindow(owner);
            let _ = UnregisterClassW(class.lpszClassName, Some(instance));
        }
    }
    unsafe extern "system" fn test_window_proc(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, message, wp, lp) }
    }
    #[test]
    fn native_gdi_preview_and_owner_draw_callbacks() {
        unsafe {
            let instance = windows::Win32::Foundation::HINSTANCE(
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    .unwrap()
                    .0,
            );
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("WiniOSPopupPreviewTest"),
                lpfnWndProc: Some(test_window_proc),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!("WiniOS preview test"),
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
            let icon = HICON(
                LoadImageW(
                    Some(instance),
                    PCWSTR(1usize as *const u16),
                    IMAGE_ICON,
                    32,
                    32,
                    LR_DEFAULTCOLOR,
                )
                .unwrap()
                .0,
            );
            let header = Row {
                kind: Kind::Header,
                secondary: "Connected".into(),
                ..Row::action(10, "WiniOS", "", false)
            };
            let endpoint = Row {
                kind: Kind::Endpoint,
                secondary: "https://desktop.example.ts.net".into(),
                ..Row::action(11, "HTTPS endpoint", "", true)
            };
            let checkbox = Row {
                kind: Kind::Checkbox,
                checked: false,
                ..Row::action(4, "Start with &Windows", "", false)
            };
            let footer = Row {
                kind: Kind::Footer,
                ..Row::action(12, "Local-first. Yours to control.", "", true)
            };
            let rows = vec![
                header,
                endpoint,
                Row::separator(6),
                Row::action(1, "&Pause Clipboard Sharing", "Ⅱ", false),
                Row::action(2, "Re&try Connection", "↻", false),
                checkbox,
                Row::separator(7),
                Row::action(3, "&Quit WiniOS", "×", false),
                footer,
            ];
            let theme = Theme::new(owner, POINT { x: 0, y: 0 }, rows, icon).unwrap();
            let width = theme.width;
            let height: i32 = theme
                .rows
                .iter()
                .map(|r| row_height(r.row.kind, theme.dpi) as i32)
                .sum();
            let dc = CreateCompatibleDC(None);
            assert!(!dc.0.is_null());
            let bitmap_info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: 40,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(dc), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0)
                    .unwrap();
            let old = SelectObject(dc, HGDIOBJ(bitmap.0));
            let mut top = 0;
            for row in &theme.rows {
                let bottom = top + row_height(row.row.kind, theme.dpi) as i32;
                let item = DRAWITEMSTRUCT {
                    CtlType: ODT_MENU,
                    hDC: dc,
                    rcItem: RECT {
                        left: 0,
                        right: width,
                        top,
                        bottom,
                    },
                    itemState: if row.row.id == 1 {
                        ODS_SELECTED
                    } else {
                        ODS_FLAGS(0)
                    },
                    ..Default::default()
                };
                theme.draw(&item, &row.row);
                top = bottom;
            }
            let _ = GdiFlush();
            let length = (width * height * 4) as usize;
            let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), length).to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
            let mut bmp = Vec::from(*b"BM");
            bmp.extend_from_slice(&((54 + length) as u32).to_le_bytes());
            bmp.extend_from_slice(&[0; 4]);
            bmp.extend_from_slice(&54u32.to_le_bytes());
            bmp.extend_from_slice(&40u32.to_le_bytes());
            bmp.extend_from_slice(&width.to_le_bytes());
            bmp.extend_from_slice(&(-height).to_le_bytes());
            bmp.extend_from_slice(&[1, 0, 32, 0]);
            for value in [0, length as u32, 0, 0, 0, 0] {
                bmp.extend_from_slice(&value.to_le_bytes());
            }
            bmp.extend_from_slice(&pixels);
            std::fs::write(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/popup-preview.bmp"),
                bmp,
            )
            .unwrap();
            let data = (&*theme.rows[3] as *const AccessibleRow) as usize;
            ACTIVE.with(|active| *active.borrow_mut() = Some(theme));
            let mut measure = MEASUREITEMSTRUCT {
                CtlType: ODT_MENU,
                itemData: data,
                ..Default::default()
            };
            assert_eq!(
                handle_message(
                    WM_MEASUREITEM,
                    WPARAM(0),
                    LPARAM((&mut measure as *mut MEASUREITEMSTRUCT) as isize)
                ),
                Some(LRESULT(1))
            );
            assert_eq!(measure.itemWidth, width as u32);
            let key = handle_message(WM_MENUCHAR, WPARAM('p' as usize), LPARAM(0)).unwrap();
            assert_eq!((key.0 as u32 >> 16), MNC_EXECUTE);
            assert_eq!(key.0 as u32 & 0xffff, 3);
            ACTIVE.with(|active| {
                active.borrow_mut().take();
            });
            let _ = DestroyIcon(icon);
            let _ = DestroyWindow(owner);
            let _ = UnregisterClassW(class.lpszClassName, Some(instance));
        }
    }
    #[test]
    fn dpi_metrics_scale_and_fit_small_or_negative_origin_monitors() {
        assert_eq!(row_height(Kind::Action, 96), 42);
        assert_eq!(row_height(Kind::Action, 192), 84);
        assert_eq!(popup_width(96, 1920), 344);
        assert!(popup_width(288, 800) < 800);
        let row = AccessibleRow::new(Row::action(1, "&Pause Clipboard Sharing", "", false));
        assert_eq!(row.info.dwMSAASignature, MSAA_MENU_SIG as u32);
        assert_eq!(
            unsafe { row.info.pszWText.to_string().unwrap() },
            "Pause Clipboard Sharing"
        );
    }
}
