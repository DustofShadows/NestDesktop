//! Scroll geometry and the native scrollbar for the folder overview.
use crate::*;
use std::ops::Range;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

const LEFT: f32 = 238.;
const TOP: f32 = 158.;
const FOOTER: f32 = 107.;
const CARD_HEIGHT: f32 = 170.;
const ROW_GAP: f32 = 18.;
const COLUMN_GAP: f32 = 16.;
const LINE: i32 = 30;
const ID_MANAGER_SCROLL: usize = 608;

pub struct Overview {
    pub viewport: D2D_RECT_F,
    pub scrollbar: D2D_RECT_F,
    pub columns: usize,
    pub card_width: f32,
    pub content_height: i32,
    pub max_scroll: i32,
}

impl Overview {
    pub fn new(width: f32, height: f32, count: usize) -> Self {
        let bottom = (height - FOOTER).max(TOP);
        let available = (width - LEFT - 32.).max(1.);
        let columns = ((available + COLUMN_GAP) / (210. + COLUMN_GAP))
            .floor()
            .clamp(1., 3.) as usize;
        let rows = count.div_ceil(columns);
        let content_height = if rows == 0 {
            0
        } else {
            (rows as f32 * (CARD_HEIGHT + ROW_GAP) - ROW_GAP).ceil() as i32
        };
        let max_scroll = (content_height - (bottom - TOP).floor() as i32).max(0);
        let right = (width - 32. - if max_scroll > 0 { 20. } else { 0. }).max(LEFT + 1.);
        let viewport = render::rect(LEFT, TOP, right - LEFT, bottom - TOP);
        Self {
            viewport,
            scrollbar: render::rect(width - 24., TOP, 12., bottom - TOP),
            columns,
            card_width: ((right - LEFT - COLUMN_GAP * (columns - 1) as f32) / columns as f32)
                .max(1.),
            content_height,
            max_scroll,
        }
    }

    pub fn clamp(&self, offset: i32) -> i32 {
        offset.clamp(0, self.max_scroll)
    }

    pub fn card(&self, index: usize, offset: i32) -> D2D_RECT_F {
        render::rect(
            self.viewport.left + (index % self.columns) as f32 * (self.card_width + COLUMN_GAP),
            TOP + (index / self.columns) as f32 * (CARD_HEIGHT + ROW_GAP)
                - self.clamp(offset) as f32,
            self.card_width,
            CARD_HEIGHT,
        )
    }

    pub fn visible(&self, offset: i32, count: usize) -> Range<usize> {
        let offset = self.clamp(offset) as f32;
        let first = (offset / (CARD_HEIGHT + ROW_GAP)).floor() as usize * self.columns;
        let last = ((offset + self.viewport.bottom - self.viewport.top) / (CARD_HEIGHT + ROW_GAP))
            .ceil() as usize
            * self.columns;
        first.min(count)..last.min(count)
    }
}

