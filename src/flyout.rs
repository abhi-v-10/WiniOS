//! Lightweight asynchronous native popup. Standard child controls retain UIA.
use crate::{
    fluent::{self, Control, Form},
    native::AppError,
    popup::{Kind, Row},
};
use windows::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::*,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
const WIDTH: i32 = 280;
const PAD: i32 = 10;
const GAP: i32 = 2;
fn row_height(kind: Kind) -> i32 {
    match kind {
        Kind::Header | Kind::Endpoint => 42,
        Kind::Footer => 18,
        Kind::Separator => 6,
        _ => 28,
    }
}
struct Window {
    form: Form,
    controls: Vec<(HWND, usize, bool)>,
    rows: Vec<Row>,
    endpoint_labels: Vec<(HWND, usize)>,
}
pub struct Controller {
    parent: HWND,
    window: Option<Window>,
}
impl Controller {
    pub fn new(parent: HWND) -> Self {
        Self {
            parent,
            window: None,
        }
    }
    pub fn open(&mut self, point: POINT, rows: Vec<Row>) -> Result<(), AppError> {
        self.open_inner(point, rows, true)
    }
    fn open_inner(&mut self, point: POINT, rows: Vec<Row>, show: bool) -> Result<(), AppError> {
        self.close();
        let mut form = Form::new(self.parent, "WiniOS", true, WIDTH, 300)?;
        let mut controls = Vec::new();
        let mut endpoint_labels = Vec::new();
        let mut panels = Vec::new();
        let mut y = 8;
        for row in &rows {
            if row.kind == Kind::Separator {
                y += row_height(row.kind);
                continue;
            }
            let role = matches!(row.kind, Kind::Action | Kind::Checkbox | Kind::Header);
            let action = role && !row.disabled;
            let label = match row.kind {
                Kind::Header => format!("WiniOS\r\n{}", row.secondary),
                Kind::Endpoint => row.label.clone(),
                Kind::Checkbox => row.label.clone(),
                _ => row.label.clone(),
            };
            let hwnd = form.add(
                if row.kind == Kind::Checkbox {
                    Control::Check
                } else if role {
                    Control::Button
                } else {
                    Control::Label
                },
                row.id,
                &label,
            )?;
            if row.kind == Kind::Endpoint {
                fluent::muted(hwnd);
                let address = form.add(Control::EllipsisLabel, 0, &row.secondary)?;
                endpoint_labels.push((address, row.id));
            }
            match row.kind {
                Kind::Header => fluent::brand(hwnd, self.parent),
                Kind::Action => fluent::glyph(
                    hwnd,
                    match row.id {
                        1001 => 0xe8bb,
                        1002 => 0xe72c,
                        1003 => {
                            if row.label.contains("Resume") {
                                0xe768
                            } else {
                                0xe769
                            }
                        }
                        1006 => 0xe713,
                        _ => 0xe946,
                    },
                ),
                _ => {}
            }
            let height = row_height(row.kind);
            form.place(hwnd, PAD, y, WIDTH - PAD * 2, height);
            // Endpoint is an informational label, never a scrollable edit box.
            fluent::enabled(hwnd, !row.disabled || row.kind == Kind::Endpoint);
            if row.kind == Kind::Checkbox {
                fluent::checked(hwnd, row.checked);
            }
            controls.push((hwnd, row.id, action));
            y += height + GAP;
        }
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return Err("Popup monitor is unavailable.".into());
        }
        unsafe {
            let _ = SetWindowPos(
                form.hwnd,
                None,
                point.x,
                point.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        form.refresh_fonts();
        if info.rcWork.right - info.rcWork.left < fluent::scale(WIDTH, form.dpi)
            || info.rcWork.bottom - info.rcWork.top < fluent::scale(y + 8, form.dpi)
        {
            return Err("Use the native scrolling menu on this small work area.".into());
        }
        let width = fluent::scale(WIDTH, form.dpi).min(info.rcWork.right - info.rcWork.left);
        let height = fluent::scale(y + 8, form.dpi).min(info.rcWork.bottom - info.rcWork.top);
        let (x, top) = position(point, width, height, info.rcWork);
        unsafe {
            SetWindowPos(
                form.hwnd,
                Some(HWND_TOPMOST),
                x,
                top,
                width,
                height,
                SWP_NOACTIVATE,
            )?;
        }
        // Positioning may move to a different DPI monitor. Relayout exactly once.
        let mut y = 8;
        let mut index = 0;
        for row in &rows {
            if row.kind == Kind::Separator {
                panels.push(windows::Win32::Foundation::RECT {
                    left: PAD,
                    top: y + 2,
                    right: WIDTH - PAD,
                    bottom: y + 3,
                });
                y += row_height(row.kind);
                continue;
            }
            let height = row_height(row.kind);
            if row.kind == Kind::Endpoint {
                form.place(controls[index].0, PAD + 8, y + 3, WIDTH - 2 * PAD - 16, 16);
                if let Some((address, _)) = endpoint_labels.iter().find(|(_, id)| *id == row.id) {
                    form.place(*address, PAD + 8, y + 20, WIDTH - 2 * PAD - 16, 18);
                }
            } else if row.kind == Kind::Checkbox {
                form.place(
                    controls[index].0,
                    PAD + 8,
                    y + 3,
                    WIDTH - 2 * PAD - 16,
                    height - 6,
                );
            } else {
                form.place(controls[index].0, PAD, y, WIDTH - PAD * 2, height);
            }
            index += 1;
            y += height + GAP;
        }
        form.panels(0, &panels);
        if show {
            form.show();
            if let Some((hwnd, _, _)) = controls.iter().find(|(_, _, enabled)| *enabled) {
                unsafe {
                    let _ = SetFocus(Some(*hwnd));
                }
            }
        }
        self.window = Some(Window {
            form,
            controls,
            rows,
            endpoint_labels,
        });
        Ok(())
    }
    pub fn close(&mut self) {
        self.window.take();
    }
    pub fn is_open(&self) -> bool {
        self.window.is_some()
    }
    pub fn refresh(&mut self, rows: Vec<Row>) {
        let Some(window) = &mut self.window else {
            return;
        };
        if window.rows.len() != rows.len() {
            return;
        }
        for (row, old) in rows.iter().zip(&window.rows) {
            if row.label == old.label
                && row.secondary == old.secondary
                && row.checked == old.checked
                && row.disabled == old.disabled
            {
                continue;
            }
            if let Some((hwnd, _, enabled)) =
                window.controls.iter_mut().find(|(_, id, _)| *id == row.id)
            {
                let label = match row.kind {
                    Kind::Header => format!("WiniOS\r\n{}", row.secondary),
                    Kind::Endpoint => row.label.clone(),
                    _ => row.label.clone(),
                };
                fluent::text(*hwnd, &label);
                if row.kind == Kind::Endpoint {
                    if let Some((address, _)) =
                        window.endpoint_labels.iter().find(|(_, id)| *id == row.id)
                    {
                        fluent::text(*address, &row.secondary);
                    }
                }
                fluent::enabled(*hwnd, !row.disabled || row.kind == Kind::Endpoint);
                *enabled = !row.disabled
                    && matches!(row.kind, Kind::Action | Kind::Checkbox | Kind::Header);
                if row.kind == Kind::Checkbox {
                    fluent::checked(*hwnd, row.checked);
                }
            }
        }
        window.rows = rows;
    }
    pub fn drain(&mut self) -> Option<usize> {
        let mut action = None;
        let mut close = false;
        if let Some(window) = &mut self.window {
            while let Some(event) = window.form.poll() {
                match event.id {
                    fluent::CLOSE | fluent::INACTIVE => close = true,
                    fluent::THEME => window.form.refresh_theme(),
                    fluent::LAYOUT => {}
                    id => {
                        if window.rows.iter().any(|r| r.id == id && !r.disabled) {
                            action = Some(id);
                            close = true;
                        }
                    }
                }
            }
        }
        if close {
            self.close();
        }
        action
    }
    pub fn dialog(&mut self, msg: &MSG) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        let child = msg.hwnd == window.form.hwnd
            || unsafe { IsChild(window.form.hwnd, msg.hwnd) }.as_bool();
        if !child {
            return false;
        }
        if msg.message == WM_KEYDOWN {
            if msg.wParam.0 == VK_ESCAPE.0 as usize {
                self.close();
                return true;
            }
            let directions = [
                VK_DOWN.0 as usize,
                VK_UP.0 as usize,
                VK_HOME.0 as usize,
                VK_END.0 as usize,
            ];
            if directions.contains(&msg.wParam.0) {
                let focus = unsafe { GetFocus() };
                let enabled = window
                    .controls
                    .iter()
                    .filter(|(_, _, enabled)| *enabled)
                    .map(|(hwnd, _, _)| *hwnd)
                    .collect::<Vec<_>>();
                if !enabled.is_empty() {
                    let current = enabled.iter().position(|hwnd| *hwnd == focus).unwrap_or(0);
                    let next = match msg.wParam.0 {
                        x if x == VK_DOWN.0 as usize => (current + 1) % enabled.len(),
                        x if x == VK_UP.0 as usize => (current + enabled.len() - 1) % enabled.len(),
                        x if x == VK_END.0 as usize => enabled.len() - 1,
                        _ => 0,
                    };
                    unsafe {
                        let _ = SetFocus(Some(enabled[next]));
                    }
                }
                return true;
            }
        }
        window.form.dialog(msg)
    }
}
pub fn position(
    point: POINT,
    width: i32,
    height: i32,
    work: windows::Win32::Foundation::RECT,
) -> (i32, i32) {
    let x = point
        .x
        .clamp(work.left, (work.right - width).max(work.left));
    let y = if point.y + height > work.bottom {
        point.y - height
    } else {
        point.y
    };
    (x, y.clamp(work.top, (work.bottom - height).max(work.top)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_branded_popup_renders_actions_dismissal_and_disabled_items() {
        let mut parent = Form::new(HWND::default(), "Tray fixture owner", false, 800, 640).unwrap();
        let logo = parent.add(Control::Icon, 0, "").unwrap();
        parent.artwork(logo, 32);
        unsafe {
            let icon = SendMessageW(logo, STM_GETICON, None, None);
            SendMessageW(
                parent.hwnd,
                WM_SETICON,
                Some(windows::Win32::Foundation::WPARAM(ICON_BIG as usize)),
                Some(windows::Win32::Foundation::LPARAM(icon.0)),
            );
        }
        let mut header = Row::action(1004, "WiniOS", "", false);
        header.kind = Kind::Header;
        header.secondary = "Connected".into();
        let mut endpoint = Row::action(1101, "PRIVATE HTTPS", "", true);
        endpoint.kind = Kind::Endpoint;
        endpoint.secondary = "https://fixture.example".into();
        let mut startup = Row::action(1005, "Start with Windows", "", true);
        startup.kind = Kind::Checkbox;
        let rows = vec![
            header,
            endpoint,
            Row::separator(1102),
            Row::action(1003, "Pause clipboard sharing", "", false),
            Row::action(1002, "Retry connection", "", false),
            Row::action(1006, "Open Settings", "", false),
            startup,
            Row::separator(1104),
            Row::action(1001, "Quit WiniOS", "", false),
        ];
        let mut popup = Controller::new(parent.hwnd);
        popup
            .open_inner(POINT { x: 100, y: 100 }, rows.clone(), false)
            .unwrap();
        assert_eq!(popup.drain(), None);
        let hwnd = popup.window.as_ref().unwrap().form.hwnd;
        assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
        let window = popup.window.as_ref().unwrap();
        let (width, height) = window.form.size();
        assert!(
            width <= WIDTH && height <= 280,
            "full menu must remain a compact popup"
        );
        let assert_plain_label = |label| unsafe {
            let mut class = [0u16; 16];
            let length = GetClassNameW(label, &mut class) as usize;
            assert_eq!(
                String::from_utf16_lossy(&class[..length]).to_ascii_uppercase(),
                "STATIC"
            );
            assert_eq!(
                GetWindowLongW(label, GWL_STYLE) as u32
                    & (WS_VSCROLL.0 | WS_HSCROLL.0 | WS_TABSTOP.0),
                0
            );
        };
        let caption = window
            .controls
            .iter()
            .find(|(_, id, _)| *id == 1101)
            .unwrap()
            .0;
        let address = window.endpoint_labels[0].0;
        assert_plain_label(caption);
        assert_plain_label(address);
        popup
            .window
            .as_ref()
            .unwrap()
            .form
            .render_fixture_bitmap(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/fluent-tray.bmp"),
            )
            .unwrap();
        let mut changed = rows.clone();
        let long_address = format!("https://{}.{}.ts.net", "x".repeat(63), "y".repeat(63));
        changed
            .iter_mut()
            .find(|row| row.id == 1101)
            .unwrap()
            .secondary = long_address.clone();
        popup.refresh(changed);
        assert_plain_label(address);
        assert_eq!(
            unsafe { GetWindowTextLengthW(address) } as usize,
            long_address.encode_utf16().count(),
            "ellipsis is presentation only; native accessibility keeps the full endpoint"
        );
        unsafe {
            SendMessageW(
                hwnd,
                WM_COMMAND,
                Some(windows::Win32::Foundation::WPARAM(1005)),
                None,
            );
        }
        assert_eq!(popup.drain(), None);
        assert!(popup.is_open());
        unsafe {
            SendMessageW(
                hwnd,
                WM_COMMAND,
                Some(windows::Win32::Foundation::WPARAM(1003)),
                None,
            );
        }
        assert_eq!(popup.drain(), Some(1003));
        assert!(!popup.is_open());
        popup
            .open_inner(POINT { x: 100, y: 100 }, rows, false)
            .unwrap();
        let hwnd = popup.window.as_ref().unwrap().form.hwnd;
        let escape = MSG {
            hwnd,
            message: WM_KEYDOWN,
            wParam: windows::Win32::Foundation::WPARAM(VK_ESCAPE.0 as usize),
            ..Default::default()
        };
        assert!(popup.dialog(&escape));
        assert!(!popup.is_open());
    }
    #[test]
    fn popup_position_respects_negative_origin_and_small_work_areas() {
        let work = windows::Win32::Foundation::RECT {
            left: -1920,
            top: -1080,
            right: 0,
            bottom: 0,
        };
        let (x, y) = position(POINT { x: -10, y: -10 }, 360, 400, work);
        assert!(x >= work.left && x + 360 <= work.right && y >= work.top && y + 400 <= work.bottom);
        let small = windows::Win32::Foundation::RECT {
            left: 10,
            top: 20,
            right: 200,
            bottom: 220,
        };
        assert_eq!(
            position(POINT { x: -1000, y: 9000 }, 360, 400, small),
            (10, 20)
        );
    }
}
