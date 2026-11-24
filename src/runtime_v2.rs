//! Interaction-scoped timers and native controls for the compact desktop UI.
//! No timer runs while all folders are closed and the pointer is away.
use crate::*;
use std::time::Instant;

const DWELL_TIMER: usize = 8;
const LEAVE_TIMER: usize = 9;
const EXPAND_TIMER: usize = 10;
const MENU_WARM_TIMER: usize = 11;
const TITLE_ID: usize = 605;
const EXPAND_MS: f32 = 140.;

pub(crate) struct ExpandAnimation {
    pub from: RECT,
    pub to: RECT,
    pub started: Instant,
    pub by_hover: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum TitleTarget {
    Group(usize),
    Item(usize, String),
}

fn mix_rect(from: RECT, to: RECT, amount: f32) -> RECT {
    let mix = |a: i32, b: i32| (a as f32 + (b - a) as f32 * amount).round() as i32;
    RECT {
        left: mix(from.left, to.left),
        top: mix(from.top, to.top),
        right: mix(from.right, to.right),
        bottom: mix(from.bottom, to.bottom),
    }
}

fn inside(point: POINT, rect: RECT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn snap_axis(value: f32, size: f32, cell: f32, origin: f32, maximum: f32) -> i32 {
    let cell = if cell.is_finite() && cell >= 24. {
        cell
    } else {
        75.
    };
    let origin = if origin.is_finite() { origin } else { 0. };
    let span = (size / cell).ceil().max(1.) * cell;
    let inset = (span - size) / 2.;
    let slot = ((value - origin - inset) / span).round();
    (origin + slot * span + inset)
        .clamp(8., maximum.max(8.))
        .round() as i32
}

impl App {
    pub(crate) fn v2_popup_layout(&self) -> layout::PopupLayout {
        // Keep the base folder's size while filtering. Descendant search can
        // expand it, but typing a query never collapses the grid under a hand.
        layout::popup_layout(self.current_items().len().max(self.visible_items().len()))
    }

    unsafe fn v2_popup_destination(&self, l: &layout::PopupLayout) -> RECT {
        let work = platform::primary_work_area();
        let width = (l.width * self.scale).round() as i32;
        let height = (l.height * self.scale).round() as i32;
        let mut tile = RECT::default();
        let hwnd = self.tiles.get(self.group).copied().unwrap_or_default();
        if GetWindowRect(hwnd, &mut tile).is_err() {
            let x = work.left + ((work.right - work.left) - width) / 2;
            let y = work.top + ((work.bottom - work.top) - height) / 2;
            tile = RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            };
        }
        let x = ((tile.left + tile.right - width) / 2)
            .clamp(work.left + 8, (work.right - width - 8).max(work.left + 8));
        let y = ((tile.top + tile.bottom - height) / 2)
            .clamp(work.top + 8, (work.bottom - height - 8).max(work.top + 8));
        RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        }
    }

