//! Native color popover with an in-memory draft and explicitly saved presets.
use crate::palette::{Color, Hsv};
use crate::palette_view::{Control, ViewState};
use crate::*;

pub(crate) struct ColorSession {
    pub group_id: String,
    pub view: ViewState,
}

impl App {
    pub(crate) fn v4_color_control_at(&self, x: f32, y: f32) -> Option<Control> {
        self.hits
            .get(&(self.color_window.0 as isize))?
            .iter()
            .find(|hit| x >= hit.r.left && x <= hit.r.right && y >= hit.r.top && y <= hit.r.bottom)
            .and_then(|hit| match hit.action {
                Action::Palette(control) => Some(control),
                _ => None,
            })
    }

    unsafe fn v4_refresh_colors(&mut self) {
        // Palette mutations can add/remove hit regions. Never reuse a hover or
        // clickable region from the preceding frame while a new one is pending.
        let key = self.color_window.0 as isize;
        self.hover.remove(&key);
        self.hits.remove(&key);
        invalidate(self.color_window);
    }

    unsafe fn v4_save_current_color(&mut self, replace: Option<usize>) {
        if !self.v4_read_color_hex() {
            return;
        }
        let session = self.color_session.as_ref().unwrap();
        if replace.is_some() && session.view.saved.len() != palette::MAX_CUSTOM_COLORS {
            return;
        }
        let mut saved = session.view.saved.clone();
        match palette::save_custom(&mut saved, &session.view.selected_hex, replace) {
            Ok(selected) => {
                let previous = self.config.clone();
                self.config.settings.custom_colors = saved.clone();
                if let Err(error) = self.config.save(&self.path) {
                    self.config = previous;
                    self.color_session.as_mut().unwrap().view.error = error;
                } else {
                    let view = &mut self.color_session.as_mut().unwrap().view;
                    view.saved = saved;
                    view.selected_custom = Some(selected);
                    view.error.clear();
                }
            }
            Err(error) => self.color_session.as_mut().unwrap().view.error = error,
        }
        self.v4_refresh_colors();
    }

    pub(crate) unsafe fn v4_color_height(&self) -> f32 {
        let mut r = RECT::default();
        if GetClientRect(self.color_window, &mut r).is_ok() {
            r.bottom as f32 / (GetDpiForWindow(self.color_window).max(96) as f32 / 96.)
        } else {
            palette_view::HEIGHT
        }
    }

