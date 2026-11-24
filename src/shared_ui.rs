//! Explicit per-shortcut consent before converting shared desktop shortcuts.
//! This modal owns its state independently of App, whose borrow may be held
//! by the caller. Choosing rows never changes files or starts elevation.
use crate::render::{rect, Canvas, BG, LAVENDER, MUTED, PANEL, TEXT};
use std::{cell::RefCell, ffi::c_void, path::PathBuf};
use windows::{
    core::w,
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{Dwm::*, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::WM_MOUSELEAVE,
            HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow},
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
};

const HEADER: f32 = 148.;
const ROW: f32 = 68.;
const FOOTER: f32 = 94.;
const PAGER: f32 = 36.;
const MAX_ROWS: usize = 8;
const EDGE: u32 = 0x30394d;
const SOFT: u32 = 0x252d40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Row(usize),
    Previous,
    Next,
    Cancel,
    Accept,
}

struct Row {
    path: PathBuf,
    label: String,
    checked: bool,
}

struct Dialog {
    rows: Vec<Row>,
    page: usize,
    page_size: usize,
    focus: Target,
    hover: Option<Target>,
    pressed: Option<Target>,
    canvas: Option<Canvas>,
    scale: f32,
    width: f32,
    height: f32,
    done: bool,
    result: Option<Vec<PathBuf>>,
}

impl Dialog {
    fn new(paths: &[PathBuf], page_size: usize, width: f32, height: f32, scale: f32) -> Self {
        Self {
            rows: paths
                .iter()
                .map(|path| Row {
                    path: path.clone(),
                    label: path
                        .file_stem()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    checked: true,
                })
                .collect(),
            page: 0,
            page_size: page_size.clamp(1, MAX_ROWS),
            focus: Target::Row(0),
            hover: None,
            pressed: None,
            canvas: None,
            scale,
            width,
            height,
            done: false,
            result: None,
        }
    }

    fn pages(&self) -> usize {
        self.rows.len().div_ceil(self.page_size).max(1)
    }

    fn visible(&self) -> std::ops::Range<usize> {
        let start = (self.page * self.page_size).min(self.rows.len());
        start..(start + self.page_size).min(self.rows.len())
    }