pub fn clipped(rect: D2D_RECT_F, viewport: D2D_RECT_F) -> Option<D2D_RECT_F> {
    let r = D2D_RECT_F {
        left: rect.left.max(viewport.left),
        top: rect.top.max(viewport.top),
        right: rect.right.min(viewport.right),
        bottom: rect.bottom.min(viewport.bottom),
    };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

fn native_color(hex: u32) -> COLORREF {
    COLORREF((hex & 0x00ff00) | ((hex & 0xff) << 16) | ((hex >> 16) & 0xff))
}

/// The native control can draw directly during its tracking loop. Supply its
/// classic background as well as our WM_PAINT skin, without borrowing App.
pub(crate) unsafe fn scrollbar_background(dc: HDC) -> LRESULT {
    SetBkColor(dc, native_color(render::BG));
    SetDCBrushColor(dc, native_color(render::BG));
    LRESULT(GetStockObject(DC_BRUSH).0 as isize)
}

unsafe fn paint_scrollbar(hwnd: HWND, dc: HDC) {
    let saved = SaveDC(dc);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let brush = HBRUSH(GetStockObject(DC_BRUSH).0);
    SetDCBrushColor(dc, native_color(render::BG));
    FillRect(dc, &client, brush);
    let mut info = SCROLLBARINFO {
        cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
        ..Default::default()
    };
    if GetScrollBarInfo(hwnd, OBJID_CLIENT, &mut info).is_err() {
        if saved != 0 {
            let _ = RestoreDC(dc, saved);
        }
        return;
    }
    let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
    let width = client.right.max(1);
    let height = client.bottom.max(1);
    let inset = (2. * scale).round().max(1.) as i32;
    let arrow = info.dxyLineButton.max(0).min(height / 2);
    let mut pointer = POINT::default();
    let _ = GetCursorPos(&mut pointer);
    let _ = ScreenToClient(hwnd, &mut pointer);
    let inside = pointer.x >= 0 && pointer.x < width && pointer.y >= 0 && pointer.y < height;
    SelectObject(dc, GetStockObject(DC_BRUSH));
    SelectObject(dc, GetStockObject(NULL_PEN));
    let track_width = (3. * scale).round().max(2.) as i32;
    let track_left = (width - track_width) / 2;
    SetDCBrushColor(dc, native_color(0x242c3e));
    let _ = RoundRect(
        dc,
        track_left,
        arrow,
        track_left + track_width,
        height - arrow,
        track_width,
        track_width,
    );
    let thumb_top = info.xyThumbTop.clamp(arrow, height - arrow);
    let thumb_bottom = info.xyThumbBottom.clamp(thumb_top, height - arrow);
    if thumb_bottom > thumb_top {
        let over = inside && pointer.y >= thumb_top && pointer.y < thumb_bottom;
        let pressed =
            info.rgstate[3] & STATE_SYSTEM_PRESSED.0 != 0 || (GetCapture() == hwnd && over);
        SetDCBrushColor(
            dc,
            native_color(if pressed {
                render::LAVENDER
            } else if over {
                0xa6a9c4
            } else {
                0x707d97
            }),
        );
        if GetFocus() == hwnd {
            SelectObject(dc, GetStockObject(DC_PEN));
            SetDCPenColor(dc, native_color(render::LAVENDER));
        }
        let diameter = (width - inset * 2).max(2);
        let _ = RoundRect(
            dc,
            inset,
            thumb_top,
            (width - inset).max(inset + 1),
            thumb_bottom,
            diameter,
            diameter,
        );
        SelectObject(dc, GetStockObject(NULL_PEN));
    }
    for (top, bottom, state, up) in [
        (0, arrow, info.rgstate[1], true),
        (height - arrow, height, info.rgstate[5], false),
    ] {
        if bottom <= top {
            continue;
        }
        let hot = inside && pointer.y >= top && pointer.y < bottom;
        if state & STATE_SYSTEM_PRESSED.0 != 0 {
            SetDCBrushColor(dc, native_color(0x30384e));
            let _ = RoundRect(dc, 0, top, width, bottom, inset * 3, inset * 3);
        }
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCPenColor(
            dc,
            native_color(if state & STATE_SYSTEM_UNAVAILABLE.0 != 0 {
                0x505a70
            } else if hot {
                render::LAVENDER
            } else {
                render::MUTED
            }),
        );
        let center_x = width / 2;
        let center_y = (top + bottom) / 2;
        let gap = (2.5 * scale).round().max(2.) as i32;
        let direction = if up { -1 } else { 1 };
        let _ = MoveToEx(dc, center_x - gap, center_y - direction, None);
        let _ = LineTo(dc, center_x, center_y + direction * (gap - 1));
        let _ = LineTo(dc, center_x + gap, center_y - direction);
        SelectObject(dc, GetStockObject(NULL_PEN));
    }
    if saved != 0 {
        let _ = RestoreDC(dc, saved);
    }
}

unsafe fn repaint_scrollbar(hwnd: HWND) {
    if IsWindow(hwnd).as_bool() && IsWindowVisible(hwnd).as_bool() {
        let _ = RedrawWindow(
            hwnd,
            None,
            None,
            RDW_INVALIDATE | RDW_UPDATENOW | RDW_NOERASE | RDW_NOCHILDREN,
        );
    }
}

/// Skin only this app-owned native child. Native hit testing, capture, keyboard,
/// scroll notifications and accessibility remain in the standard procedure.
unsafe extern "system" fn scrollbar_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            paint_scrollbar(hwnd, dc);
            let _ = EndPaint(hwnd, &paint);
            return LRESULT(0);
        }
        WM_PRINTCLIENT => {
            paint_scrollbar(hwnd, HDC(wp.0 as *mut _));
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(scrollbar_proc), id);
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        _ => {}
    }
    let result = DefSubclassProc(hwnd, msg, wp, lp);
    if msg == WM_SETFOCUS {
        // The classic control creates a gray bitmap caret over its thumb.
        // Our lavender thumb outline supplies the focus indication instead.
        // Hide once per focus acquisition; native tracking's balanced
        // HideCaret/ShowCaret pairs then keep this caret hidden.
        let _ = HideCaret(hwnd);
    }
    if msg == WM_MOUSEMOVE {
        let mut tracking = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut tracking);
    }
    if matches!(
        msg,
        WM_MOUSEMOVE
            | WM_MOUSELEAVE
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_CAPTURECHANGED
            | WM_SETFOCUS
            | WM_KILLFOCUS
            | WM_ENABLE
            | WM_SIZE
            | WM_THEMECHANGED
            | WM_KEYDOWN
            | WM_KEYUP
            | SBM_SETSCROLLINFO
            | SBM_SETPOS
            | SBM_SETRANGE
            | SBM_SETRANGEREDRAW
    ) || (msg == WM_TIMER && GetCapture() == hwnd)
    {
        repaint_scrollbar(hwnd);
    }
    result
}

