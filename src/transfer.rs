//! Conservative relinking of imported layouts against this device's app list.
//! Importing a layout never launches a shortcut or changes sign-in startup.

use crate::model::{Config, Item};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Keeps existing targets and shell/protocol targets. A missing filesystem
/// target is replaced only when its display name or former file stem matches
/// exactly one distinct target in the local catalogue (case-insensitively).
/// Returns the number of missing shortcut leaves that could not be relinked.
pub fn relink_config(config: &mut Config, catalogue: &[(String, String)]) -> usize {
    let mut aliases: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, (name, target)) in catalogue.iter().enumerate() {
        if target.is_empty() || (!is_nonfilesystem_target(target) && !Path::new(target).exists()) {
            continue;
        }
        for alias in item_aliases(name, target) {
            aliases.entry(alias).or_default().push(index);
        }
    }
    let mut unresolved = 0;
    for group in &mut config.groups {
        relink_items(&mut group.items, catalogue, &aliases, &mut unresolved);
    }
    unresolved
}

fn relink_items(
    items: &mut [Item],
    catalogue: &[(String, String)],
    aliases: &HashMap<String, Vec<usize>>,
    unresolved: &mut usize,
) {
    for item in items {
        if item.is_folder() {
            relink_items(&mut item.children, catalogue, aliases, unresolved);
            continue;
        }
        if is_nonfilesystem_target(&item.target) || Path::new(&item.target).exists() {
            continue;
        }
        let mut candidates: HashMap<String, usize> = HashMap::new();
        for alias in item_aliases(&item.name, &item.target) {
            if let Some(indices) = aliases.get(&alias) {
                for &index in indices {
                    // The same physical target may appear under several aliases
                    // or be discovered in more than one catalogue entry.
                    let key = catalogue[index].1.replace('/', "\\").to_lowercase();
                    candidates.entry(key).or_insert(index);
                }
            }
        }
        if candidates.len() == 1 {
            let index = *candidates.values().next().unwrap();
            item.target = catalogue[index].1.clone();
        } else {
            *unresolved += 1;
        }
    }
}

fn item_aliases(name: &str, target: &str) -> HashSet<String> {
    let mut result = HashSet::new();
    let label = normalized_name(name);
    if !label.is_empty() {
        result.insert(label);
    }
    if !is_nonfilesystem_target(target) {
        // Accept either Windows or forward-slash paths in exported layouts.
        let filename = target.rsplit(['\\', '/']).next().unwrap_or(target);
        let stem = Path::new(filename)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(filename);
        let alias = normalized_name(stem);
        if !alias.is_empty() {
            result.insert(alias);
        }
    }
    result
}

fn normalized_name(name: &str) -> String {
    let mut value = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    // Display names occasionally include the shortcut extension as well.
    for extension in [".lnk", ".url", ".exe", ".appref-ms"] {
        if value.ends_with(extension) {
            value.truncate(value.len() - extension.len());
            value = value.trim_end().to_string();
            break;
        }
    }
    loop {
        let before = value.len();
        for suffix in [" - shortcut", " - verknüpfung"] {
            if value.ends_with(suffix) {
                value.truncate(value.len() - suffix.len());
                value = value.trim_end().to_string();
                break;
            }
        }
        if value.len() == before {
            return value;
        }
    }
}

