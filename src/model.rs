//! Local, versioned layout storage. App entries are shortcuts; this module never
//! moves, launches, or deletes the applications and documents they refer to.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_GROUP_COLOR: &str = "#8CA9F8";
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_GROUPS: usize = 128;
const MAX_ITEMS: usize = 10_000;
const MAX_DEPTH: usize = 12;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub version: u32,
    pub groups: Vec<Group>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub color: String,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// An empty target denotes a folder, including an empty folder.
    pub target: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub children: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub start_with_windows: bool,
    /// "system", "light", or "dark".
    pub theme: String,
    pub tile_size: i32,
    pub show_item_counts: bool,
    pub snap_to_grid: bool,
    pub hover_open: bool,
    pub hover_delay_ms: u32,
    /// Up to sixteen custom picker swatches, stored independently of groups.
    pub custom_colors: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            theme: "system".into(),
            tile_size: 112,
            show_item_counts: false,
            snap_to_grid: true,
            hover_open: true,
            hover_delay_ms: 450,
            custom_colors: Vec::new(),
        }
    }
}

/// Parse the seconds shown in settings, accepting either decimal separator.
/// Keep the durable preference in whole milliseconds so restart is exact.
pub fn parse_hover_delay_seconds(value: &str) -> Result<u32, String> {
    let seconds = value
        .trim()
        .replace(',', ".")
        .parse::<f64>()
        .map_err(|_| "Enter a delay between 0.10 and 5.00 seconds.".to_string())?;
    if !seconds.is_finite() || !(0.1..=5.).contains(&seconds) {
        return Err("Enter a delay between 0.10 and 5.00 seconds.".into());
    }
    Ok((seconds * 1000.).round() as u32)
}

impl Default for Config {
    fn default() -> Self {
        let groups = [
            ("Work", DEFAULT_GROUP_COLOR),
            ("Create", "#C69AF5"),
            ("Play", "#90D5BA"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (name, color))| Group {
            color: color.into(),
            x: 48 + i as i32 * 136,
            y: 64,
            ..Group::new(name)
        })
        .collect();
        Self {
            version: SCHEMA_VERSION,
            groups,
            settings: Settings::default(),
        }
    }
}

pub fn new_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}-{nanos:x}-{:x}",
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

impl Group {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id("group"),
            name: name.into(),
            color: DEFAULT_GROUP_COLOR.into(),
            x: 48,
            y: 64,
            items: Vec::new(),
        }
    }

    pub fn find_item(&self, id: &str) -> Option<&Item> {
        find_in(&self.items, id)
    }

    pub fn find_item_mut(&mut self, id: &str) -> Option<&mut Item> {
        find_in_mut(&mut self.items, id)
    }

    pub fn remove_item(&mut self, id: &str) -> Option<Item> {
        remove_from(&mut self.items, id)
    }
}

impl Item {
    pub fn shortcut(name: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            id: new_id("app"),
            name: name.into(),
            target: target.into(),
            arguments: String::new(),
            children: Vec::new(),
        }
    }

    pub fn folder(name: impl Into<String>) -> Self {
        Self {
            id: new_id("folder"),
            name: name.into(),
            target: String::new(),
            arguments: String::new(),
            children: Vec::new(),
        }
    }

    pub fn is_folder(&self) -> bool {
        self.target.is_empty()
    }
}

