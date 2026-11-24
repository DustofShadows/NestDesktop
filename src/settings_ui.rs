//! Small native settings controls and the direct folder color menu.
use crate::*;

impl App {
    pub(crate) unsafe fn v3_sync_hover_delay_field(&mut self) {
        if !self.settings || !IsWindow(self.manager).as_bool() {
            let _ = ShowWindow(self.hover_delay_edit, SW_HIDE);
            return;
        }
        if !IsWindow(self.hover_delay_edit).as_bool() {
            self.hover_delay_edit = edit(
                self.manager,
                ID_HOVER_DELAY,
                0,
                0,
                60,
                22,
                self.scale,
                self.font,
                "Seconds",
            );
            SendMessageW(self.hover_delay_edit, EM_SETLIMITTEXT, WPARAM(8), LPARAM(0));
        }
        let mut client = RECT::default();
        let _ = GetClientRect(self.manager, &mut client);
        let _ = SetWindowPos(
            self.hover_delay_edit,
            HWND_TOP,
            client.right - (230. * self.scale).round() as i32,
            (342. * self.scale).round() as i32,
            (60. * self.scale).round() as i32,
            (22. * self.scale).round() as i32,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        if GetFocus() != self.hover_delay_edit {
            self.v3_reset_hover_delay_text();
        }
    }

    unsafe fn v3_reset_hover_delay_text(&self) {
        let mut value = format!("{:.3}", self.config.settings.hover_delay_ms as f64 / 1000.);
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.push('0');
        }
        let value = wide(&value);
        let _ = SetWindowTextW(self.hover_delay_edit, PCWSTR(value.as_ptr()));
    }

    pub(crate) unsafe fn v3_finish_hover_delay(
        &mut self,
        commit: bool,
        keep_invalid_focus: bool,
    ) -> bool {
        if !IsWindow(self.hover_delay_edit).as_bool() {
            return true;
        }
        if !commit {
            self.status.clear();
        }
        if commit {
            match model::parse_hover_delay_seconds(&text(self.hover_delay_edit)) {
                Ok(delay) => {
                    self.status.clear();
                    if self.config.settings.hover_delay_ms != delay {
                        self.config.settings.hover_delay_ms = delay;
                        self.save();
                    }
                }
                Err(error) => {
                    self.status = error;
                    invalidate(self.manager);
                    if keep_invalid_focus {
                        return false;
                    }
                }
            }
        }
        self.v3_reset_hover_delay_text();
        invalidate(self.manager);
        true
    }

    pub(crate) unsafe fn v3_choose_color_menu(&mut self) {
        self.v4_open_colors();
    }
}
