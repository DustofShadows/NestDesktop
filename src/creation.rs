//! Folder creation stays a reversible draft until its inline name is accepted.
use crate::*;
use std::result::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    Group { id: String },
    Item { group_id: String, item_id: String },
}

impl Pending {
    fn remove(&self, config: &mut Config) {
        match self {
            Self::Group { id } => config.groups.retain(|group| &group.id != id),
            Self::Item { group_id, item_id } => {
                if let Some(group) = config.find_group_mut(group_id) {
                    group.remove_item(item_id);
                }
            }
        }
    }

    fn matches(&self, config: &Config, target: &runtime_v2::TitleTarget) -> bool {
        match (self, target) {
            (Self::Group { id }, runtime_v2::TitleTarget::Group(index)) => config
                .groups
                .get(*index)
                .is_some_and(|group| &group.id == id),
            (Self::Item { group_id, item_id }, runtime_v2::TitleTarget::Item(index, id)) => {
                item_id == id
                    && config
                        .groups
                        .get(*index)
                        .is_some_and(|group| &group.id == group_id)
            }
            _ => false,
        }
    }

    fn rename(&self, config: &mut Config, name: &str) -> bool {
        let value = match self {
            Self::Group { id } => config.find_group_mut(id).map(|group| &mut group.name),
            Self::Item { group_id, item_id } => config
                .find_group_mut(group_id)
                .and_then(|group| group.find_item_mut(item_id))
                .map(|item| &mut item.name),
        };
        if let Some(value) = value {
            *value = name.to_owned();
            true
        } else {
            false
        }
    }
}

/// Use for persistence/export while the draft is visible. Other preferences
/// can be saved without accidentally accepting a folder still being named.
pub(crate) fn without_pending(config: &Config, pending: Option<&Pending>) -> Config {
    let mut result = config.clone();
    if let Some(pending) = pending {
        pending.remove(&mut result);
    }
    result
}

fn default_name<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: std::collections::HashSet<_> = names.map(str::to_lowercase).collect();
    let base = "New folder";
    if !names.contains(&base.to_lowercase()) {
        return base.to_owned();
    }
    for index in 2.. {
        let candidate = format!("{base} ({index})");
        if !names.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!()
}

fn draft_group(config: &mut Config, position: (i32, i32)) -> Result<Pending, String> {
    let name = default_name(config.groups.iter().map(|group| group.name.as_str()));
    let group = Group {
        x: position.0,
        y: position.1,
        ..Group::new(name)
    };
    let pending = Pending::Group {
        id: group.id.clone(),
    };
    config.groups.push(group);
    if let Err(error) = config.validate() {
        pending.remove(config);
        return Err(error);
    }
    Ok(pending)
}

fn draft_item(
    config: &mut Config,
    group_id: &str,
    parent_id: Option<&str>,
) -> Result<Pending, String> {
    let group = config
        .find_group_mut(group_id)
        .ok_or_else(|| "The desktop folder no longer exists.".to_string())?;
    let items = if let Some(parent_id) = parent_id {
        let parent = group
            .find_item_mut(parent_id)
            .filter(|item| item.is_folder())
            .ok_or_else(|| "The destination folder no longer exists.".to_string())?;
        &mut parent.children
    } else {
        &mut group.items
    };
    let name = default_name(items.iter().map(|item| item.name.as_str()));
    let item = Item::folder(name);
    let pending = Pending::Item {
        group_id: group_id.to_owned(),
        item_id: item.id.clone(),
    };
    items.push(item);
    if let Err(error) = config.validate() {
        pending.remove(config);
        return Err(error);
    }
    Ok(pending)
}

impl App {
    pub(crate) unsafe fn begin_group_creation(&mut self) {
        self.v2_finish_title(true);
        self.v2_close_popup();
        self.reveal_desktop();
        let Some(position) = self.creation_position() else {
            self.status = "No free desktop space for another folder. Move a folder first.".into();
            message(self.controller, &self.status);
            return;
        };
        match draft_group(&mut self.config, position) {
            Ok(pending) => {
                let index = self.config.groups.len() - 1;
                // Rebuilding closes an old editor, so attach pending only once
                // that operation has completed. Nothing is saved here.
                self.rebuild_tiles();
                self.pending_creation = Some(pending);
                self.v2_edit_group(index);
                if self.title_target.is_none() {
                    self.cancel_creation();
                }
                invalidate(self.manager);
            }
            Err(error) => {
                self.status = error;
                message(self.controller, &self.status);
            }
        }
    }