impl Config {
    /// A missing file means first launch; all other errors are returned so that
    /// the caller can offer recovery instead of silently losing a layout.
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Could not open layout: {error}")),
        };
        let mut json = String::new();
        Read::by_ref(&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_string(&mut json)
            .map_err(|error| format!("Could not read layout: {error}"))?;
        if json.len() as u64 > MAX_FILE_BYTES {
            return Err("Layout is too large (maximum 8 MB).".into());
        }
        Self::from_json(&json)
    }

    /// Validates imports with exactly the same rules as the local layout.
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() as u64 > MAX_FILE_BYTES {
            return Err("Layout is too large (maximum 8 MB).".into());
        }
        let config: Self = serde_json::from_str(json)
            .map_err(|error| format!("Layout could not be read: {error}"))?;
        config.validate()?;
        Ok(config)
    }

    /// Writes in the destination directory, flushes before replacement, and
    /// keeps the last readable layout at `<file>.bak`. Invalid existing files
    /// are never silently replaced, even by a newly-created default layout.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let json = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Could not encode layout: {error}"))?;
        if json.len() as u64 > MAX_FILE_BYTES {
            return Err("Layout is too large (maximum 8 MB).".into());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create layout folder: {error}"))?;

        if let Ok(metadata) = fs::metadata(path) {
            if metadata.len() > MAX_FILE_BYTES {
                return Err("Existing layout is too large; it has been preserved.".into());
            }
        }
        let existing = match fs::read(path) {
            Ok(bytes) => {
                let text = std::str::from_utf8(&bytes).map_err(|_| {
                    "Existing layout is unreadable; it has been preserved.".to_string()
                })?;
                Self::from_json(text).map_err(|_| {
                    "Existing layout is invalid; it has been preserved. Restore its .bak file or choose a new destination.".to_string()
                })?;
                Some(bytes)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("Could not check existing layout: {error}")),
        };

        // Preserve the last meaningful backup and avoid synchronous writes for
        // paint/settings/quit paths that do not change the durable layout.
        if existing.as_deref() == Some(json.as_slice()) {
            return Ok(());
        }

        let temporary = sibling_path(path, &format!(".{}.tmp", new_id("save")));
        write_new_synced(&temporary, &json)?;
        if let Some(bytes) = existing {
            let backup = sibling_path(path, ".bak");
            let backup_temporary = sibling_path(path, &format!(".{}.bak.tmp", new_id("save")));
            if let Err(error) = write_new_synced(&backup_temporary, &bytes)
                .and_then(|_| replace(&backup_temporary, &backup))
            {
                let _ = fs::remove_file(&temporary);
                let _ = fs::remove_file(&backup_temporary);
                return Err(error);
            }
        }
        if let Err(error) = replace(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != SCHEMA_VERSION {
            return Err(format!(
                "Unsupported layout version {} (expected {}).",
                self.version, SCHEMA_VERSION
            ));
        }
        if self.groups.len() > MAX_GROUPS {
            return Err(format!("A layout can contain at most {MAX_GROUPS} groups."));
        }
        if !matches!(self.settings.theme.as_str(), "system" | "light" | "dark") {
            return Err("Theme must be system, light, or dark.".into());
        }
        if !(80..=240).contains(&self.settings.tile_size) {
            return Err("Tile size must be between 80 and 240.".into());
        }
        if !(100..=5000).contains(&self.settings.hover_delay_ms) {
            return Err("Hover delay must be between 100 and 5000 milliseconds.".into());
        }
        if self.settings.custom_colors.len() > 16 {
            return Err("The custom color palette can contain at most 16 colors.".into());
        }
        if self
            .settings
            .custom_colors
            .iter()
            .any(|color| !valid_color(color))
        {
            return Err("Custom colors must use #RRGGBB format.".into());
        }
        let mut ids = HashSet::new();
        let mut count = 0;
        for group in &self.groups {
            check_id(&group.id, &mut ids)?;
            check_name(&group.name)?;
            if !valid_color(&group.color) {
                return Err("Group colors must use #RRGGBB format.".into());
            }
            for item in &group.items {
                validate_item(item, 1, &mut ids, &mut count)?;
            }
        }
        Ok(())
    }

    pub fn find_group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == id)
    }

    pub fn find_group_mut(&mut self, id: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|group| group.id == id)
    }

    pub fn find_item(&self, id: &str) -> Option<&Item> {
        self.groups.iter().find_map(|group| group.find_item(id))
    }

    pub fn find_item_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.groups
            .iter_mut()
            .find_map(|group| group.find_item_mut(id))
    }

    pub fn remove_item(&mut self, id: &str) -> Option<Item> {
        self.groups
            .iter_mut()
            .find_map(|group| group.remove_item(id))
    }
}

fn valid_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|c| c.is_ascii_hexdigit())
}

fn find_in<'a>(items: &'a [Item], id: &str) -> Option<&'a Item> {
    for item in items {
        if item.id == id {
            return Some(item);
        }
        if let Some(found) = find_in(&item.children, id) {
            return Some(found);
        }
    }
    None
}

fn find_in_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Item> {
    for item in items {
        if item.id == id {
            return Some(item);
        }
        if let Some(found) = find_in_mut(&mut item.children, id) {
            return Some(found);
        }
    }
    None
}