    unsafe fn v2_position_search(&self, l: &layout::PopupLayout) {
        if self.search.0.is_null() {
            return;
        }
        let _ = SetWindowPos(
            self.search,
            HWND_TOP,
            (l.search_x * self.scale).round() as i32,
            (l.search_y * self.scale).round() as i32,
            (l.search_w * self.scale).round() as i32,
            (l.search_h * self.scale).round() as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }

    pub(crate) unsafe fn v2_open_group(&mut self, index: usize, by_hover: bool) {
        if index >= self.config.groups.len() {
            return;
        }
        self.v2_finish_title(true);
        self.v2_close_popup();
        self.group = index;
        self.trail.clear();
        self.query.clear();
        self.page = 0;
        self.focused = 0;
        self.popup_pinned = false;
        self.hover_opened = by_hover;
        let l = self.v2_popup_layout();
        if self.popup.0.is_null() || !IsWindow(self.popup).as_bool() {
            self.popup = make_window(
                POPUP,
                "NestDesktop — Open folder",
                l.width as i32,
                l.height as i32,
                self.controller,
                WS_POPUP | WS_BORDER,
                WS_EX_TOOLWINDOW,
                self.scale,
            );
            rounded(
                self.popup,
                (l.width * self.scale) as i32,
                (l.height * self.scale) as i32,
                16,
            );
            let _ = surface::apply_glass(self.popup);
            self.search = edit(
                self.popup,
                ID_SEARCH,
                l.search_x as i32,
                l.search_y as i32,
                l.search_w as i32,
                l.search_h as i32,
                self.scale,
                self.font,
                "Search this folder",
            );
            let _ = surface::apply_opaque_control(self.search);
            SendMessageW(
                self.search,
                EM_SETMARGINS,
                WPARAM(EC_LEFTMARGIN as usize | EC_RIGHTMARGIN as usize),
                LPARAM(0),
            );
            DragAcceptFiles(self.popup, true);
        }
        let owner = if IsWindowVisible(self.manager).as_bool() {
            self.manager
        } else {
            self.controller
        };
        SetWindowLongPtrW(self.popup, GWLP_HWNDPARENT, owner.0 as isize);
        let _ = SetWindowTextW(self.search, w!(""));
        self.v2_position_search(&l);
        let _ = ShowWindow(self.search, SW_HIDE);
        let destination = self.v2_popup_destination(&l);
        let mut source = RECT::default();
        let tile = self.tiles.get(index).copied().unwrap_or_default();
        if GetWindowRect(tile, &mut source).is_err() {
            source = destination;
        }
        let _ = ShowWindow(tile, SW_HIDE);
        self.open_animation_progress = 0.;
        self.animation = Some(ExpandAnimation {
            from: source,
            to: destination,
            started: Instant::now(),
            by_hover,
        });
        let flags = SWP_SHOWWINDOW
            | if by_hover {
                SWP_NOACTIVATE
            } else {
                SWP_NOOWNERZORDER
            };
        let _ = SetWindowPos(
            self.popup,
            HWND_TOP,
            source.left,
            source.top,
            (source.right - source.left).max(1),
            (source.bottom - source.top).max(1),
            flags,
        );
        if !by_hover {
            let _ = SetForegroundWindow(self.popup);
            let _ = SetFocus(self.popup);
        }
        SetTimer(self.controller, EXPAND_TIMER, 16, None);
        if by_hover {
            SetTimer(self.controller, LEAVE_TIMER, 25, None);
        }
        invalidate(self.popup);
    }

    pub(crate) unsafe fn v2_close_popup(&mut self) {
        self.v4_close_colors(false);
        for id in [DWELL_TIMER, LEAVE_TIMER, EXPAND_TIMER, MENU_WARM_TIMER] {
            let _ = KillTimer(self.controller, id);
        }
        self.v2_finish_title(true);
        if self.item_drag.take().is_some() {
            let _ = ReleaseCapture();
        }
        self.animation = None;
        self.open_animation_progress = 1.;
        self.popup_pinned = false;
        self.hover_opened = false;
        self.hover_started = None;
        self.hover_progress = 0.;
        self.hovered_group = None;
        self.menu_warm_path = None;
        self.reorder_before = None;
        self.reorder_at_end = false;
        if !self.popup.0.is_null() {
            let _ = ShowWindow(self.popup, SW_HIDE);
        }
        if let Some(tile) = self.tiles.get(self.group).copied() {
            self.show_ready_tile(tile);
            invalidate(tile);
        }
    }

    pub(crate) unsafe fn v2_hover_tile(&mut self, index: usize, over_preview: bool) {
        if !over_preview
            || !self.config.settings.hover_open
            || self.title_target.is_some()
            || self.drag.is_some()
            || self.item_drag.is_some()
            || index >= self.tiles.len()
        {
            if self.hovered_group == Some(index) && !self.hover_opened {
                self.v2_leave_tile(index);
            }
            return;
        }
        if IsWindowVisible(self.popup).as_bool() {
            return;
        }
        if self.hovered_group == Some(index) && self.hover_started.is_some() {
            return;
        }
        if let Some(previous) = self.hovered_group {
            if let Some(h) = self.tiles.get(previous) {
                invalidate(*h)
            }
        }
        self.hovered_group = Some(index);
        self.hover_progress = 0.;
        self.hover_started = Some(Instant::now());
        SetTimer(self.controller, DWELL_TIMER, 16, None);
        invalidate(self.tiles[index]);
    }

    pub(crate) unsafe fn v2_leave_tile(&mut self, index: usize) {
        if self.hovered_group != Some(index) {
            return;
        }
        if !self.hover_opened {
            let _ = KillTimer(self.controller, DWELL_TIMER);
            self.hover_started = None;
            self.hovered_group = None;
            self.hover_progress = 0.;
            if let Some(h) = self.tiles.get(index) {
                invalidate(*h)
            }
        }
    }

    pub(crate) unsafe fn v2_timer(&mut self, id: usize) -> bool {
        match id {
            DWELL_TIMER => {
                let Some(index) = self.hovered_group else {
                    let _ = KillTimer(self.controller, id);
                    return true;
                };
                let Some(started) = self.hover_started else {
                    let _ = KillTimer(self.controller, id);
                    return true;
                };
                if !self.config.settings.hover_open
                    || self.title_target.is_some()
                    || self.drag.is_some()
                {
                    self.v2_leave_tile(index);
                    return true;
                }
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let mut tile = RECT::default();
                let hwnd = self.tiles.get(index).copied().unwrap_or_default();
                let under_pointer = WindowFromPoint(point);
                let mut local = point;
                let _ = ScreenToClient(hwnd, &mut local);
                let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
                if GetWindowRect(hwnd, &mut tile).is_err()
                    || !inside(point, tile)
                    || !layout::tile_preview_contains(
                        local.x as f32 / scale,
                        local.y as f32 / scale,
                    )
                    || (under_pointer != hwnd && !IsChild(hwnd, under_pointer).as_bool())
                {
                    self.v2_leave_tile(index);
                    return true;
                }
                self.hover_progress = (started.elapsed().as_secs_f32() * 1000.
                    / self.config.settings.hover_delay_ms.max(1) as f32)
                    .min(1.);
                invalidate(hwnd);
                if self.hover_progress >= 1. {
                    let _ = KillTimer(self.controller, id);
                    self.v2_open_group(index, true);
                }
                true
            }
            LEAVE_TIMER => {
                if self.popup_pinned || !self.hover_opened || !IsWindowVisible(self.popup).as_bool()
                {
                    let _ = KillTimer(self.controller, id);
                    return true;
                }
                if self.title_target.is_some()
                    || self.item_drag.is_some()
                    || self.prompt_state.is_some()
                    || self.color_session.is_some()
                {
                    return true;
                }
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let mut keep = false;
                for window in [self.popup, self.picker, self.prompt, self.color_window] {
                    let mut r = RECT::default();
                    if IsWindowVisible(window).as_bool()
                        && GetWindowRect(window, &mut r).is_ok()
                        && inside(point, r)
                    {
                        keep = true;
                        break;
                    }
                }
                if !keep {
                    self.v2_close_popup();
                }
                true
            }
            EXPAND_TIMER => {
                let Some(animation) = self.animation.as_ref() else {
                    let _ = KillTimer(self.controller, id);
                    return true;
                };
                let t =
                    (animation.started.elapsed().as_secs_f32() * 1000. / EXPAND_MS).clamp(0., 1.);
                let ease = 1. - (1. - t).powi(3);
                let frame = mix_rect(animation.from, animation.to, ease);
                let by_hover = animation.by_hover;
                let _ = SetWindowPos(
                    self.popup,
                    HWND_TOP,
                    frame.left,
                    frame.top,
                    (frame.right - frame.left).max(1),
                    (frame.bottom - frame.top).max(1),
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
                self.open_animation_progress = ease;
                invalidate(self.popup);
                if t >= 1. {
                    self.animation = None;
                    self.open_animation_progress = 1.;
                    let _ = KillTimer(self.controller, id);
                    let l = self.v2_popup_layout();
                    self.v2_position_search(&l);
                    let _ = ShowWindow(self.search, SW_SHOWNOACTIVATE);
                    if !by_hover {
                        let _ = SetFocus(self.popup);
                    }
                    invalidate(self.popup);
                }
                true
            }
            MENU_WARM_TIMER => {
                let _ = KillTimer(self.controller, id);
                if let Some(path) = self.menu_warm_path.take() {
                    if IsWindowVisible(self.popup).as_bool() {
                        let _ = shell_menu::warm_path(self.popup, &path);
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) unsafe fn v2_relayout_popup(&mut self, animate: bool) {
        if !IsWindowVisible(self.popup).as_bool() {
            return;
        }
        if self.group >= self.config.groups.len() {
            self.v2_close_popup();
            return;
        }
        let l = self.v2_popup_layout();
        let destination = self.v2_popup_destination(&l);
        self.v2_position_search(&l);
        if let Some(animation) = self.animation.as_mut() {
            animation.to = destination;
            return;
        }
        if animate {
            let mut from = RECT::default();
            let _ = GetWindowRect(self.popup, &mut from);
            self.animation = Some(ExpandAnimation {
                from,
                to: destination,
                started: Instant::now(),
                by_hover: self.hover_opened,
            });
            self.open_animation_progress = 0.;
            let _ = ShowWindow(self.search, SW_HIDE);
            SetTimer(self.controller, EXPAND_TIMER, 16, None);
        } else {
            let _ = SetWindowPos(
                self.popup,
                HWND_TOP,
                destination.left,
                destination.top,
                destination.right - destination.left,
                destination.bottom - destination.top,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        if let Some(tile) = self.tiles.get(self.group) {
            let _ = ShowWindow(*tile, SW_HIDE);
        }
        invalidate(self.popup);
    }

    /// Settle before positioning a floating inline item-name editor. Creating a
    /// folder from a closed tile's menu must not leave its editor in midair.
    pub(crate) unsafe fn v2_settle_popup(&mut self) {
        let _ = KillTimer(self.controller, EXPAND_TIMER);
        self.animation = None;
        self.open_animation_progress = 1.;
        self.v2_relayout_popup(false);
        let _ = ShowWindow(self.search, SW_SHOWNOACTIVATE);
        invalidate(self.popup);
    }

    pub(crate) unsafe fn v2_start_title(
        &mut self,
        target: TitleTarget,
        parent: HWND,
        r: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
        name: String,
        popup: bool,
    ) {
        self.v2_finish_title(true);
        self.title_generation = self.title_generation.wrapping_add(1);
        let dpi = GetDpiForWindow(parent).max(96) as f32 / 96.;
        let size = if popup { 22. } else { 13. };
        self.title_font = CreateFontW(
            (-size * dpi).round() as i32,
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe UI"),
        );
        let inset_y = if popup { 5. } else { 2. };
        let inset_x = if popup { 7. } else { 2. };
        // UpdateLayeredWindow tiles cannot reliably host a native EDIT. A
        // temporary ordinary HWND covers only the label while it is edited.
        let edit_parent = if popup {
            parent
        } else {
            if self.title_host.0.is_null() || !IsWindow(self.title_host).as_bool() {
                self.title_host = make_window(
                    TITLE_HOST,
                    "NestDesktop — Rename folder",
                    1,
                    1,
                    self.controller,
                    WS_POPUP,
                    WS_EX_TOOLWINDOW,
                    dpi,
                );
            }
            if !IsWindow(self.title_host).as_bool() {
                let _ = DeleteObject(self.title_font);
                self.title_font = HFONT::default();
                return;
            }
            let owner = if parent == self.popup {
                self.popup
            } else if IsWindowVisible(self.manager).as_bool() {
                self.manager
            } else {
                self.controller
            };
            SetWindowLongPtrW(self.title_host, GWLP_HWNDPARENT, owner.0 as isize);
            let mut point = POINT {
                x: (r.left * dpi).round() as i32,
                y: (r.top * dpi).round() as i32,
            };
            let _ = ClientToScreen(parent, &mut point);
            let _ = SetWindowPos(
                self.title_host,
                HWND_TOP,
                point.x,
                point.y,
                ((r.right - r.left) * dpi).round() as i32,
                ((r.bottom - r.top) * dpi).round() as i32,
                SWP_NOACTIVATE,
            );
            self.title_host
        };
        if self.title_edit.0.is_null() || !IsWindow(self.title_edit).as_bool() {
            self.title_edit = edit(edit_parent, TITLE_ID, 0, 0, 1, 1, dpi, self.title_font, "");
        } else {
            let _ = SetParent(self.title_edit, edit_parent);
        }
        let _ = surface::apply_opaque_control(self.title_edit);
        if !IsWindow(self.title_edit).as_bool() {
            let _ = ShowWindow(self.title_host, SW_HIDE);
            let _ = DeleteObject(self.title_font);
            self.title_font = HFONT::default();
            return;
        }
        let x = ((if popup { r.left } else { 0. } + inset_x) * dpi).round() as i32;
        let y = ((if popup { r.top } else { 0. } + inset_y) * dpi).round() as i32;
        let w = ((r.right - r.left - inset_x * 2.) * dpi).round() as i32;
        let h = ((r.bottom - r.top - inset_y * 2.) * dpi).round() as i32;
        let _ = SetWindowPos(self.title_edit, HWND_TOP, x, y, w, h, SWP_SHOWWINDOW);
        SendMessageW(
            self.title_edit,
            WM_SETFONT,
            WPARAM(self.title_font.0 as usize),
            LPARAM(1),
        );
        SendMessageW(self.title_edit, EM_SETLIMITTEXT, WPARAM(80), LPARAM(0));
        SendMessageW(
            self.title_edit,
            EM_SETMARGINS,
            WPARAM(EC_LEFTMARGIN as usize | EC_RIGHTMARGIN as usize),
            LPARAM(0),
        );
        let value = wide(&name);
        let _ = SetWindowTextW(self.title_edit, PCWSTR(value.as_ptr()));
        self.title_target = Some(target);
        if !popup {
            let _ = ShowWindow(self.title_host, SW_SHOWNORMAL);
            let _ = SetWindowPos(
                self.title_host,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            invalidate(self.title_host);
        }
        let _ = SetForegroundWindow(if popup { self.popup } else { self.title_host });
        let _ = SetFocus(self.title_edit);
        SendMessageW(self.title_edit, EM_SETSEL, WPARAM(0), LPARAM(-1));
        invalidate(parent);
    }

    pub(crate) unsafe fn v2_edit_group(&mut self, index: usize) {
        let Some(group) = self.config.groups.get(index) else {
            return;
        };
        let name = group.name.clone();
        let parent = self.tiles.get(index).copied().unwrap_or_default();
        if !IsWindow(parent).as_bool() {
            return;
        }
        let _ = KillTimer(self.controller, DWELL_TIMER);
        self.hover_started = None;
        self.hovered_group = None;
        self.hover_progress = 0.;
        self.v2_start_title(
            TitleTarget::Group(index),
            parent,
            layout::tile_title_rect(self.config.settings.show_item_counts),
            name,
            false,
        );
    }

    pub(crate) unsafe fn v2_edit_title(&mut self) {
        if !IsWindowVisible(self.popup).as_bool() {
            return;
        }
        let target = if let Some(id) = self.trail.last() {
            TitleTarget::Item(self.group, id.clone())
        } else {
            TitleTarget::Group(self.group)
        };
        self.v2_start_title(
            target,
            self.popup,
            self.v2_popup_layout().header_title,
            self.current_title(),
            true,
        );
    }

    pub(crate) unsafe fn v2_finish_title(&mut self, commit: bool) {
        let Some(target) = self.title_target.take() else {
            return;
        };
        let parent = GetParent(self.title_edit).unwrap_or_default();
        let had_focus = GetFocus() == self.title_edit;
        let desktop_edit = parent == self.title_host;
        let value = text(self.title_edit).trim().to_string();
        let _ = ShowWindow(self.title_edit, SW_HIDE);
        if desktop_edit {
            if had_focus {
                let _ = SetFocus(HWND::default());
            }
            let _ = ShowWindow(self.title_host, SW_HIDE);
        }
        SendMessageW(
            self.title_edit,
            WM_SETFONT,
            WPARAM(self.font.0 as usize),
            LPARAM(0),
        );
        if !self.title_font.0.is_null() {
            let _ = DeleteObject(self.title_font);
            self.title_font = HFONT::default();
        }
        if self.finish_creation_title(&target, commit, &value, had_focus) {
            return;
        }
        let previous = self.config.clone();
        let mut changed = false;
        if commit && !value.is_empty() {
            match target {
                TitleTarget::Group(index) => {
                    if let Some(group) = self.config.groups.get_mut(index) {
                        changed = group.name != value;
                        group.name = value;
                    }
                }
                TitleTarget::Item(group, id) => {
                    if let Some(item) = self
                        .config
                        .groups
                        .get_mut(group)
                        .and_then(|g| g.find_item_mut(&id))
                    {
                        changed = item.name != value;
                        item.name = value;
                    }
                }
            }
        }
        if changed {
            if self.config.validate().is_err() {
                self.config = previous
            } else {
                self.save();
            }
        }
        if !desktop_edit && IsWindow(parent).as_bool() {
            if had_focus && IsWindowVisible(parent).as_bool() {
                let _ = SetFocus(parent);
            }
            invalidate(parent);
        }
        for tile in &self.tiles {
            invalidate(*tile)
        }
        invalidate(self.popup);
        invalidate(self.manager);
    }

    pub(crate) unsafe fn v2_snap_group(&mut self, index: usize) {
        if !self.config.settings.snap_to_grid {
            return;
        }
        let Some(group) = self.config.groups.get(index) else {
            return;
        };
        let (x, y) = self.v2_snap_position(group.x, group.y);
        if let Some(group) = self.config.groups.get_mut(index) {
            group.x = x;
            group.y = y;
        }
    }

    /// Shared by the drag preview and the committed drop position.
    pub(crate) unsafe fn v2_snap_position(&self, x: i32, y: i32) -> (i32, i32) {
        let work = platform::primary_work_area();
        let width = layout::TILE_WIDTH;
        let height = layout::tile_height(self.config.settings.show_item_counts);
        let maxx = (work.right - work.left) as f32 / self.scale - width - 8.;
        let maxy = (work.bottom - work.top) as f32 / self.scale - height - 8.;
        if !self.config.settings.snap_to_grid {
            return (
                x.clamp(8, maxx.max(8.) as i32),
                y.clamp(8, maxy.max(8.) as i32),
            );
        }
        (
            snap_axis(x as f32, width, self.grid.cell_w, self.grid.origin_x, maxx),
            snap_axis(y as f32, height, self.grid.cell_h, self.grid.origin_y, maxy),
        )
    }

    pub(crate) unsafe fn v2_update_preferences(&mut self) {
        self.v2_finish_title(true);
        self.grid = desktop_grid::metrics((self.scale * 96.).round() as u32);
        for index in 0..self.config.groups.len() {
            self.v2_snap_group(index)
        }
        self.rebuild_tiles();
        self.v2_relayout_popup(false);
        self.save();
        self.refresh();
    }

    pub(crate) unsafe fn v2_custom_color(&mut self) {
        let Some(group) = self.config.groups.get(self.group) else {
            return;
        };
        let initial = group.color.clone();
        if let Some((color, palette)) = native_color::choose(
            self.dialog_owner(),
            &initial,
            &self.config.settings.custom_colors,
        ) {
            if let Some(group) = self.config.groups.get_mut(self.group) {
                group.color = color;
            }
            self.config.settings.custom_colors = palette;
            self.save();
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn span_snapping_keeps_large_tiles_on_grid_and_inside_work_area() {
        // 104px folder occupies two 75px cells and sits centrally in that slot.
        assert_eq!(snap_axis(48., 104., 75., 0., 900.), 23);
        assert_eq!(snap_axis(177., 104., 75., 0., 900.), 173);
        assert_eq!(snap_axis(-100., 104., 75., 0., 900.), 8);
        assert!(snap_axis(10000., 104., 75., 0., 900.) <= 900);
        assert!(snap_axis(45., 104., f32::NAN, 0., 900.) >= 8);
    }
    #[test]
    fn expansion_starts_at_preview_and_finishes_at_full_popup() {
        let from = RECT {
            left: 50,
            top: 60,
            right: 154,
            bottom: 174,
        };
        let to = RECT {
            left: 8,
            top: 8,
            right: 496,
            bottom: 248,
        };
        let first = mix_rect(from, to, 0.);
        let last = mix_rect(from, to, 1.);
        let middle = mix_rect(from, to, 0.5);
        assert_eq!(
            (first.left, first.top, first.right, first.bottom),
            (50, 60, 154, 174)
        );
        assert_eq!(
            (last.left, last.top, last.right, last.bottom),
            (8, 8, 496, 248)
        );
        assert!(middle.right - middle.left > first.right - first.left);
        assert!(middle.right - middle.left < last.right - last.left);
    }
}