    pub(crate) unsafe fn begin_item_creation(&mut self) {
        self.v2_finish_title(true);
        let Some(group) = self.config.groups.get(self.group) else {
            return;
        };
        let group_id = group.id.clone();
        if !IsWindowVisible(self.popup).as_bool() {
            self.v2_open_group(self.group, false);
        }
        if !IsWindow(self.popup).as_bool() {
            self.status = "The folder could not be opened.".into();
            return;
        }
        self.v2_settle_popup();
        let parent_id = self.trail.last().cloned();
        match draft_item(&mut self.config, &group_id, parent_id.as_deref()) {
            Ok(pending) => {
                let Pending::Item { item_id, .. } = &pending else {
                    unreachable!()
                };
                let id = item_id.clone();
                self.query.clear();
                let _ = SetWindowTextW(self.search, w!(""));
                let index = self.current_items().len() - 1;
                self.page = index / layout::PAGE_ITEMS;
                self.focused = index % layout::PAGE_ITEMS;
                self.pending_creation = Some(pending);
                self.refresh();
                let l = self.v2_popup_layout();
                let r = layout::item_rect(&l, self.focused);
                let name = self.config.find_item(&id).unwrap().name.clone();
                self.v2_start_title(
                    runtime_v2::TitleTarget::Item(self.group, id),
                    self.popup,
                    layout::bounds(r.left + 1., r.top + 44., l.cell_w - 5., 24.),
                    name,
                    false,
                );
                if self.title_target.is_none() {
                    self.cancel_creation();
                }
            }
            Err(error) => {
                self.status = error;
                message(self.popup, &self.status);
            }
        }
    }