fn remove_from(items: &mut Vec<Item>, id: &str) -> Option<Item> {
    if let Some(index) = items.iter().position(|item| item.id == id) {
        return Some(items.remove(index));
    }
    items
        .iter_mut()
        .find_map(|item| remove_from(&mut item.children, id))
}

fn validate_item(
    item: &Item,
    depth: usize,
    ids: &mut HashSet<String>,
    count: &mut usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "Folders can be nested at most {MAX_DEPTH} levels deep."
        ));
    }
    *count += 1;
    if *count > MAX_ITEMS {
        return Err(format!("A layout can contain at most {MAX_ITEMS} items."));
    }
    check_id(&item.id, ids)?;
    check_name(&item.name)?;
    if item.target.contains('\0') || item.arguments.contains('\0') {
        return Err("Shortcut paths and arguments cannot contain a NUL character.".into());
    }
    if item.target.len() > 131_072 || item.arguments.len() > 131_072 {
        return Err("Shortcut path or arguments are too long.".into());
    }
    if !item.target.is_empty() && !item.children.is_empty() {
        return Err("A shortcut cannot also contain child items.".into());
    }
    for child in &item.children {
        validate_item(child, depth + 1, ids, count)?;
    }
    Ok(())
}

fn check_id(id: &str, ids: &mut HashSet<String>) -> Result<(), String> {
    if id.is_empty() || id.len() > 256 || id.contains('\0') {
        return Err("Every group and item needs a valid identifier.".into());
    }
    if !ids.insert(id.into()) {
        return Err(format!("The layout contains a duplicate identifier: {id}"));
    }
    Ok(())
}

fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.chars().count() > 128 || name.chars().any(|c| c.is_control())
    {
        return Err("Names must contain 1–128 characters and no control characters.".into());
    }
    Ok(())
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(format!("Could not save layout: {error}"));
    }
    Ok(())
}