impl App {
    pub(crate) unsafe fn sync_manager_scrollbar(&mut self) {
        if !IsWindow(self.manager).as_bool() {
            return;
        }
        if self.settings {
            self.manager_scroll = 0;
            self.manager_wheel_delta = 0;
            let _ = ShowWindow(self.manager_scrollbar, SW_HIDE);
            return;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(self.manager, &mut client);
        let scale = GetDpiForWindow(self.manager).max(96) as f32 / 96.;
        let overview = Overview::new(
            client.right as f32 / scale,
            client.bottom as f32 / scale,
            self.config.groups.len(),
        );
        let clamped = overview.clamp(self.manager_scroll);
        if clamped != self.manager_scroll {
            self.manager_scroll = clamped;
            self.hover.remove(&(self.manager.0 as isize));
            self.hits.remove(&(self.manager.0 as isize));
        }
        if overview.max_scroll == 0 {
            self.manager_wheel_delta = 0;
            let _ = ShowWindow(self.manager_scrollbar, SW_HIDE);
            return;
        }
        if !IsWindow(self.manager_scrollbar).as_bool() {
            self.manager_scrollbar = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("SCROLLBAR"),
                None,
                WS_CHILD | WS_TABSTOP | WINDOW_STYLE(SBS_VERT as u32),
                0,
                0,
                1,
                1,
                self.manager,
                HMENU(ID_MANAGER_SCROLL as *mut _),
                GetModuleHandleW(None).unwrap_or_default(),
                None,
            )
            .unwrap_or_default();
            // Disable themed drawing only for this native child; its normal
            // procedure still owns all interaction and accessibility state.
            let _ = SetWindowTheme(self.manager_scrollbar, w!(""), w!(""));
            let _ = SetWindowSubclass(
                self.manager_scrollbar,
                Some(scrollbar_proc),
                ID_MANAGER_SCROLL,
                0,
            );
        }
        let r = overview.scrollbar;
        let _ = SetWindowPos(
            self.manager_scrollbar,
            HWND_TOP,
            (r.left * scale).round() as i32,
            (r.top * scale).round() as i32,
            ((r.right - r.left) * scale).round() as i32,
            ((r.bottom - r.top) * scale).round().max(1.) as i32,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: overview.content_height.saturating_sub(1),
            nPage: (overview.viewport.bottom - overview.viewport.top)
                .floor()
                .max(1.) as u32,
            nPos: self.manager_scroll,
            ..Default::default()
        };
        SetScrollInfo(self.manager_scrollbar, SB_CTL, &info, false);
        repaint_scrollbar(self.manager_scrollbar);
    }

    unsafe fn manager_overview(&self) -> Overview {
        let mut client = RECT::default();
        let _ = GetClientRect(self.manager, &mut client);
        let scale = GetDpiForWindow(self.manager).max(96) as f32 / 96.;
        Overview::new(
            client.right as f32 / scale,
            client.bottom as f32 / scale,
            self.config.groups.len(),
        )
    }

