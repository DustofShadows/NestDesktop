//! Transactional layout changes for Android-style drag-and-drop grouping.
//!
//! These helpers only edit the layout model. Original files and applications
//! are never moved, copied, launched, or removed.

use crate::model::{Config, Item};

/// Move an item to a group's root or onto another item at any nesting depth.
///
/// Dropping onto a folder appends the item to that folder. Dropping onto an
/// application replaces that application, at its existing position, with a
/// new folder containing the target followed by the dragged item. The new
/// folder's ID is returned so the caller can offer to rename it.
///
/// Any error leaves `config` unchanged, including errors from model limits.
pub fn drop_item(
    config: &mut Config,
    source_group: usize,
    source_id: &str,
    destination_group: usize,
    destination_id: Option<&str>,
) -> Result<Option<String>, String> {
    config.validate()?;
    let source = config
        .groups
        .get(source_group)
        .ok_or_else(|| "The source desktop folder no longer exists.".to_string())?
        .find_item(source_id)
        .ok_or_else(|| "The dragged item no longer exists in its source folder.".to_string())?;
    let destination = config
        .groups
        .get(destination_group)
        .ok_or_else(|| "The destination desktop folder no longer exists.".to_string())?;

    if let Some(id) = destination_id {
        if source_id == id {
            return Err("An item cannot be dropped onto itself.".into());
        }
        destination
            .find_item(id)
            .ok_or_else(|| "The destination item no longer exists in this folder.".to_string())?;
        if contains_id(&source.children, id) {
            return Err("A folder cannot be moved into one of its own descendants.".into());
        }
    }

    // Work on a candidate so even a depth/item-limit failure preserves the
    // complete original layout, item ordering, shortcut IDs and arguments.
    let mut candidate = config.clone();
    let source = candidate.groups[source_group]
        .remove_item(source_id)
        .ok_or_else(|| "The dragged item could not be found.".to_string())?;
    let created = if let Some(id) = destination_id {
        let target = candidate.groups[destination_group]
            .find_item_mut(id)
            .ok_or_else(|| "The destination item could not be found.".to_string())?;
        if target.is_folder() {
            target.children.push(source);
            None
        } else {
            let folder = Item::folder("New folder");
            let folder_id = folder.id.clone();
            let original_target = std::mem::replace(target, folder);
            target.children.push(original_target);
            target.children.push(source);
            Some(folder_id)
        }
    } else {
        candidate.groups[destination_group].items.push(source);
        None
    };
    candidate.validate()?;
    *config = candidate;
    Ok(created)
}

/// Reorder an item among the immediate children of one desktop or nested folder.
///
/// `before_id` names the sibling that should follow the moved item; None moves
/// it to the end. This never groups items or moves them across parent folders.
/// Invalid destinations and model-limit failures leave the layout unchanged.
pub fn reorder_item(
    config: &mut Config,
    group: usize,
    parent_id: Option<&str>,
    source_id: &str,
    before_id: Option<&str>,
) -> Result<(), String> {
    config.validate()?;
    let mut candidate = config.clone();
    let group = candidate
        .groups
        .get_mut(group)
        .ok_or_else(|| "The desktop folder no longer exists.".to_string())?;
    let siblings = if let Some(parent_id) = parent_id {
        let parent = group
            .find_item_mut(parent_id)
            .ok_or_else(|| "The parent folder no longer exists.".to_string())?;
        if !parent.is_folder() {
            return Err("Items can only be arranged inside a folder.".into());
        }
        &mut parent.children
    } else {
        &mut group.items
    };
    let source_index = siblings
        .iter()
        .position(|item| item.id == source_id)
        .ok_or_else(|| "The dragged item is not in this folder.".to_string())?;
    if before_id == Some(source_id) {
        return Ok(());
    }
    if let Some(before_id) = before_id {
        if !siblings.iter().any(|item| item.id == before_id) {
            return Err("The destination item is not in this folder.".into());
        }
    }
    let source = siblings.remove(source_index);
    let index = before_id
        .and_then(|id| siblings.iter().position(|item| item.id == id))
        .unwrap_or(siblings.len());
    siblings.insert(index, source);
    candidate.validate()?;
    *config = candidate;
    Ok(())
}