fn replace(source: &Path, destination: &Path) -> Result<(), String> {
    // std::fs::rename replaces a writable existing file on Windows as well as
    // Unix. Both paths are siblings so replacement stays on one filesystem.
    fs::rename(source, destination)
        .map_err(|error| format!("Could not replace layout file: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFolder(PathBuf);
    impl TestFolder {
        fn new() -> Self {
            let path = std::env::temp_dir().join(new_id("nestdesktop-test"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> PathBuf {
            self.0.join("layout.json")
        }
    }
    impl Drop for TestFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_layout_uses_empty_real_groups() {
        let folder = TestFolder::new();
        let config = Config::load(&folder.file()).unwrap();
        assert_eq!(
            config
                .groups
                .iter()
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>(),
            ["Work", "Create", "Play"]
        );
        assert!(config.groups.iter().all(|g| g.items.is_empty()));
        assert!(!folder.file().exists());
    }

    #[test]
    fn nested_layout_roundtrips_and_previous_layout_is_backed_up() {
        let folder = TestFolder::new();
        let path = folder.file();
        let mut config = Config::default();
        let mut nested = Item::folder("Utilities");
        let mut app = Item::shortcut("Editor", "C:\\Program Files\\Editor\\Editor.exe");
        app.arguments = "--new-window".into();
        nested.children.push(app);
        config.groups[0].items.push(nested);
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        let previous = config.clone();
        config.groups[0].name = "Focus".into();
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        assert_eq!(
            Config::load(&sibling_path(&path, ".bak")).unwrap(),
            previous
        );
    }

    #[test]
    fn saving_unchanged_layout_keeps_the_previous_distinct_backup_and_file_time() {
        let folder = TestFolder::new();
        let path = folder.file();
        let mut config = Config::default();
        config.save(&path).unwrap();
        let original = fs::read(&path).unwrap();
        config.groups[0].name = "Focus".into();
        config.save(&path).unwrap();
        let saved = fs::read(&path).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let backup = sibling_path(&path, ".bak");
        let backup_modified = fs::metadata(&backup).unwrap().modified().unwrap();
        config.save(&path).unwrap();
        config.save(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(
            fs::metadata(&backup).unwrap().modified().unwrap(),
            backup_modified
        );
    }

    #[test]
    fn malformed_existing_layout_is_preserved() {
        let folder = TestFolder::new();
        let path = folder.file();
        let bad = b"{\"version\": 1, \"groups\": [";
        fs::write(&path, bad).unwrap();
        assert!(Config::load(&path).is_err());
        assert!(Config::default().save(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), bad);
    }

    #[test]
    fn invalid_mutation_does_not_replace_valid_layout() {
        let folder = TestFolder::new();
        let path = folder.file();
        let mut config = Config::default();
        config.save(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        config.groups[0].name.clear();
        assert!(config.save(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn duplicate_ids_unsupported_versions_and_mixed_items_are_rejected() {
        let mut config = Config::default();
        config.groups[1].id = config.groups[0].id.clone();
        assert!(config.validate().is_err());
        config = Config::default();
        config.version = 999;
        assert!(Config::from_json(&serde_json::to_string(&config).unwrap()).is_err());
        config = Config::default();
        let mut mixed = Item::shortcut("App", "app.exe");
        mixed.children.push(Item::folder("Invalid child"));
        config.groups[0].items.push(mixed);
        assert!(config.validate().is_err());
    }

    #[test]
    fn nested_lookup_removal_and_depth_limit_work() {
        let mut config = Config::default();
        let mut branch = Item::folder("Deepest");
        let leaf_id = branch.id.clone();
        for n in 1..MAX_DEPTH {
            let mut parent = Item::folder(format!("Level {n}"));
            parent.children.push(branch);
            branch = parent;
        }
        config.groups[0].items.push(branch);
        assert!(config.validate().is_ok());
        assert_eq!(config.find_item(&leaf_id).unwrap().name, "Deepest");
        config
            .find_item_mut(&leaf_id)
            .unwrap()
            .children
            .push(Item::folder("Too deep"));
        assert!(config.validate().is_err());
        let removed = config.remove_item(&leaf_id).unwrap();
        assert_eq!(removed.name, "Deepest");
        assert!(config.find_item(&leaf_id).is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn unicode_names_survive_and_bad_colors_are_rejected_without_panicking() {
        let mut config = Config::default();
        config.groups[0].name = "Création 🎨".into();
        assert!(Config::from_json(&serde_json::to_string(&config).unwrap()).is_ok());
        config.groups[0].color = "#éabcd".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn legacy_profiles_gain_compact_hover_and_grid_defaults() {
        let legacy = r#"{"version":1,"groups":[],"settings":{"start_with_windows":true,"theme":"dark","tile_size":112}}"#;
        let config = Config::from_json(legacy).unwrap();
        assert_eq!(config.version, 1);
        assert!(config.settings.start_with_windows);
        assert_eq!(config.settings.theme, "dark");
        assert!(!config.settings.show_item_counts);
        assert!(config.settings.snap_to_grid);
        assert!(config.settings.hover_open);
        assert_eq!(config.settings.hover_delay_ms, 450);
        assert!(config.settings.custom_colors.is_empty());
    }

    #[test]
    fn appearance_preferences_and_custom_palette_roundtrip() {
        let mut config = Config::default();
        config.settings.show_item_counts = true;
        config.settings.snap_to_grid = false;
        config.settings.hover_open = false;
        config.settings.hover_delay_ms = 850;
        config.settings.custom_colors = vec!["#aBcD09".into(), "#123456".into()];
        let decoded = Config::from_json(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn hover_delay_seconds_accepts_local_decimals_and_rejects_invalid_ranges() {
        for (value, expected) in [
            ("0.45", 450),
            (" 0,8 ", 800),
            ("0.10", 100),
            ("5", 5000),
            ("0.456", 456),
        ] {
            assert_eq!(parse_hover_delay_seconds(value).unwrap(), expected);
        }
        for value in [
            "", "0", "0.099", "5.001", "-1", "NaN", "inf", "0,4.5", "seconds",
        ] {
            assert!(parse_hover_delay_seconds(value).is_err(), "{value}");
        }
    }

    #[test]
    fn custom_palette_and_hover_delay_limits_are_checked_on_import() {
        let mut config = Config::default();
        config.settings.custom_colors = vec!["#123456".into(); 16];
        assert!(config.validate().is_ok());
        config.settings.custom_colors.push("#abcdef".into());
        assert!(config.validate().is_err());
        config.settings.custom_colors = vec!["#éabcd".into()];
        assert!(config.validate().is_err());
        config.settings.custom_colors = vec!["123456".into()];
        assert!(config.validate().is_err());
        config.settings.custom_colors.clear();
        for delay in [0, 99, 5001, u32::MAX] {
            config.settings.hover_delay_ms = delay;
            assert!(Config::from_json(&serde_json::to_string(&config).unwrap()).is_err());
        }
    }
}