    pub(crate) unsafe fn scroll_manager_to(&mut self, offset: i32) {
        if self.settings || self.quitting {
            return;
        }
        let next = self.manager_overview().clamp(offset);
        if next == self.manager_scroll {
            return;
        }
        self.manager_scroll = next;
        self.hover.remove(&(self.manager.0 as isize));
        self.hits.remove(&(self.manager.0 as isize));
        self.sync_manager_scrollbar();
        invalidate(self.manager);
    }

    pub(crate) unsafe fn scroll_manager(&mut self, command: u16) {
        if self.settings {
            return;
        }
        let overview = self.manager_overview();
        let page = (overview.viewport.bottom - overview.viewport.top)
            .round()
            .max(1.) as i32;
        let next = match command {
            value if value == SB_LINEUP.0 as u16 => self.manager_scroll.saturating_sub(LINE),
            value if value == SB_LINEDOWN.0 as u16 => self.manager_scroll.saturating_add(LINE),
            value if value == SB_PAGEUP.0 as u16 => self.manager_scroll.saturating_sub(page),
            value if value == SB_PAGEDOWN.0 as u16 => self.manager_scroll.saturating_add(page),
            value if value == SB_TOP.0 as u16 => 0,
            value if value == SB_BOTTOM.0 as u16 => overview.max_scroll,
            value if value == SB_THUMBTRACK.0 as u16 || value == SB_THUMBPOSITION.0 as u16 => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_TRACKPOS,
                    ..Default::default()
                };
                if GetScrollInfo(self.manager_scrollbar, SB_CTL, &mut info).is_err() {
                    return;
                }
                info.nTrackPos
            }
            _ => return,
        };
        self.scroll_manager_to(next);
    }

    pub(crate) unsafe fn manager_wheel(&mut self, delta: i32) {
        if self.settings {
            return;
        }
        self.manager_wheel_delta = self.manager_wheel_delta.saturating_add(delta);
        let steps = self.manager_wheel_delta / WHEEL_DELTA as i32;
        self.manager_wheel_delta %= WHEEL_DELTA as i32;
        if steps == 0 {
            return;
        }
        let mut lines = 3u32;
        let _ = SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some((&mut lines as *mut u32).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let amount = if lines == u32::MAX {
            let overview = self.manager_overview();
            (overview.viewport.bottom - overview.viewport.top).round() as i32
        } else {
            LINE.saturating_mul(lines.min(100) as i32)
        };
        self.scroll_manager_to(
            self.manager_scroll
                .saturating_sub(steps.saturating_mul(amount)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_groups_fit_and_seventh_enables_scroll_without_new_pages() {
        let six = Overview::new(1036., 663., 6);
        assert_eq!(six.columns, 3);
        assert_eq!(six.max_scroll, 0);
        let seven = Overview::new(1036., 663., 7);
        assert!(seven.max_scroll > 0);
        assert_eq!(seven.visible(seven.max_scroll, 7).end, 7);
        let last = seven.card(6, seven.max_scroll);
        assert!((last.bottom - seven.viewport.bottom).abs() < 0.01);
    }

    #[test]
    fn clipping_prevents_scrolled_card_and_menu_hits_in_header_or_footer() {
        let view = Overview::new(1036., 663., 18);
        let first = view.card(0, 100);
        assert!(first.top < view.viewport.top);
        let hit = clipped(first, view.viewport).unwrap();
        assert_eq!(hit.top, view.viewport.top);
        let menu = render::rect(first.right - 41., first.top + 12., 29., 29.);
        assert!(clipped(menu, view.viewport).is_none());
        let last = view.card(17, view.max_scroll);
        assert!(clipped(last, view.viewport).unwrap().bottom <= view.viewport.bottom);
    }

    #[test]
    fn offsets_clamp_after_group_removal_or_resize_and_preserve_indices() {
        let many = Overview::new(1036., 663., 24);
        let visible = many.visible(many.max_scroll, 24);
        assert!(visible.start > 0);
        assert_eq!(visible.end, 24);
        let first = many.card(visible.start, many.max_scroll);
        assert!(clipped(first, many.viewport).is_some());
        let fewer = Overview::new(1036., 663., 3);
        assert_eq!(fewer.clamp(many.max_scroll), 0);
        assert_eq!(many.clamp(-100), 0);
        assert_eq!(many.clamp(i32::MAX), many.max_scroll);
        assert!(Overview::new(760., 663., 24).columns < many.columns);
    }
}