fn contains_id(items: &[Item], id: &str) -> bool {
    items
        .iter()
        .any(|item| item.id == id || contains_id(&item.children, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(name: &str) -> Item {
        let mut item = Item::shortcut(name, format!("C:\\Apps\\{name}.exe"));
        item.arguments = format!("--profile \"{name} profile\"");
        item
    }

    #[test]
    fn moves_nested_item_across_groups_without_changing_its_identity() {
        let mut config = Config::default();
        let app = shortcut("Editor");
        let mut folder = Item::folder("Tools");
        folder.children.push(app.clone());
        let folder_id = folder.id.clone();
        config.groups[0].items.push(folder);
        let existing = shortcut("Browser");
        config.groups[1].items.push(existing.clone());

        assert_eq!(drop_item(&mut config, 0, &app.id, 1, None), Ok(None));
        assert!(config.groups[0]
            .find_item(&folder_id)
            .unwrap()
            .children
            .is_empty());
        assert_eq!(config.groups[1].items, [existing, app]);
        config.validate().unwrap();
    }

    #[test]
    fn dropping_on_nested_app_groups_at_its_original_position() {
        let mut config = Config::default();
        let source = shortcut("Editor");
        let target = shortcut("Terminal");
        let before = shortcut("Before");
        let after = shortcut("After");
        config.groups[0].items.push(source.clone());
        let mut parent = Item::folder("Utilities");
        parent.children = vec![before.clone(), target.clone(), after.clone()];
        let parent_id = parent.id.clone();
        config.groups[1].items.push(parent);

        let new_id = drop_item(&mut config, 0, &source.id, 1, Some(&target.id))
            .unwrap()
            .expect("dropping on an app creates a folder");
        let children = &config.groups[1].find_item(&parent_id).unwrap().children;
        assert_eq!(children.len(), 3);
        assert_eq!(children[0], before);
        assert_eq!(children[2], after);
        assert_eq!(children[1].id, new_id);
        assert_eq!(children[1].name, "New folder");
        assert_eq!(children[1].children, [target, source]);
        assert!(config.groups[0].items.is_empty());
        config.validate().unwrap();
    }

    #[test]
    fn dropping_on_folder_moves_it_instead_of_creating_an_extra_wrapper() {
        let mut config = Config::default();
        let source = shortcut("Editor");
        let mut folder = Item::folder("Utilities");
        let existing = shortcut("Terminal");
        folder.children.push(existing.clone());
        let destination_id = folder.id.clone();
        config.groups[0].items = vec![source.clone(), folder];

        assert_eq!(
            drop_item(&mut config, 0, &source.id, 0, Some(&destination_id)),
            Ok(None)
        );
        assert_eq!(config.groups[0].items.len(), 1);
        assert_eq!(config.groups[0].items[0].children, [existing, source]);
        config.validate().unwrap();
    }

    #[test]
    fn rejects_a_folder_dropped_on_descendant_folder_or_app_without_mutation() {
        let mut config = Config::default();
        let leaf = shortcut("Editor");
        let mut nested = Item::folder("Nested");
        nested.children.push(leaf.clone());
        let nested_id = nested.id.clone();
        let mut source = Item::folder("Source");
        source.children.push(nested);
        let source_id = source.id.clone();
        config.groups[0].items.push(source);
        let before = config.clone();

        for destination in [&nested_id, &leaf.id] {
            assert!(drop_item(&mut config, 0, &source_id, 0, Some(destination)).is_err());
            assert_eq!(config, before);
        }
    }

    #[test]
    fn rejects_same_target_and_missing_indices_or_ids_without_mutation() {
        let mut config = Config::default();
        let source = shortcut("Editor");
        let target = shortcut("Terminal");
        config.groups[0].items.push(source.clone());
        config.groups[1].items.push(target.clone());
        let before = config.clone();

        for (source_group, source_id, destination_group, destination_id) in [
            (0, source.id.as_str(), 0, Some(source.id.as_str())),
            (99, source.id.as_str(), 1, None),
            (0, source.id.as_str(), 99, None),
            (1, source.id.as_str(), 0, None),
            (0, "missing", 1, None),
            (0, source.id.as_str(), 0, Some(target.id.as_str())),
            (0, source.id.as_str(), 1, Some("missing")),
        ] {
            assert!(drop_item(
                &mut config,
                source_group,
                source_id,
                destination_group,
                destination_id
            )
            .is_err());
            assert_eq!(config, before);
        }
    }

    #[test]
    fn depth_limit_failure_preserves_both_source_and_destination() {
        let mut config = Config::default();
        let source = shortcut("Editor");
        config.groups[0].items.push(source.clone());
        let mut branch = Item::folder("Deepest");
        let deepest_id = branch.id.clone();
        for level in 1..12 {
            let mut parent = Item::folder(format!("Level {level}"));
            parent.children.push(branch);
            branch = parent;
        }
        config.groups[1].items.push(branch);
        config.validate().unwrap();
        let before = config.clone();

        assert!(drop_item(&mut config, 0, &source.id, 1, Some(&deepest_id)).is_err());
        assert_eq!(config, before);
    }

    #[test]
    fn grouping_at_item_limit_rolls_back_the_new_folder_and_both_apps() {
        let mut config = Config::default();
        config.groups[0].items = (0..10_000).map(|i| shortcut(&format!("App{i}"))).collect();
        config.validate().unwrap();
        let source_id = config.groups[0].items[0].id.clone();
        let target_id = config.groups[0].items[1].id.clone();
        let before = config.clone();

        assert!(drop_item(&mut config, 0, &source_id, 0, Some(&target_id)).is_err());
        assert_eq!(config, before);
        // Moving an existing item does not increase the item count.
        assert_eq!(drop_item(&mut config, 0, &source_id, 1, None), Ok(None));
        assert_eq!(config.groups[0].items.len(), 9_999);
        assert_eq!(config.groups[1].items.len(), 1);
        config.validate().unwrap();
    }

    #[test]
    fn reordering_root_items_preserves_identity_arguments_and_relative_order() {
        let mut config = Config::default();
        let first = shortcut("First");
        let middle = shortcut("Middle");
        let last = shortcut("Last");
        config.groups[0].items = vec![first.clone(), middle.clone(), last.clone()];
        reorder_item(&mut config, 0, None, &last.id, Some(&first.id)).unwrap();
        assert_eq!(
            config.groups[0].items,
            [last.clone(), first.clone(), middle.clone()]
        );
        reorder_item(&mut config, 0, None, &last.id, None).unwrap();
        assert_eq!(config.groups[0].items, [first, middle, last]);
    }

    #[test]
    fn reordering_nested_items_keeps_their_parent_and_other_groups_unchanged() {
        let mut config = Config::default();
        let first = shortcut("First");
        let middle = shortcut("Middle");
        let last = shortcut("Last");
        let mut nested = Item::folder("Nested");
        let parent_id = nested.id.clone();
        nested.children = vec![first.clone(), middle.clone(), last.clone()];
        let outer = shortcut("Outside");
        config.groups[0].items = vec![outer.clone(), nested];
        let other_groups = config.groups[1..].to_vec();
        reorder_item(&mut config, 0, Some(&parent_id), &first.id, Some(&last.id)).unwrap();
        assert_eq!(config.groups[0].items[0], outer);
        assert_eq!(
            config.groups[0].find_item(&parent_id).unwrap().children,
            [middle, first, last]
        );
        assert_eq!(config.groups[1..], other_groups);
    }

    #[test]
    fn reorder_same_item_is_a_noop_and_invalid_siblings_never_move_items() {
        let mut config = Config::default();
        let first = shortcut("First");
        let child = shortcut("Child");
        let mut nested = Item::folder("Nested");
        let nested_id = nested.id.clone();
        nested.children.push(child.clone());
        config.groups[0].items = vec![first.clone(), nested];
        let before = config.clone();
        reorder_item(&mut config, 0, None, &first.id, Some(&first.id)).unwrap();
        assert_eq!(config, before);
        for (group, parent, source, destination) in [
            (99, None, first.id.as_str(), None),
            (0, Some("missing"), first.id.as_str(), None),
            (0, Some(first.id.as_str()), first.id.as_str(), None),
            (0, None, "missing", None),
            (0, None, first.id.as_str(), Some("missing")),
            (0, None, first.id.as_str(), Some(child.id.as_str())),
            (
                0,
                Some(nested_id.as_str()),
                first.id.as_str(),
                Some(child.id.as_str()),
            ),
            (0, None, child.id.as_str(), Some(first.id.as_str())),
        ] {
            assert!(reorder_item(&mut config, group, parent, source, destination).is_err());
            assert_eq!(config, before);
        }
    }
}