fn is_nonfilesystem_target(target: &str) -> bool {
    if target.starts_with("::{") {
        return true;
    }
    let Some(colon) = target.find(':') else {
        return false;
    };
    // A drive prefix, including a drive-relative path, is not a URI scheme.
    if colon == 1 && target.as_bytes()[0].is_ascii_alphabetic() {
        return false;
    }
    let scheme = &target[..colon];
    !scheme.is_empty()
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::new_id;
    use std::fs;
    use std::path::PathBuf;

    struct TestFolder(PathBuf);
    impl TestFolder {
        fn new() -> Self {
            let path = std::env::temp_dir().join(new_id("nestdesktop-transfer-test"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn app(&self, filename: &str) -> String {
            let path = self.0.join(filename);
            fs::write(&path, b"test shortcut").unwrap();
            path.to_string_lossy().into_owned()
        }
        fn missing(&self, filename: &str) -> String {
            self.0.join(filename).to_string_lossy().into_owned()
        }
    }
    impl Drop for TestFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nested_shortcuts_relink_by_name_and_old_file_stem() {
        let files = TestFolder::new();
        let paint = files.app("FreshPaint.lnk");
        let browser = files.app("Firefox.lnk");
        let mut config = Config::default();
        config.settings.start_with_windows = true;
        let mut folder = Item::folder("Nested");
        folder.children.push(Item::shortcut(
            "  PAINT   - Shortcut  ",
            files.missing("old-paint.lnk"),
        ));
        folder.children.push(Item::shortcut(
            "My browser",
            files.missing("FIREFOX - Verknüpfung.lnk"),
        ));
        config.groups[0].items.push(folder);
        let catalogue = vec![
            ("Paint".into(), paint.clone()),
            ("Browser".into(), browser.clone()),
        ];
        assert_eq!(relink_config(&mut config, &catalogue), 0);
        assert_eq!(config.groups[0].items[0].children[0].target, paint);
        assert_eq!(config.groups[0].items[0].children[1].target, browser);
        assert!(config.settings.start_with_windows);
    }

    #[test]
    fn ambiguous_names_and_conflicting_aliases_are_never_guessed() {
        let files = TestFolder::new();
        let first = files.app("EditorA.lnk");
        let second = files.app("EditorB.lnk");
        let old = files.missing("missing.lnk");
        let mut config = Config::default();
        config.groups[0].items.push(Item::shortcut("Editor", &old));
        let catalogue = vec![
            ("Editor".into(), first),
            ("EDITOR - Shortcut".into(), second),
        ];
        assert_eq!(relink_config(&mut config, &catalogue), 1);
        assert_eq!(config.groups[0].items[0].target, old);

        let mut conflicting = Config::default();
        let old = files.missing("old-device\\EditorB.lnk");
        conflicting.groups[0]
            .items
            .push(Item::shortcut("EditorA", &old));
        assert_eq!(relink_config(&mut conflicting, &catalogue), 1);
        assert_eq!(conflicting.groups[0].items[0].target, old);
    }

    #[test]
    fn existing_files_and_nonfilesystem_targets_are_preserved() {
        let files = TestFolder::new();
        let existing = files.app("Existing.lnk");
        let replacement = files.app("Other.lnk");
        let mut config = Config::default();
        for target in [
            &existing,
            "shell:AppsFolder\\Microsoft.App!App",
            "https://example.com",
            "ms-settings:display",
        ] {
            config.groups[0]
                .items
                .push(Item::shortcut("Same name", target));
        }
        let before = config.clone();
        assert_eq!(
            relink_config(&mut config, &[("Same name".into(), replacement)]),
            0
        );
        assert_eq!(config, before);
    }

    #[test]
    fn duplicate_catalogue_aliases_for_one_target_are_not_ambiguous() {
        let files = TestFolder::new();
        let existing = files.app("Editor.lnk");
        let mut config = Config::default();
        config.groups[0]
            .items
            .push(Item::shortcut("Editor", files.missing("old.lnk")));
        let catalogue = vec![
            ("Editor".into(), existing.clone()),
            ("Editor - Shortcut".into(), existing.clone()),
        ];
        assert_eq!(relink_config(&mut config, &catalogue), 0);
        assert_eq!(config.groups[0].items[0].target, existing);
    }

    #[test]
    fn missing_items_are_counted_and_empty_folders_are_ignored() {
        let files = TestFolder::new();
        let mut config = Config::default();
        let mut folder = Item::folder("Folder");
        folder.children.push(Item::folder("Empty"));
        folder
            .children
            .push(Item::shortcut("One", files.missing("one.exe")));
        folder
            .children
            .push(Item::shortcut("Two", files.missing("two.lnk")));
        config.groups[1].items.push(folder);
        assert_eq!(relink_config(&mut config, &[]), 2);
    }

    #[test]
    fn stale_catalogue_entries_do_not_replace_one_missing_target_with_another() {
        let files = TestFolder::new();
        let old = files.missing("old-device\\Editor.lnk");
        let mut config = Config::default();
        config.groups[0].items.push(Item::shortcut("Editor", &old));
        let stale = vec![("Editor".into(), files.missing("deleted-app\\Editor.lnk"))];
        assert_eq!(relink_config(&mut config, &stale), 1);
        assert_eq!(config.groups[0].items[0].target, old);
    }

    #[test]
    fn drive_paths_are_not_misidentified_as_protocols() {
        assert!(!is_nonfilesystem_target("C:\\Apps\\Editor.exe"));
        assert!(!is_nonfilesystem_target("D:Editor.exe"));
        assert!(!is_nonfilesystem_target("\\\\server\\folder\\app.lnk"));
        assert!(is_nonfilesystem_target("shell:AppsFolder\\App!App"));
        assert!(is_nonfilesystem_target("custom-app+name:open"));
        assert!(!is_nonfilesystem_target("123:invalid"));
    }
}