    fn choices(&self) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter(|row| row.checked)
            .map(|row| row.path.clone())
            .collect()
    }

    fn targets(&self) -> Vec<Target> {
        let mut targets: Vec<_> = self.visible().map(Target::Row).collect();
        if self.page > 0 {
            targets.push(Target::Previous);
        }
        if self.page + 1 < self.pages() {
            targets.push(Target::Next);
        }
        targets.extend([Target::Cancel, Target::Accept]);
        targets
    }

    fn tab(&mut self, backwards: bool) {
        let targets = self.targets();
        let index = targets
            .iter()
            .position(|target| *target == self.focus)
            .unwrap_or(0);
        self.focus = targets[if backwards {
            (index + targets.len() - 1) % targets.len()
        } else {
            (index + 1) % targets.len()
        }];
    }

    fn set_page(&mut self, page: usize) {
        self.page = page.min(self.pages() - 1);
        self.focus = Target::Row(self.visible().start);
        self.hover = None;
        self.pressed = None;
    }

    fn move_row_focus(&mut self, delta: isize) {
        let current = match self.focus {
            Target::Row(index) => index,
            _ => self.visible().start,
        };
        let index = current
            .saturating_add_signed(delta)
            .min(self.rows.len().saturating_sub(1));
        self.page = index / self.page_size;
        self.focus = Target::Row(index);
    }

    fn activate(&mut self, target: Target) {
        match target {
            Target::Row(index) => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.checked = !row.checked;
                    self.focus = target;
                }
            }
            Target::Previous => self.set_page(self.page.saturating_sub(1)),
            Target::Next => self.set_page(self.page + 1),
            Target::Cancel => {
                self.result = None;
                self.done = true;
            }
            Target::Accept => {
                self.result = Some(self.choices());
                self.done = true;
            }
        }
    }

    fn area(&self, target: Target) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
        match target {
            Target::Row(index) => rect(
                20.,
                HEADER + (index - self.visible().start) as f32 * ROW,
                self.width - 40.,
                ROW - 4.,
            ),
            Target::Previous => rect(20., self.height - FOOTER - PAGER + 2., 88., 30.),
            Target::Next => rect(
                self.width - 108.,
                self.height - FOOTER - PAGER + 2.,
                88.,
                30.,
            ),
            Target::Cancel => rect(self.width - 286., self.height - 58., 112., 36.),
            Target::Accept => rect(self.width - 162., self.height - 58., 142., 36.),
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        self.targets().into_iter().find(|target| {
            let r = self.area(*target);
            x >= r.left && x < r.right && y >= r.top && y < r.bottom
        })
    }

    unsafe fn paint(&mut self, hwnd: HWND) -> bool {
        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client).is_err() || client.right <= 0 || client.bottom <= 0 {
            return false;
        }
        let width = client.right as u32;
        let height = client.bottom as u32;
        let mut canvas = match self.canvas.take() {
            Some(mut canvas) => {
                canvas.resize(width, height);
                canvas
            }
            None => match Canvas::new(hwnd, width, height) {
                Ok(canvas) => canvas,
                Err(_) => return false,
            },
        };
        canvas.begin(BG);
        canvas.text(
            "Choose which shortcuts to make personal.",
            rect(24., 16., self.width - 48., 36.),
            22.,
            TEXT,
            true,
            false,
        );
        canvas.text(
            "Moved shortcuts stop appearing for other Windows accounts.",
            rect(24., 58., self.width - 48., 24.),
            13.,
            MUTED,
            false,
            false,
        );
        canvas.text(
            "Windows may ask for administrator approval.",
            rect(24., 82., self.width - 48., 24.),
            13.,
            MUTED,
            false,
            false,
        );
        canvas.text(
            "SHORTCUTS",
            rect(24., 117., self.width - 48., 22.),
            11.,
            MUTED,
            true,
            false,
        );
        for index in self.visible() {
            let target = Target::Row(index);
            let row = &self.rows[index];
            let r = self.area(target);
            canvas.fill(
                r,
                if self.hover == Some(target) {
                    SOFT
                } else {
                    PANEL
                },
                10.,
            );
            if self.focus == target {
                canvas.outline(r, LAVENDER, 10., 1.);
            }
            let check = rect(r.left + 14., r.top + 23., 18., 18.);
            if row.checked {
                canvas.fill(check, LAVENDER, 4.);
                canvas.text("✓", check, 14., 0x23203b, true, true);
            } else {
                canvas.outline(check, MUTED, 4., 1.4);
            }
            let icon = rect(r.left + 46., r.top + 14., 36., 36.);
            if !canvas.icon(&row.path.to_string_lossy(), icon) {
                canvas.fill(icon, SOFT, 8.);
                canvas.text("↗", icon, 23., MUTED, true, true);
            }
            canvas.text(
                &row.label,
                rect(r.left + 96., r.top + 6., r.right - r.left - 112., 26.),
                14.,
                TEXT,
                true,
                false,
            );
            canvas.text(
                if row.checked {
                    "Move into this folder"
                } else {
                    "Keep on desktop; add a reference"
                },
                rect(r.left + 96., r.top + 32., r.right - r.left - 112., 24.),
                12.,
                if row.checked { LAVENDER } else { MUTED },
                false,
                false,
            );
        }
        if self.pages() > 1 {
            canvas.text(
                &format!("{} / {}", self.page + 1, self.pages()),
                rect(
                    112.,
                    self.height - FOOTER - PAGER + 2.,
                    self.width - 224.,
                    30.,
                ),
                12.,
                MUTED,
                false,
                true,
            );
            if self.page > 0 {
                self.button(&canvas, Target::Previous, "‹ Previous", false);
            }
            if self.page + 1 < self.pages() {
                self.button(&canvas, Target::Next, "Next ›", false);
            }
        }
        let checked = self.rows.iter().filter(|row| row.checked).count();
        let kept = self.rows.len() - checked;
        canvas.fill(
            rect(20., self.height - FOOTER + 1., self.width - 40., 1.),
            EDGE,
            0.,
        );
        canvas.text(
            &format!("{checked} to move · {kept} to keep on desktop"),
            rect(24., self.height - FOOTER + 10., self.width - 48., 24.),
            12.,
            MUTED,
            false,
            false,
        );
        self.button(&canvas, Target::Cancel, "Cancel", false);
        self.button(
            &canvas,
            Target::Accept,
            if checked == 0 {
                "Add references"
            } else {
                "Move selected"
            },
            true,
        );
        let ok = canvas.end().is_ok();
        if ok {
            self.canvas = Some(canvas);
        }
        ok
    }

    unsafe fn button(&self, canvas: &Canvas, target: Target, label: &str, primary: bool) {
        let r = self.area(target);
        let hover = self.hover == Some(target);
        canvas.fill(
            r,
            if primary {
                if hover {
                    0xdbd2ff
                } else {
                    LAVENDER
                }
            } else if hover {
                0x343e55
            } else {
                SOFT
            },
            9.,
        );
        if self.focus == target {
            canvas.outline(
                rect(
                    r.left - 3.,
                    r.top - 3.,
                    r.right - r.left + 6.,
                    r.bottom - r.top + 6.,
                ),
                LAVENDER,
                11.,
                1.,
            );
        } else if !primary {
            canvas.outline(r, EDGE, 9., 1.);
        }
        canvas.text(
            label,
            r,
            13.,
            if primary { 0x23203b } else { TEXT },
            true,
            true,
        );
    }
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lp.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<Dialog>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if msg == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        BeginPaint(hwnd, &mut paint);
        if let Ok(mut state) = (*ptr).try_borrow_mut() {
            state.paint(hwnd);
        }
        let _ = EndPaint(hwnd, &paint);
        return LRESULT(0);
    }
    let Ok(mut state) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            state.scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
            state.width = (lp.0 as u32 & 0xffff) as f32 / state.scale;
            state.height = ((lp.0 as u32 >> 16) & 0xffff) as f32 / state.scale;
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = &*(lp.0 as *const RECT);
            state.scale = (wp.0 as u32 & 0xffff).max(96) as f32 / 96.;
            // SetWindowPos synchronously sends WM_SIZE. Release this dialog's
            // borrow so that message can update its logical client bounds.
            drop(state);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lp.0 as i16) as f32 / state.scale;
            let y = ((lp.0 >> 16) as i16) as f32 / state.scale;
            let hover = state.hit(x, y);
            if state.hover != hover {
                state.hover = hover;
                let _ = InvalidateRect(hwnd, None, false);
            }
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tracking);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            state.hover = None;
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_SETCURSOR if (lp.0 as u32 & 0xffff) == HTCLIENT => {
            let cursor = if state.hover.is_some() {
                IDC_HAND
            } else {
                IDC_ARROW
            };
            SetCursor(LoadCursorW(None, cursor).unwrap_or_default());
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let _ = SetFocus(hwnd);
            state.pressed = state.hit(
                (lp.0 as i16) as f32 / state.scale,
                ((lp.0 >> 16) as i16) as f32 / state.scale,
            );
            if let Some(target) = state.pressed {
                state.focus = target;
                SetCapture(hwnd);
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let hit = state.hit(
                (lp.0 as i16) as f32 / state.scale,
                ((lp.0 >> 16) as i16) as f32 / state.scale,
            );
            let pressed = state.pressed.take();
            let _ = ReleaseCapture();
            if let Some(target) = pressed.filter(|target| Some(*target) == hit) {
                state.activate(target);
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            state.pressed = None;
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) as u16) as i16;
            let page = if delta < 0 {
                state.page + 1
            } else {
                state.page.saturating_sub(1)
            };
            state.set_page(page);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wp.0 as u16 {
                0x1b => state.activate(Target::Cancel),
                0x09 => state.tab(GetKeyState(VK_SHIFT.0 as i32) < 0),
                0x20 if lp.0 & (1 << 30) == 0 => {
                    let target = state.focus;
                    state.activate(target);
                }
                0x0d => {
                    let target = if matches!(state.focus, Target::Row(_)) {
                        Target::Accept
                    } else {
                        state.focus
                    };
                    state.activate(target);
                }
                0x26 => state.move_row_focus(-1),
                0x28 => state.move_row_focus(1),
                0x21 => {
                    let page = state.page.saturating_sub(1);
                    state.set_page(page);
                }
                0x22 => {
                    let page = state.page + 1;
                    state.set_page(page);
                }
                0x24 => state.set_page(0),
                0x23 => {
                    let page = state.pages() - 1;
                    state.set_page(page);
                }
                _ => return DefWindowProcW(hwnd, msg, wp, lp),
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_CHAR => LRESULT(0),
        WM_CLOSE => {
            state.activate(Target::Cancel);
            LRESULT(0)
        }
        WM_DESTROY => {
            state.canvas = None;
            state.done = true;
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// None cancels the entire operation. Some(empty) explicitly keeps every shared
/// shortcut on the Desktop and adds references. All rows start selected.
pub unsafe fn choose(owner: HWND, paths: &[PathBuf]) -> Option<Vec<PathBuf>> {
    if paths.is_empty() {
        return Some(Vec::new());
    }
    let dpi = if owner.is_invalid() {
        GetDpiForSystem()
    } else {
        GetDpiForWindow(owner)
    }
    .max(96);
    let scale = dpi as f32 / 96.;
    let work = crate::platform::primary_work_area();
    let available_h = ((work.bottom - work.top) as f32 / scale - 64.).max(HEADER + FOOTER + ROW);
    let page_size =
        (((available_h - HEADER - FOOTER - PAGER) / ROW).floor() as usize).clamp(1, MAX_ROWS);
    let rows = paths.len().min(page_size);
    let width = 700f32
        .min((work.right - work.left) as f32 / scale - 40.)
        .max(460.);
    let height =
        HEADER + rows as f32 * ROW + FOOTER + if paths.len() > page_size { PAGER } else { 0. };
    let state = Box::new(RefCell::new(Dialog::new(
        paths, page_size, width, height, scale,
    )));
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let class = WNDCLASSW {
        lpfnWndProc: Some(dialog_proc),
        hInstance: instance.into(),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        lpszClassName: w!("NestDesktopSharedChoices"),
        ..Default::default()
    };
    RegisterClassW(&class);
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    let ex = WS_EX_DLGMODALFRAME;
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: (width * scale).round() as i32,
        bottom: (height * scale).round() as i32,
    };
    AdjustWindowRectExForDpi(&mut frame, style, false, ex, dpi).ok()?;
    let window_w = frame.right - frame.left;
    let window_h = frame.bottom - frame.top;
    let hwnd = CreateWindowExW(
        ex,
        w!("NestDesktopSharedChoices"),
        w!("Shared desktop shortcuts"),
        style,
        work.left + ((work.right - work.left - window_w) / 2).max(0),
        work.top + ((work.bottom - work.top - window_h) / 2).max(0),
        window_w,
        window_h,
        owner,
        None,
        instance,
        Some((&*state as *const RefCell<Dialog>).cast()),
    )
    .ok()?;
    // This window's user-data is a state pointer, not App's surface-kind tag.
    // Configure DWM directly, avoiding the desktop tile's layered-window path.
    let dark = BOOL(1);
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        &dark as *const _ as *const c_void,
        std::mem::size_of::<BOOL>() as u32,
    );
    let corner = DWMWCP_ROUND;
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &corner as *const _ as *const c_void,
        std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
    );
    crate::brand::apply_window(hwnd);
    let ready = state.borrow_mut().paint(hwnd);
    if !ready {
        state.borrow_mut().canvas = None;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let _ = DestroyWindow(hwnd);
        return None;
    }
    let enabled = !owner.is_invalid() && IsWindowEnabled(owner).as_bool();
    if enabled {
        let _ = EnableWindow(owner, false);
    }
    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
    let _ = SetFocus(hwnd);
    let mut quit = None;
    while !state.borrow().done {
        let mut message = MSG::default();
        let result = GetMessageW(&mut message, None, 0, 0).0;
        if result <= 0 {
            if result == 0 {
                quit = Some(message.wParam.0 as i32);
            }
            break;
        }
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    let selected = state.borrow_mut().result.take();
    // HWND targets and their icon bitmaps must release before window teardown.
    state.borrow_mut().canvas = None;
    if enabled {
        let _ = EnableWindow(owner, true);
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    if IsWindow(hwnd).as_bool() {
        let _ = DestroyWindow(hwnd);
    }
    if !owner.is_invalid() && IsWindow(owner).as_bool() && enabled {
        let _ = SetForegroundWindow(owner);
        let _ = SetFocus(owner);
    }
    if let Some(code) = quit {
        PostQuitMessage(code);
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(count: usize) -> Vec<PathBuf> {
        (0..count)
            .map(|index| PathBuf::from(format!(r"C:\SyntheticPublicDesktop\App {index}.lnk")))
            .collect()
    }

    #[test]
    fn explicit_selection_returns_only_checked_original_paths() {
        let paths = paths(3);
        let mut dialog = Dialog::new(&paths, 8, 700., 446., 1.);
        assert_eq!(dialog.choices(), paths);
        dialog.activate(Target::Row(1));
        dialog.activate(Target::Accept);
        assert_eq!(
            dialog.result,
            Some(vec![paths[0].clone(), paths[2].clone()])
        );
        assert!(dialog.done);
    }

    #[test]
    fn keep_references_is_distinct_from_cancel() {
        let paths = paths(1);
        let mut dialog = Dialog::new(&paths, 8, 700., 310., 1.);
        dialog.activate(Target::Row(0));
        dialog.activate(Target::Accept);
        assert_eq!(dialog.result, Some(Vec::new()));
        let mut cancelled = Dialog::new(&paths, 8, 700., 310., 1.);
        cancelled.activate(Target::Cancel);
        assert!(cancelled.done);
        assert!(cancelled.result.is_none());
    }

    #[test]
    fn keyboard_and_page_hits_remain_bound_to_the_visible_rows() {
        let mut dialog = Dialog::new(&paths(19), 8, 700., 822., 1.);
        assert_eq!(dialog.visible(), 0..8);
        dialog.set_page(2);
        assert_eq!(dialog.visible(), 16..19);
        assert_eq!(dialog.hit(120., HEADER + 12.), Some(Target::Row(16)));
        assert_eq!(dialog.hit(120., HEADER + ROW * 3. + 12.), None);
        dialog.move_row_focus(-1);
        assert_eq!(dialog.page, 1);
        assert_eq!(dialog.focus, Target::Row(15));
        dialog.set_page(50);
        assert_eq!(dialog.page, 2);
        dialog.focus = Target::Accept;
        dialog.tab(false);
        assert_eq!(dialog.focus, Target::Row(16));
    }
}