    /// Called only after the native editor/font is safely hidden and released.
    /// Returns true when creation handled the operation instead of a rename.
    pub(crate) unsafe fn finish_creation_title(
        &mut self,
        target: &runtime_v2::TitleTarget,
        commit: bool,
        name: &str,
        had_focus: bool,
    ) -> bool {
        if !self
            .pending_creation
            .as_ref()
            .is_some_and(|pending| pending.matches(&self.config, target))
        {
            return false;
        }
        let pending = self.pending_creation.take().unwrap();
        let desktop_folder = matches!(pending, Pending::Group { .. });
        let accepted = if commit && !name.is_empty() {
            let mut candidate = self.config.clone();
            if pending.rename(&mut candidate, name) {
                match candidate.validate() {
                    Ok(()) => {
                        self.config = candidate;
                        true
                    }
                    Err(error) => {
                        self.status = error;
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        };
        if accepted {
            // Accepting the default name still needs the first durable save.
            self.save();
        } else {
            pending.remove(&mut self.config);
            self.group = self.group.min(self.config.groups.len().saturating_sub(1));
            if desktop_folder {
                self.trail.clear();
                self.rebuild_tiles();
            }
        }
        self.refresh();
        if !desktop_folder && had_focus && IsWindowVisible(self.popup).as_bool() {
            let _ = SetForegroundWindow(self.popup);
            let _ = SetFocus(self.popup);
        }
        true
    }

    unsafe fn cancel_creation(&mut self) {
        if let Some(pending) = self.pending_creation.take() {
            let desktop_folder = matches!(pending, Pending::Group { .. });
            pending.remove(&mut self.config);
            self.group = self.group.min(self.config.groups.len().saturating_sub(1));
            if desktop_folder {
                self.rebuild_tiles();
            }
            self.refresh();
        }
    }

    unsafe fn creation_position(&self) -> Option<(i32, i32)> {
        let work = platform::primary_work_area();
        let width = layout::TILE_WIDTH;
        let height = layout::tile_height(self.config.settings.show_item_counts);
        let usable_w = (work.right - work.left) as f32 / self.scale;
        let usable_h = (work.bottom - work.top) as f32 / self.scale;
        let step = |size: f32, cell: f32| {
            if self.config.settings.snap_to_grid && cell.is_finite() && cell >= 24. {
                (size / cell).ceil().max(1.) * cell
            } else {
                size + 24.
            }
        };
        let step_x = step(width, self.grid.cell_w);
        let step_y = step(height, self.grid.cell_h);
        // rebuild_tiles clamps before snapping. Test that same final position,
        // otherwise a bottom/right candidate can move onto an occupied slot.
        let max_x = (usable_w as i32 - 128).max(8);
        let max_y = (usable_h as i32 - 150).max(8);
        for row in 0..(usable_h / step_y).ceil() as usize {
            for col in 0..(usable_w / step_x).ceil() as usize {
                let (x, y) = self.v2_snap_position(
                    ((8. + col as f32 * step_x).round() as i32).clamp(8, max_x),
                    ((8. + row as f32 * step_y).round() as i32).clamp(8, max_y),
                );
                let overlaps = self.config.groups.iter().any(|group| {
                    (x as f32) < group.x as f32 + width
                        && x as f32 + width > group.x as f32
                        && (y as f32) < group.y as f32 + height
                        && y as f32 + height > group.y as f32
                });
                if !overlaps {
                    return Some((x, y));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_group_creation_restores_the_exact_existing_layout() {
        let mut config = Config::default();
        config.groups[0]
            .items
            .push(Item::shortcut("Editor", "editor.lnk"));
        let before = serde_json::to_vec(&config).unwrap();
        let pending = draft_group(&mut config, (400, 300)).unwrap();
        assert_eq!(config.groups.len(), 4);
        assert_eq!(
            serde_json::to_vec(&without_pending(&config, Some(&pending))).unwrap(),
            before,
            "saving another preference must not accept an unfinished folder"
        );
        pending.remove(&mut config);
        assert_eq!(serde_json::to_vec(&config).unwrap(), before);
    }

    #[test]
    fn cancelling_nested_creation_preserves_siblings_and_every_existing_id() {
        let mut config = Config::default();
        let mut parent = Item::folder("Tools");
        parent.children.push(Item::shortcut("Editor", "editor.lnk"));
        let parent_id = parent.id.clone();
        config.groups[0].items.push(parent);
        config.groups[0].items.push(Item::folder("Other"));
        let group_id = config.groups[0].id.clone();
        let before = serde_json::to_vec(&config).unwrap();
        let pending = draft_item(&mut config, &group_id, Some(&parent_id)).unwrap();
        assert_eq!(config.find_item(&parent_id).unwrap().children.len(), 2);
        pending.remove(&mut config);
        assert_eq!(serde_json::to_vec(&config).unwrap(), before);
    }

    #[test]
    fn default_names_fill_the_first_free_number_without_case_duplicates() {
        let names = ["new FOLDER", "New folder (2)", "New folder (4)", "Tools"];
        assert_eq!(default_name(names.into_iter()), "New folder (3)");
        assert_eq!(default_name(["Tools"].into_iter()), "New folder");
        let mut config = Config::default();
        config.groups[0].name = "New folder".into();
        let pending = draft_group(&mut config, (400, 300)).unwrap();
        let Pending::Group { id } = pending else {
            panic!()
        };
        assert_eq!(config.find_group(&id).unwrap().name, "New folder (2)");
    }

    #[test]
    fn invalid_destination_and_depth_limit_leave_the_layout_untouched() {
        let mut config = Config::default();
        let mut parent = Item::folder("Level 12");
        let deepest_id = parent.id.clone();
        for _ in 1..12 {
            let mut next = Item::folder("Parent");
            next.children.push(parent);
            parent = next;
        }
        config.groups[0].items.push(parent);
        let group_id = config.groups[0].id.clone();
        let before = serde_json::to_vec(&config).unwrap();
        assert!(draft_item(&mut config, &group_id, Some(&deepest_id)).is_err());
        assert!(draft_item(&mut config, &group_id, Some("missing")).is_err());
        assert_eq!(serde_json::to_vec(&config).unwrap(), before);
    }
}
