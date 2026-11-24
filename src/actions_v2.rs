//! UI command dispatch shared by the desktop tiles and expanded folders.

use crate::{
    folder_path, invalidate, message, model::Config, platform, transfer, Action, App, PromptKind,
};
use windows::{core::w, Win32::UI::WindowsAndMessaging::*};

impl App {
    pub(crate) unsafe fn dispatch_action(&mut self, action: Action) {
        if self.quitting {
            return;
        }
        match action {
            Action::RevealDesktop => self.reveal_desktop(),
            Action::CollectDesktop => self.collect_desktop(),
            Action::RestoreDesktop => self.restore_desktop(),
            Action::Open(index) => self.v2_open_group(index, false),
            Action::Menu(index) => {
                if self.group != index {
                    self.v2_close_popup();
                    self.trail.clear();
                }
                self.group = index;
                self.menu(None);
            }
            Action::NewGroup => self.begin_group_creation(),
            Action::AddApps => self.show_picker(),
            Action::Browse => {
                let paths = platform::choose_files(self.picker);
                self.add_paths(paths);
            }
            Action::NewSubfolder => self.begin_item_creation(),
            Action::Back => {
                self.trail.pop();
                self.query.clear();
                self.page = 0;
                self.focused = 0;
                let _ = SetWindowTextW(self.search, w!(""));
                self.refresh();
            }
            Action::ClosePicker => {
                let _ = ShowWindow(self.picker, SW_HIDE);
            }
            Action::PageBack => {
                self.page = self.page.saturating_sub(1);
                self.focused = 0;
                self.refresh();
            }
            Action::PageNext => {
                if (self.page + 1) * 12 < self.visible_items().len() {
                    self.page += 1;
                    self.focused = 0;
                    self.refresh();
                }
            }
            Action::Close => self.v2_close_popup(),
            Action::Manage => {
                self.v2_close_popup();
                self.v3_finish_hover_delay(true, false);
                self.settings = false;
                self.show_manager();
            }
            Action::Settings => {
                self.settings = true;
                self.manager_scroll = 0;
                self.manager_wheel_delta = 0;
                self.show_manager();
            }
            Action::Home => {
                self.v3_finish_hover_delay(true, false);
                self.settings = false;
                self.sync_manager_scrollbar();
                self.v3_sync_hover_delay_field();
                invalidate(self.manager);
            }
            Action::Quit => {
                self.begin_quit();
            }
            Action::Launch(id) => {
                let item = self
                    .config
                    .groups
                    .get(self.group)
                    .and_then(|group| group.find_item(&id))
                    .cloned();
                if let Some(item) = item {
                    if item.is_folder() {
                        let mut path = Vec::new();
                        if self
                            .config
                            .groups
                            .get(self.group)
                            .map(|group| folder_path(&group.items, &id, &mut path))
                            .unwrap_or(false)
                        {
                            self.trail = path;
                        }
                        self.page = 0;
                        self.focused = 0;
                        self.query.clear();
                        let _ = SetWindowTextW(self.search, w!(""));
                        self.refresh();
                    } else {
                        match platform::launch(&item.target, &item.arguments) {
                            Ok(()) if !self.popup_pinned => self.v2_close_popup(),
                            Ok(()) => {}
                            Err(error) => message(
                                self.popup,
                                &format!(
                                    "{} could not be opened.\n\n{error}\n\nIf this layout came from another PC, add this application again on this device.",
                                    item.name
                                ),
                            ),
                        }
                    }
                }
            }
            Action::PickAdd => self.add_selected(),
            Action::PromptOk => self.prompt_ok(),
            Action::PromptCancel => {
                let _ = ShowWindow(self.prompt, SW_HIDE);
                self.prompt_state = None;
            }
            Action::Startup => {
                let enabled = !platform::startup_enabled();
                if let Ok(executable) = std::env::current_exe() {
                    match platform::set_startup(&executable, enabled) {
                        Ok(()) => self.save(),
                        Err(error) => message(self.manager, &error),
                    }
                }
                invalidate(self.manager);
            }
            Action::DataFolder => {
                if let Some(path) = self.path.parent() {
                    let _ = platform::launch(&path.to_string_lossy(), "");
                }
            }
            Action::Uninstall => self.uninstall(),
            Action::Export => {
                if let Some(path) = platform::choose_layout_path(self.manager, true) {
                    match crate::creation::without_pending(&self.config, self.pending_creation.as_ref()).save(&path) {
                        Ok(()) => {
                            self.status =
                                "Layout exported. App locations may differ on another PC.".into()
                        }
                        Err(error) => message(self.manager, &error),
                    }
                    invalidate(self.manager);
                }
            }
            Action::Import => {
                if let Some(path) = platform::choose_layout_path(self.manager, false) {
                    match Config::load(&path) {
                        Ok(mut config) => {
                            if MessageBoxW(
                                self.manager,
                                w!("Replace your current folder layout? A backup of the current layout will be kept."),
                                w!("Import layout"),
                                MB_YESNO | MB_ICONQUESTION,
                            ) == IDYES
                            {
                                // Startup is a property of this device. Appearance and
                                // interaction preferences travel with the layout.
                                config.settings.start_with_windows = platform::startup_enabled();
                                let unresolved =
                                    transfer::relink_config(&mut config, &self.catalogue);
                                self.v2_close_popup();
                                if !self.commit_structure(config) { return; }
                                self.group = 0;
                                self.trail.clear();
                                self.rebuild_tiles();
                                self.status = if unresolved == 0 {
                                    "Layout imported.".into()
                                } else {
                                    format!("Layout imported. {unresolved} shortcuts need to be added again on this PC.")
                                };
                                invalidate(self.manager);
                            }
                        }
                        Err(error) => message(self.manager, &error),
                    }
                }
            }
            Action::SetColor(color) => {
                let previous = self.config.clone();
                if let Some(group) = self.config.groups.get_mut(self.group) {
                    group.color = color;
                }
                if self.accept_change(previous) {
                    self.save();
                    self.refresh();
                }
            }
            Action::RenameGroup => {
                let tile_visible = self
                    .tiles
                    .get(self.group)
                    .map(|tile| IsWindowVisible(*tile).as_bool())
                    .unwrap_or(false);
                if !tile_visible && IsWindowVisible(self.popup).as_bool() {
                    // The preview is hidden while expanded. Edit the corresponding
                    // desktop folder's header, rather than an invisible preview.
                    self.trail.clear();
                    self.query.clear();
                    self.page = 0;
                    self.focused = 0;
                    let _ = SetWindowTextW(self.search, w!(""));
                    self.refresh();
                    self.v2_edit_title();
                } else {
                    self.v2_edit_group(self.group);
                }
            }
            Action::RenameItem(id) => self.show_prompt(PromptKind::RenameItem(id)),
            Action::RemoveGroup => {
                if self.group < self.config.groups.len()
                    && MessageBoxW(
                        self.manager,
                        w!("Remove this NestDesktop folder? Your apps and original shortcuts stay on this PC."),
                        w!("Remove folder"),
                        MB_YESNO | MB_ICONQUESTION,
                    ) == IDYES
                {
                    self.v2_close_popup();
                    let mut candidate = self.config.clone();
                    candidate.groups.remove(self.group);
                    if !self.commit_structure(candidate) { return; }
                    self.group = 0;
                    self.trail.clear();
                    self.rebuild_tiles();
                    self.refresh();
                }
            }
            Action::RemoveItem(id) => {
                self.v2_finish_title(true);
                let mut candidate = self.config.clone();
                if let Some(group) = candidate.groups.get_mut(self.group) {
                    group.remove_item(&id);
                }
                if !self.commit_structure(candidate) { return; }
                self.refresh();
            }
            Action::MoveItem(id) => {
                self.context_item = Some(id);
                self.move_menu();
            }
            Action::ToggleCounts => {
                self.config.settings.show_item_counts = !self.config.settings.show_item_counts;
                self.v2_update_preferences();
            }
            Action::ToggleSnap => {
                self.config.settings.snap_to_grid = !self.config.settings.snap_to_grid;
                self.v2_update_preferences();
            }
            Action::ToggleHover => {
                self.config.settings.hover_open = !self.config.settings.hover_open;
                if !self.config.settings.hover_open && self.hover_opened {
                    self.v2_close_popup();
                }
                self.v2_update_preferences();
            }
            Action::CustomColor => self.v4_open_colors(),
            Action::ChooseColor => self.v3_choose_color_menu(),
            Action::Palette(control) => self.v4_color_action(control),
            Action::TogglePin => {
                self.popup_pinned = !self.popup_pinned;
                if self.popup_pinned {
                    let _ = KillTimer(self.controller, 9);
                } else if self.config.settings.hover_open {
                    self.hover_opened = true;
                    SetTimer(self.controller, 9, 25, None);
                }
                invalidate(self.popup);
            }
            Action::EditTitle => self.v2_edit_title(),
        }
    }
}