    pub(crate) unsafe fn v4_open_colors(&mut self) {
        self.v2_finish_title(true);
        self.v4_close_colors(false);
        let Some(group) = self.config.groups.get(self.group) else {
            return;
        };
        let color = Color::parse(&group.color).unwrap_or(Color::from_packed(0x9b6dff));
        let hsv = color.hsv();
        let saved = palette::canonical_custom_colors(&self.config.settings.custom_colors)
            .unwrap_or_default();
        let selected_custom = saved
            .iter()
            .position(|value| value.eq_ignore_ascii_case(&color.hex()));
        self.color_session = Some(ColorSession {
            group_id: group.id.clone(),
            view: ViewState {
                selected_hex: color.hex(),
                saved,
                selected_custom,
                hue: hsv.h / 360.,
                saturation: hsv.s,
                value: hsv.v,
                error: String::new(),
            },
        });
        let work = platform::primary_work_area();
        let height =
            palette_view::HEIGHT.min(((work.bottom - work.top - 16) as f32 / self.scale).max(488.));
        let owner = self.dialog_owner();
        if !IsWindow(self.color_window).as_bool() {
            self.color_window = make_window(
                COLOR_PANEL,
                "NestDesktop — Folder color",
                palette_view::WIDTH as i32,
                height as i32,
                owner,
                WS_POPUP,
                WS_EX_TOOLWINDOW,
                self.scale,
            );
            rounded(self.color_window, 0, 0, 12);
            self.color_hex_edit = edit(
                self.color_window,
                ID_COLOR_HEX,
                0,
                0,
                96,
                22,
                self.scale,
                self.font,
                "#RRGGBB",
            );
            SendMessageW(self.color_hex_edit, EM_SETLIMITTEXT, WPARAM(7), LPARAM(0));
        }
        SetWindowLongPtrW(self.color_window, GWLP_HWNDPARENT, owner.0 as isize);
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        if IsWindowVisible(self.popup).as_bool() {
            let b = layout::popup_folder_button_rect();
            point = POINT {
                x: (b.left * self.scale).round() as i32,
                y: ((b.bottom + 7.) * self.scale).round() as i32,
            };
            let _ = ClientToScreen(self.popup, &mut point);
        }
        let width = (palette_view::WIDTH * self.scale).round() as i32;
        let ph = (height * self.scale).round() as i32;
        point.x = point
            .x
            .clamp(work.left + 8, (work.right - width - 8).max(work.left + 8));
        point.y = point
            .y
            .clamp(work.top + 8, (work.bottom - ph - 8).max(work.top + 8));
        let _ = SetWindowPos(
            self.color_window,
            HWND_TOP,
            point.x,
            point.y,
            width,
            ph,
            SWP_NOACTIVATE,
        );
        let hex = palette_view::hex_rect(height);
        let _ = SetWindowPos(
            self.color_hex_edit,
            HWND_TOP,
            (hex.left * self.scale).round() as i32,
            (hex.top * self.scale).round() as i32,
            ((hex.right - hex.left) * self.scale).round() as i32,
            ((hex.bottom - hex.top) * self.scale).round() as i32,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        self.v4_sync_color_text();
        let _ = KillTimer(self.controller, 9);
        let _ = ShowWindow(self.color_window, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(self.color_window);
        let _ = SetFocus(self.color_window);
        self.v4_refresh_colors();
    }

    pub(crate) unsafe fn v4_close_colors(&mut self, focus_back: bool) {
        self.color_pressed = None;
        let was_open = self.color_session.take().is_some();
        if self.color_drag.take().is_some() || GetCapture() == self.color_window {
            let _ = ReleaseCapture();
        }
        let _ = ShowWindow(self.color_window, SW_HIDE);
        if was_open
            && self.hover_opened
            && !self.popup_pinned
            && IsWindowVisible(self.popup).as_bool()
        {
            SetTimer(self.controller, 9, 25, None);
        }
        if was_open && focus_back {
            let owner = self.dialog_owner();
            if IsWindowVisible(owner).as_bool() {
                let _ = SetForegroundWindow(owner);
                let _ = SetFocus(owner);
            }
        }
    }

    unsafe fn v4_sync_color_text(&mut self) {
        let Some(session) = self.color_session.as_ref() else {
            return;
        };
        let value = wide(&session.view.selected_hex);
        self.color_sync = true;
        let _ = SetWindowTextW(self.color_hex_edit, PCWSTR(value.as_ptr()));
        self.color_sync = false;
    }

    pub(crate) unsafe fn v4_color_text_changed(&mut self) {
        if self.color_sync {
            return;
        }
        if let Ok(color) = Color::parse(&text(self.color_hex_edit)) {
            if let Some(session) = self.color_session.as_mut() {
                let hsv = color.hsv();
                session.view.selected_hex = color.hex();
                session.view.hue = hsv.h / 360.;
                session.view.saturation = hsv.s;
                session.view.value = hsv.v;
                session.view.selected_custom = session
                    .view
                    .saved
                    .iter()
                    .position(|value| value.eq_ignore_ascii_case(&session.view.selected_hex));
                session.view.error.clear();
            }
            self.v4_refresh_colors();
        }
    }

    unsafe fn v4_read_color_hex(&mut self) -> bool {
        match Color::parse(&text(self.color_hex_edit)) {
            Ok(color) => {
                if let Some(session) = self.color_session.as_mut() {
                    session.view.selected_hex = color.hex();
                    session.view.error.clear();
                }
                true
            }
            Err(error) => {
                if let Some(session) = self.color_session.as_mut() {
                    session.view.error = error;
                }
                self.v4_refresh_colors();
                let _ = SetFocus(self.color_hex_edit);
                false
            }
        }
    }

    pub(crate) unsafe fn v4_drag_color(&mut self, x: f32, y: f32) {
        let height = self.v4_color_height();
        let Some(session) = self.color_session.as_mut() else {
            return;
        };
        let hsv = Hsv::new(
            session.view.hue * 360.,
            session.view.saturation,
            session.view.value,
        );
        let next = match self.color_drag {
            Some(Control::SaturationValue) => {
                let r = palette_view::sv_rect(height);
                hsv.from_sv_point(x - r.left, y - r.top, r.right - r.left, r.bottom - r.top)
            }
            Some(Control::Hue) => {
                let r = palette_view::hue_rect(height);
                hsv.with_hue(palette::hue_from_point(x - r.left, r.right - r.left))
            }
            _ => return,
        };
        session.view.hue = next.h / 360.;
        session.view.saturation = next.s;
        session.view.value = next.v;
        session.view.selected_hex = next.color().hex();
        session.view.selected_custom = session
            .view
            .saved
            .iter()
            .position(|value| value.eq_ignore_ascii_case(&session.view.selected_hex));
        session.view.error.clear();
        self.v4_sync_color_text();
        self.v4_refresh_colors();
    }

    pub(crate) unsafe fn v4_color_action(&mut self, control: Control) {
        if self.color_session.is_none() {
            return;
        }
        match control {
            Control::Close => self.v4_close_colors(true),
            Control::Preset(i) | Control::Custom(i) => {
                let session = self.color_session.as_mut().unwrap();
                let hex = if matches!(control, Control::Preset(_)) {
                    palette::COLOR_PRESETS.get(i).map(|p| p.1.to_string())
                } else {
                    session.view.saved.get(i).cloned()
                };
                if let Some(hex) = hex {
                    let color = Color::parse(&hex).unwrap();
                    let hsv = color.hsv();
                    session.view.selected_hex = hex;
                    session.view.hue = hsv.h / 360.;
                    session.view.saturation = hsv.s;
                    session.view.value = hsv.v;
                    session.view.selected_custom = if matches!(control, Control::Custom(_)) {
                        Some(i)
                    } else {
                        None
                    };
                    session.view.error.clear();
                    self.v4_sync_color_text();
                    self.v4_refresh_colors();
                }
            }
            Control::AddPreset => self.v4_save_current_color(None),
            Control::ReplacePreset(index) => self.v4_save_current_color(Some(index)),
            Control::ResetColor => {
                let color = Color::parse(model::DEFAULT_GROUP_COLOR).unwrap();
                let hsv = color.hsv();
                let view = &mut self.color_session.as_mut().unwrap().view;
                view.selected_hex = color.hex();
                view.hue = hsv.h / 360.;
                view.saturation = hsv.s;
                view.value = hsv.v;
                view.selected_custom = view
                    .saved
                    .iter()
                    .position(|value| value.eq_ignore_ascii_case(&view.selected_hex));
                view.error.clear();
                self.v4_sync_color_text();
                self.v4_refresh_colors();
            }
            Control::RemovePreset => {
                let session = self.color_session.as_mut().unwrap();
                let Some(index) = session.view.selected_custom else {
                    return;
                };
                let mut saved = session.view.saved.clone();
                match palette::remove_custom(&mut saved, index) {
                    Ok(()) => {
                        let previous = self.config.clone();
                        self.config.settings.custom_colors = saved.clone();
                        if let Err(error) = self.config.save(&self.path) {
                            self.config = previous;
                            self.color_session.as_mut().unwrap().view.error = error;
                        } else {
                            let view = &mut self.color_session.as_mut().unwrap().view;
                            view.saved = saved;
                            view.selected_custom = None;
                            view.error.clear();
                        }
                    }
                    Err(error) => self.color_session.as_mut().unwrap().view.error = error,
                }
                self.v4_refresh_colors();
            }
            Control::Apply => {
                if !self.v4_read_color_hex() {
                    return;
                }
                let session = self.color_session.as_ref().unwrap();
                let id = session.group_id.clone();
                let hex = session.view.selected_hex.clone();
                let previous = self.config.clone();
                if let Some(group) = self.config.find_group_mut(&id) {
                    group.color = hex;
                } else {
                    self.v4_close_colors(false);
                    return;
                }
                if let Err(error) = self.config.save(&self.path) {
                    self.config = previous;
                    self.color_session.as_mut().unwrap().view.error = error;
                    self.v4_refresh_colors();
                } else {
                    self.v4_close_colors(true);
                    self.refresh();
                }
            }
            Control::Hue | Control::SaturationValue => {}
        }
    }
}
