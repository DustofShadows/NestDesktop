//! Explicit, reversible collection of shortcuts from the user's Desktop.
//! Application binaries, documents, folders and public/other Desktop paths are
//! never moved. A durable ledger is written before each eligible shortcut moves.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_LEDGER_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 5_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollectedShortcut {
    pub original: String,
    pub stored: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<DesktopPosition>,
}

/// Coordinates in Explorer's Desktop folder view, captured before collection.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopPosition {
    pub x: i32,
    pub y: i32,
}

impl DesktopPosition {
    pub fn is_valid(self) -> bool {
        (-1_000_000..=1_000_000).contains(&self.x) && (-1_000_000..=1_000_000).contains(&self.y)
    }
}

/// A validated batch whose destination names are reserved in memory. No file
/// is moved until execute has durably written the complete recovery ledger.
pub struct CollectionPlan {
    storage: PathBuf,
    previous: Vec<CollectedShortcut>,
    entries: Vec<CollectedShortcut>,
    moves: Vec<CollectedShortcut>,
    reserved: Vec<CollectedShortcut>,
}

impl CollectionPlan {
    pub fn entries(&self) -> &[CollectedShortcut] {
        &self.entries
    }
    pub fn previous_records(&self) -> &[CollectedShortcut] {
        &self.previous
    }

    /// This is safe to replay after a crash even before the first move: every
    /// original still exists, so restoring an unused reservation is a no-op.
    pub fn reserve(&self) -> Result<(), String> {
        save_ledger(&self.storage, &self.reserved)
    }

    /// Returns only files newly moved by this execution. Existing collections
    /// are reused without emitting a physical-move notification.
    pub fn execute(
        &self,
        mut on_collected: impl FnMut(&CollectedShortcut),
    ) -> Result<Vec<CollectedShortcut>, String> {
        self.reserve()?;
        let mut moved = Vec::new();
        for entry in &self.moves {
            let source = Path::new(&entry.original);
            let inspected = fs::symlink_metadata(source)
                .map_err(|error| format!("Could not inspect the selected shortcut: {error}"))
                .and_then(|info| {
                    if info.is_file() && !is_link(&info) {
                        Ok(())
                    } else {
                        Err("The selected shortcut changed and was not moved.".into())
                    }
                });
            let mut destination_created = false;
            let result = inspected.and_then(|_| {
                move_without_replace_tracking(
                    source,
                    Path::new(&entry.stored),
                    &mut destination_created,
                )
            });
            if let Err(error) = result {
                let mut protected = self.previous.clone();
                // A failed cross-volume transfer can leave an intact second
                // copy. Never touch the failing destination or forget it.
                if destination_created && Path::new(&entry.stored).exists() {
                    replace_record(&mut protected, entry.clone());
                }
                let rollback = rollback_collection(&self.storage, &protected, &moved);
                return Err(match rollback {
                    Ok(()) => format!(
                        "The batch was not collected; newly moved shortcuts were returned. {error}"
                    ),
                    Err(rollback) => format!("{error}. Recovery records were retained: {rollback}"),
                });
            }
            moved.push(entry.clone());
            on_collected(entry);
        }
        Ok(moved)
    }
}

fn replace_record(records: &mut Vec<CollectedShortcut>, entry: CollectedShortcut) {
    records
        .retain(|old| path_key(Path::new(&old.original)) != path_key(Path::new(&entry.original)));
    records.push(entry);
}

/// Roll back only successful new moves. Old collections are never restored by
/// a failed batch, and name conflicts retain the new recovery record.
pub fn rollback_collection(
    storage: &Path,
    previous: &[CollectedShortcut],
    moved: &[CollectedShortcut],
) -> Result<(), String> {
    let mut remaining = previous.to_vec();
    let mut failures = Vec::new();
    for entry in moved.iter().rev() {
        if let Err(error) = restore_shortcut(entry) {
            replace_record(&mut remaining, entry.clone());
            failures.push(error);
        }
    }
    if let Err(error) = save_ledger(storage, &remaining) {
        failures.push(error);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// Plan only direct personal-Desktop shortcuts. Unlike the single-file helper,
/// an ineligible path rejects the batch before any recovery write or move.
pub fn plan_collection(
    paths: &[(PathBuf, Option<DesktopPosition>)],
    desktop: &Path,
    storage: &Path,
) -> Result<CollectionPlan, String> {
    let desktop = fs::canonicalize(desktop)
        .map_err(|error| format!("Could not resolve the user Desktop: {error}"))?;
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for (path, position) in paths {
        if !is_shortcut(path) || position.is_some_and(|value| !value.is_valid()) {
            return Err("The batch contains an invalid personal Desktop shortcut.".into());
        }
        let parent = path
            .parent()
            .ok_or("A selected shortcut has no parent folder.")?;
        let parent = fs::canonicalize(parent)
            .map_err(|error| format!("Could not inspect the selected folder: {error}"))?;
        if parent != desktop {
            return Err(
                "Only shortcuts directly on the personal Desktop can be collected in this batch."
                    .into(),
            );
        }
        let original = parent.join(
            path.file_name()
                .ok_or("A selected shortcut has no file name.")?,
        );
        let exists = match fs::symlink_metadata(&original) {
            Ok(info) if info.is_file() && !is_link(&info) => true,
            Ok(_) => return Err("A selected shortcut is not a regular file.".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("Could not inspect a selected shortcut: {error}")),
        };
        if seen.insert(path_key(&original)) {
            selected.push((original, exists, *position));
        }
    }
    fs::create_dir_all(storage)
        .map_err(|error| format!("Could not create shortcut storage: {error}"))?;
    let info = fs::symlink_metadata(storage).map_err(|error| error.to_string())?;
    if !info.is_dir() || is_link(&info) {
        return Err("Shortcut storage must be a normal local folder.".into());
    }
    let storage = fs::canonicalize(storage).map_err(|error| error.to_string())?;
    if storage == desktop {
        return Err("Shortcut storage must be separate from the Desktop.".into());
    }
    let previous = load_ledger(&storage)?;
    let mut reserved = previous.clone();
    let mut destinations: HashSet<_> = previous
        .iter()
        .map(|entry| path_key(Path::new(&entry.stored)))
        .collect();
    let mut entries = Vec::new();
    let mut moves = Vec::new();
    for (original, exists, position) in selected {
        let old = previous
            .iter()
            .find(|entry| path_key(Path::new(&entry.original)) == path_key(&original));
        if let Some(old) = old {
            let stored_exists = match fs::symlink_metadata(&old.stored) {
                Ok(info) if info.is_file() && !is_link(&info) => true,
                Ok(_) => return Err("An existing collected shortcut is not a regular file.".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return Err(format!(
                        "Could not inspect an existing collected shortcut: {error}"
                    ))
                }
            };
            if stored_exists {
                if exists {
                    return Err("A selected Desktop name already has a collected shortcut; neither file was changed.".into());
                }
                entries.push(old.clone());
                continue;
            }
            destinations.remove(&path_key(Path::new(&old.stored)));
        }
        if !exists {
            return Err("A selected Desktop shortcut is no longer available.".into());
        }
        let filename = original.file_name().ok_or("Shortcut has no file name.")?;
        let mut destination = None;
        for collision in 0..10_000 {
            let candidate = if collision == 0 {
                storage.join(filename)
            } else {
                storage.join(format!(
                    "{} ({}).{}",
                    original.file_stem().unwrap_or(filename).to_string_lossy(),
                    collision + 1,
                    original.extension().unwrap_or_default().to_string_lossy()
                ))
            };
            let vacant = match fs::symlink_metadata(&candidate) {
                Ok(_) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(error) => return Err(format!("Could not inspect shortcut storage: {error}")),
            };
            if vacant && !destinations.contains(&path_key(&candidate)) {
                destination = Some(candidate);
                break;
            }
        }
        let destination = destination.ok_or("Too many shortcuts with the same name in storage.")?;
        destinations.insert(path_key(&destination));
        let entry = CollectedShortcut {
            original: original.to_string_lossy().into_owned(),
            stored: destination.to_string_lossy().into_owned(),
            position: position.or_else(|| old.and_then(|entry| entry.position)),
        };
        replace_record(&mut reserved, entry.clone());
        entries.push(entry.clone());
        moves.push(entry);
    }
    validate_entries(&reserved)?;
    Ok(CollectionPlan {
        storage,
        previous,
        entries,
        moves,
        reserved,
    })
}

#[derive(Serialize, Deserialize)]
struct Ledger {
    version: u32,
    shortcuts: Vec<CollectedShortcut>,
}

/// Collects only a regular .lnk/.url directly inside the supplied user Desktop.
/// `storage` is the actual destination folder, containing both shortcuts and
/// collection.json. Returns None for ineligible files without changing them.
/// The destination is never overwritten, and the ledger is persisted first.
pub fn collect_shortcut(
    path: &Path,
    desktop: &Path,
    storage: &Path,
) -> Result<Option<CollectedShortcut>, String> {
    collect_shortcut_with_position(path, desktop, storage, None)
}

pub fn collect_shortcut_with_position(
    path: &Path,
    desktop: &Path,
    storage: &Path,
    position: Option<DesktopPosition>,
) -> Result<Option<CollectedShortcut>, String> {
    if position.is_some_and(|position| !position.is_valid()) {
        return Err("The saved Desktop icon position is invalid.".into());
    }
    if !is_shortcut(path) {
        return Ok(None);
    }
    let source_info = match fs::symlink_metadata(path) {
        Ok(info) => info,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Repeating a completed collection is harmless. Resolve the still
            // existing parent rather than canonicalizing the moved file.
            let desktop = fs::canonicalize(desktop)
                .map_err(|error| format!("Could not resolve the user Desktop: {error}"))?;
            if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
                if let Ok(parent) = fs::canonicalize(parent) {
                    if parent == desktop {
                        let original = parent.join(name);
                        if let Some(entry) = load_ledger(storage)?.into_iter().find(|entry| {
                            path_key(Path::new(&entry.original)) == path_key(&original)
                                && Path::new(&entry.stored).is_file()
                        }) {
                            return Ok(Some(entry));
                        }
                    }
                }
            }
            return Err(format!("Could not inspect shortcut: {error}"));
        }
        Err(error) => return Err(format!("Could not inspect shortcut: {error}")),
    };
    if is_link(&source_info) {
        return Err("Symbolic links and reparse-point shortcuts cannot be collected.".into());
    }
    if !source_info.is_file() {
        return Ok(None);
    }
    let source =
        fs::canonicalize(path).map_err(|error| format!("Could not resolve shortcut: {error}"))?;
    let desktop = fs::canonicalize(desktop)
        .map_err(|error| format!("Could not resolve the user Desktop: {error}"))?;
    if source.parent() != Some(desktop.as_path()) {
        return Ok(None);
    }

    fs::create_dir_all(storage)
        .map_err(|error| format!("Could not create shortcut storage: {error}"))?;
    let storage_info = fs::symlink_metadata(storage)
        .map_err(|error| format!("Could not inspect shortcut storage: {error}"))?;
    if is_link(&storage_info) || !storage_info.is_dir() {
        return Err("Shortcut storage must be a normal local folder.".into());
    }
    let storage = fs::canonicalize(storage)
        .map_err(|error| format!("Could not resolve shortcut storage: {error}"))?;
    if storage == desktop {
        return Err("Shortcut storage must be separate from the Desktop.".into());
    }
    let mut ledger = load_ledger(&storage)?;
    let original = source.to_string_lossy().into_owned();
    let previous_position = ledger
        .iter()
        .find(|entry| path_key(Path::new(&entry.original)) == path_key(&source))
        .and_then(|entry| entry.position);
    if let Some(previous) = ledger
        .iter()
        .find(|entry| path_key(Path::new(&entry.original)) == path_key(&source))
    {
        if Path::new(&previous.stored).exists() {
            return Err("This Desktop name already has a collected shortcut. Restore it before collecting another shortcut with the same name.".into());
        }
        // A pending pre-move ledger entry or an already restored entry can be
        // reused rather than creating a duplicate record.
    }
    ledger.retain(|entry| path_key(Path::new(&entry.original)) != path_key(&source));
    let filename = source.file_name().ok_or("Shortcut has no file name.")?;

    for collision in 0..10_000 {
        let destination = if collision == 0 {
            storage.join(filename)
        } else {
            let stem = source.file_stem().unwrap_or(filename).to_string_lossy();
            let extension = source.extension().unwrap_or_default().to_string_lossy();
            storage.join(format!(
                "{stem} ({}){dot}{extension}",
                collision + 1,
                dot = if extension.is_empty() { "" } else { "." }
            ))
        };
        if fs::symlink_metadata(&destination).is_ok() {
            continue;
        }
        let entry = CollectedShortcut {
            original: original.clone(),
            stored: destination.to_string_lossy().into_owned(),
            position: position.or(previous_position),
        };
        ledger.push(entry.clone());
        save_ledger(&storage, &ledger)?;
        match move_without_replace(&source, &destination) {
            Ok(()) => return Ok(Some(entry)),
            Err(error) => {
                // A failed cross-volume rollback may leave a second, intact
                // copy. Retain its record so it remains discoverable.
                if !destination.exists() {
                    ledger.pop();
                    if let Err(ledger_error) = save_ledger(&storage, &ledger) {
                        return Err(format!(
                            "{error}. The pending recovery record was retained: {ledger_error}"
                        ));
                    }
                }
                return Err(error);
            }
        }
    }
    Err("Too many shortcuts with the same name in shortcut storage.".into())
}

/// Restores a collected shortcut only if its original path is vacant. The
/// ledger is intentionally left untouched until the caller durably saves the
/// corresponding layout. A pre-move/already-restored record is a safe no-op.
pub fn restore_shortcut(entry: &CollectedShortcut) -> Result<(), String> {
    validate_entry(entry)?;
    let source = Path::new(&entry.stored);
    let destination = Path::new(&entry.original);
    let info = match fs::symlink_metadata(source) {
        Ok(info) => info,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return if destination.exists() {
                Ok(())
            } else {
                Err(format!(
                    "Neither the stored shortcut nor its original exists: {}",
                    entry.original
                ))
            };
        }
        Err(error) => return Err(format!("Could not inspect stored shortcut: {error}")),
    };
    if is_link(&info) || !info.is_file() {
        return Err(
            "The stored shortcut is not a regular file; it has been left untouched.".into(),
        );
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err(format!("A file already exists at {}. It has not been overwritten; the collected shortcut is still stored.", entry.original));
    }
    if !destination.parent().map(Path::is_dir).unwrap_or(false) {
        return Err(
            "The original Desktop folder no longer exists. The shortcut remains stored.".into(),
        );
    }
    move_without_replace(source, destination)
}

pub fn load_ledger(storage: &Path) -> Result<Vec<CollectedShortcut>, String> {
    let path = storage.join("collection.json");
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not open shortcut recovery ledger: {error}")),
    };
    let mut json = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_LEDGER_BYTES + 1)
        .read_to_end(&mut json)
        .map_err(|error| format!("Could not read shortcut recovery ledger: {error}"))?;
    parse_ledger(&json)
}

pub fn save_ledger(storage: &Path, entries: &[CollectedShortcut]) -> Result<(), String> {
    validate_entries(entries)?;
    fs::create_dir_all(storage)
        .map_err(|error| format!("Could not create shortcut recovery folder: {error}"))?;
    let path = storage.join("collection.json");
    let previous = match File::open(&path) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(MAX_LEDGER_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    format!("Could not read existing shortcut recovery ledger: {error}")
                })?;
            parse_ledger(&bytes)
                .map_err(|error| format!("Existing recovery ledger has been preserved: {error}"))?;
            Some(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Could not open existing shortcut recovery ledger: {error}"
            ))
        }
    };
    let bytes = serde_json::to_vec_pretty(&Ledger {
        version: 1,
        shortcuts: entries.to_vec(),
    })
    .map_err(|error| format!("Could not encode shortcut recovery ledger: {error}"))?;
    if bytes.len() as u64 > MAX_LEDGER_BYTES {
        return Err("Shortcut recovery ledger is too large.".into());
    }
    if previous.as_deref() == Some(bytes.as_slice()) {
        return Ok(());
    }
    let temporary = storage.join(format!("collection.{}.tmp", crate::model::new_id("save")));
    write_new(&temporary, &bytes)?;
    if let Some(previous) = previous {
        let backup_temp = storage.join(format!(
            "collection.{}.bak.tmp",
            crate::model::new_id("save")
        ));
        if let Err(error) = write_new(&backup_temp, &previous).and_then(|_| {
            fs::rename(&backup_temp, storage.join("collection.json.bak")).map_err(|e| e.to_string())
        }) {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_file(&backup_temp);
            return Err(format!(
                "Could not preserve shortcut recovery backup: {error}"
            ));
        }
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Could not replace shortcut recovery ledger: {error}"
        ));
    }
    Ok(())
}

fn parse_ledger(bytes: &[u8]) -> Result<Vec<CollectedShortcut>, String> {
    if bytes.len() as u64 > MAX_LEDGER_BYTES {
        return Err("Shortcut recovery ledger is too large.".into());
    }
    let ledger: Ledger = serde_json::from_slice(bytes)
        .map_err(|error| format!("Shortcut recovery ledger is unreadable: {error}"))?;
    if ledger.version != 1 {
        return Err("Unsupported shortcut recovery ledger version.".into());
    }
    validate_entries(&ledger.shortcuts)?;
    Ok(ledger.shortcuts)
}

fn validate_entries(entries: &[CollectedShortcut]) -> Result<(), String> {
    if entries.len() > MAX_ENTRIES {
        return Err("Too many shortcut recovery records.".into());
    }
    let mut originals = HashSet::new();
    let mut destinations = HashSet::new();
    for entry in entries {
        validate_entry(entry)?;
        if !originals.insert(path_key(Path::new(&entry.original)))
            || !destinations.insert(path_key(Path::new(&entry.stored)))
        {
            return Err("Duplicate shortcut recovery records.".into());
        }
    }
    Ok(())
}

fn validate_entry(entry: &CollectedShortcut) -> Result<(), String> {
    if entry.position.is_some_and(|position| !position.is_valid()) {
        return Err("Invalid Desktop icon position in shortcut recovery record.".into());
    }
    let original = Path::new(&entry.original);
    let stored = Path::new(&entry.stored);
    if !original.is_absolute()
        || !stored.is_absolute()
        || !is_shortcut(original)
        || !is_shortcut(stored)
        || entry.original.contains('\0')
        || entry.stored.contains('\0')
        || path_key(original) == path_key(stored)
        || original
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            != stored
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
    {
        return Err("Invalid shortcut recovery paths.".into());
    }
    Ok(())
}

fn is_shortcut(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("lnk" | "url")
    )
}

fn path_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(unc) = key.strip_prefix("\\\\?\\unc\\") {
        format!("\\\\{unc}")
    } else {
        key.strip_prefix("\\\\?\\").unwrap_or(&key).to_string()
    }
}

fn is_link(info: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        info.file_type().is_symlink() || info.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        info.file_type().is_symlink()
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    Ok(())
}

fn move_without_replace(source: &Path, destination: &Path) -> Result<(), String> {
    let mut destination_created = false;
    move_without_replace_tracking(source, destination, &mut destination_created)
}

fn move_without_replace_tracking(
    source: &Path,
    destination: &Path,
    destination_created: &mut bool,
) -> Result<(), String> {
    *destination_created = false;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
        }
        let source_w: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination_w: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe { MoveFileW(source_w.as_ptr(), destination_w.as_ptr()) } != 0 {
            *destination_created = true;
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(17) {
            // ERROR_NOT_SAME_DEVICE
            return Err(format!(
                "Could not move shortcut; existing files were not overwritten: {error}"
            ));
        }
    }
    #[cfg(not(windows))]
    {
        match fs::hard_link(source, destination) {
            Ok(()) => {
                *destination_created = true;
                return fs::remove_file(source).map_err(|error| {
                    let _ = fs::remove_file(destination);
                    format!("Could not remove original shortcut: {error}")
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(error.to_string())
            }
            Err(_) => {}
        }
    }
    copy_then_remove_tracking(source, destination, destination_created)
}

fn copy_then_remove(source: &Path, destination: &Path) -> Result<(), String> {
    let mut destination_created = false;
    copy_then_remove_tracking(source, destination, &mut destination_created)
}

fn copy_then_remove_tracking(
    source: &Path,
    destination: &Path,
    destination_created: &mut bool,
) -> Result<(), String> {
    *destination_created = false;
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // FILE_SHARE_READ: prevent writes while copying.
        }
        let mut input = options.open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        *destination_created = true;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        drop(input);
        fs::remove_file(source)
    })();
    if let Err(error) = result {
        // Only remove a destination created by this attempt. If create_new
        // failed because it already existed, it must remain untouched.
        if *destination_created && source.exists() {
            let _ = fs::remove_file(destination);
        }
        return Err(format!(
            "Could not transfer shortcut; the original was preserved: {error}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let root = std::env::temp_dir().join(crate::model::new_id("nest-shortcuts-test"));
            fs::create_dir_all(root.join("Desktop")).unwrap();
            Self(root)
        }
        fn desktop(&self) -> PathBuf {
            self.0.join("Desktop")
        }
        fn storage(&self) -> PathBuf {
            self.0.join("Storage")
        }
        fn shortcut(&self, name: &str) -> PathBuf {
            let path = self.desktop().join(name);
            fs::write(&path, b"original shortcut bytes").unwrap();
            path
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn batch_reserves_distinct_names_without_reusing_pending_destinations() {
        let sandbox = Sandbox::new();
        fs::create_dir_all(sandbox.storage()).unwrap();
        fs::write(sandbox.storage().join("App.lnk"), b"unrelated storage file").unwrap();
        let pending = CollectedShortcut {
            original: sandbox
                .desktop()
                .join("Pending.lnk")
                .to_string_lossy()
                .into_owned(),
            stored: fs::canonicalize(sandbox.storage())
                .unwrap()
                .join("App (2).lnk")
                .to_string_lossy()
                .into_owned(),
            position: None,
        };
        save_ledger(&sandbox.storage(), &[pending.clone()]).unwrap();
        let paths = [
            (sandbox.shortcut("App.lnk"), None),
            (sandbox.shortcut("App (2).lnk"), None),
        ];
        let plan = plan_collection(&paths, &sandbox.desktop(), &sandbox.storage()).unwrap();
        let destinations: HashSet<_> = plan
            .entries()
            .iter()
            .map(|entry| path_key(Path::new(&entry.stored)))
            .collect();
        assert_eq!(destinations.len(), 2);
        assert!(!destinations.contains(&path_key(Path::new(&pending.stored))));
        plan.execute(|_| {}).unwrap();
        assert_eq!(
            fs::read(sandbox.storage().join("App.lnk")).unwrap(),
            b"unrelated storage file"
        );
        assert!(load_ledger(&sandbox.storage()).unwrap().contains(&pending));
        for entry in plan.entries() {
            assert_eq!(fs::read(&entry.stored).unwrap(), b"original shortcut bytes");
        }
    }

    #[test]
    fn batch_reservation_is_complete_and_safe_to_replay_before_any_move() {
        let sandbox = Sandbox::new();
        let position = DesktopPosition { x: -24, y: 184 };
        let paths = [
            (sandbox.shortcut("One.lnk"), Some(position)),
            (sandbox.shortcut("Two.url"), None),
        ];
        let plan = plan_collection(&paths, &sandbox.desktop(), &sandbox.storage()).unwrap();
        plan.reserve().unwrap(); // Simulated interruption before the first move.
        assert_eq!(load_ledger(&sandbox.storage()).unwrap(), plan.entries());
        for entry in plan.entries() {
            assert!(Path::new(&entry.original).is_file());
            assert!(!Path::new(&entry.stored).exists());
            restore_shortcut(entry).unwrap();
        }
        assert_eq!(plan.entries()[0].position, Some(position));
        let mut notified = 0;
        plan.execute(|_| {
            notified += 1;
            assert_eq!(load_ledger(&sandbox.storage()).unwrap(), plan.entries());
        })
        .unwrap();
        assert_eq!(notified, 2);
    }

    #[test]
    fn failed_batch_returns_only_new_moves_and_preserves_existing_collections() {
        let sandbox = Sandbox::new();
        let old = collect_shortcut(
            &sandbox.shortcut("Old.lnk"),
            &sandbox.desktop(),
            &sandbox.storage(),
        )
        .unwrap()
        .unwrap();
        let first = sandbox.shortcut("First.lnk");
        let second = sandbox.shortcut("Second.lnk");
        let held = sandbox.0.join("Second-held.lnk");
        let plan = plan_collection(
            &[(first.clone(), None), (second.clone(), None)],
            &sandbox.desktop(),
            &sandbox.storage(),
        )
        .unwrap();
        let mut notified = 0;
        let failed = plan.execute(|_| {
            notified += 1;
            fs::rename(&second, &held).unwrap();
        });
        assert!(failed.is_err());
        assert_eq!(notified, 1);
        assert_eq!(fs::read(&first).unwrap(), b"original shortcut bytes");
        assert_eq!(fs::read(&held).unwrap(), b"original shortcut bytes");
        assert!(Path::new(&old.stored).is_file());
        assert!(!Path::new(&old.original).exists());
        assert_eq!(load_ledger(&sandbox.storage()).unwrap(), [old]);
        assert!(plan
            .entries()
            .iter()
            .all(|entry| !Path::new(&entry.stored).exists()));
    }

    #[test]
    fn failed_batch_rollback_conflict_retains_bytes_position_and_recovery_record() {
        let sandbox = Sandbox::new();
        let first = sandbox.shortcut("First.lnk");
        let second = sandbox.shortcut("Second.lnk");
        let position = DesktopPosition { x: 232, y: 460 };
        let plan = plan_collection(
            &[(first.clone(), Some(position)), (second.clone(), None)],
            &sandbox.desktop(),
            &sandbox.storage(),
        )
        .unwrap();
        let failed = plan.execute(|_| {
            fs::write(&first, b"new unrelated desktop file").unwrap();
            fs::rename(&second, sandbox.0.join("Second-held.lnk")).unwrap();
        });
        assert!(failed
            .unwrap_err()
            .contains("Recovery records were retained"));
        assert_eq!(fs::read(&first).unwrap(), b"new unrelated desktop file");
        let remaining = load_ledger(&sandbox.storage()).unwrap();
        assert_eq!(remaining, [plan.entries()[0].clone()]);
        assert_eq!(remaining[0].position, Some(position));
        assert_eq!(
            fs::read(&remaining[0].stored).unwrap(),
            b"original shortcut bytes"
        );
        fs::remove_file(&first).unwrap();
        restore_shortcut(&remaining[0]).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"original shortcut bytes");
    }

    #[test]
    fn a_destination_race_is_not_claimed_as_a_failed_batch_collection() {
        let sandbox = Sandbox::new();
        let first = sandbox.shortcut("First.lnk");
        let second = sandbox.shortcut("Second.lnk");
        let plan = plan_collection(
            &[(first.clone(), None), (second.clone(), None)],
            &sandbox.desktop(),
            &sandbox.storage(),
        )
        .unwrap();
        let occupied = Path::new(&plan.entries()[1].stored);
        let failed =
            plan.execute(|_| fs::write(occupied, b"unrelated racing storage file").unwrap());
        assert!(failed.is_err());
        assert_eq!(fs::read(&first).unwrap(), b"original shortcut bytes");
        assert_eq!(fs::read(&second).unwrap(), b"original shortcut bytes");
        assert_eq!(
            fs::read(occupied).unwrap(),
            b"unrelated racing storage file"
        );
        assert!(load_ledger(&sandbox.storage()).unwrap().is_empty());
    }

    #[test]
    fn collection_and_restore_preserve_bytes_and_recovery_record() {
        let sandbox = Sandbox::new();
        let source = sandbox.shortcut("Editor.lnk");
        let entry = collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage())
            .unwrap()
            .unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(&entry.stored).unwrap(), b"original shortcut bytes");
        assert_eq!(
            load_ledger(&sandbox.storage()).unwrap(),
            vec![entry.clone()]
        );
        assert_eq!(
            collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage()).unwrap(),
            Some(entry.clone())
        );
        assert_eq!(load_ledger(&sandbox.storage()).unwrap().len(), 1);
        restore_shortcut(&entry).unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"original shortcut bytes");
        assert!(!Path::new(&entry.stored).exists());
        assert_eq!(
            load_ledger(&sandbox.storage()).unwrap(),
            vec![entry.clone()]
        );
        restore_shortcut(&entry).unwrap(); // Safe retry after restore.
    }

    #[test]
    fn legacy_records_load_without_a_position_and_invalid_positions_are_preserved() {
        let sandbox = Sandbox::new();
        fs::create_dir_all(sandbox.storage()).unwrap();
        let source = sandbox.shortcut("Editor.lnk");
        let ledger_path = sandbox.storage().join("collection.json");
        let mut json = serde_json::json!({"version": 1, "shortcuts": [{
            "original": source, "stored": sandbox.storage().join("Editor.lnk")
        }]});
        fs::write(&ledger_path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert_eq!(load_ledger(&sandbox.storage()).unwrap()[0].position, None);
        json["shortcuts"][0]["position"] = serde_json::json!({"x": i32::MIN, "y": 0});
        let invalid = serde_json::to_vec(&json).unwrap();
        fs::write(&ledger_path, &invalid).unwrap();
        assert!(load_ledger(&sandbox.storage()).is_err());
        assert!(save_ledger(&sandbox.storage(), &[]).is_err());
        assert_eq!(fs::read(&ledger_path).unwrap(), invalid);
    }

    #[test]
    fn position_is_durable_before_collection_and_duplicate_saves_keep_last_change_backup() {
        let sandbox = Sandbox::new();
        let source = sandbox.shortcut("Editor.lnk");
        save_ledger(&sandbox.storage(), &[]).unwrap();
        let before = fs::read(sandbox.storage().join("collection.json")).unwrap();
        let position = DesktopPosition { x: -124, y: 348 };
        let entry = collect_shortcut_with_position(
            &source,
            &sandbox.desktop(),
            &sandbox.storage(),
            Some(position),
        )
        .unwrap()
        .unwrap();
        assert_eq!(load_ledger(&sandbox.storage()).unwrap(), [entry.clone()]);
        assert_eq!(entry.position, Some(position));
        let path = sandbox.storage().join("collection.json");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        save_ledger(&sandbox.storage(), &[entry.clone()]).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(
            fs::read(sandbox.storage().join("collection.json.bak")).unwrap(),
            before
        );
        assert_eq!(
            collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage()).unwrap(),
            Some(entry.clone())
        );
        restore_shortcut(&entry).unwrap();
        let reacquired = collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage())
            .unwrap()
            .unwrap();
        assert_eq!(reacquired.position, Some(position));
    }

    #[test]
    fn occupied_storage_names_are_preserved_and_uniquely_named() {
        let sandbox = Sandbox::new();
        fs::create_dir_all(sandbox.storage()).unwrap();
        let occupied = sandbox.storage().join("Editor.lnk");
        fs::write(&occupied, b"unrelated shortcut").unwrap();
        let source = sandbox.shortcut("Editor.lnk");
        let entry = collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage())
            .unwrap()
            .unwrap();
        assert_ne!(Path::new(&entry.stored), occupied);
        assert_eq!(fs::read(occupied).unwrap(), b"unrelated shortcut");
        assert_eq!(fs::read(&entry.stored).unwrap(), b"original shortcut bytes");
    }

    #[test]
    fn restore_never_overwrites_a_new_desktop_file() {
        let sandbox = Sandbox::new();
        let source = sandbox.shortcut("Website.URL");
        let entry = collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage())
            .unwrap()
            .unwrap();
        fs::write(&source, b"new shortcut on desktop").unwrap();
        assert!(restore_shortcut(&entry).is_err());
        assert_eq!(fs::read(source).unwrap(), b"new shortcut on desktop");
        assert_eq!(fs::read(entry.stored).unwrap(), b"original shortcut bytes");
    }

    #[test]
    fn apps_documents_folders_and_other_desktops_are_not_moved() {
        let sandbox = Sandbox::new();
        for name in ["App.exe", "Notes.txt"] {
            let source = sandbox.shortcut(name);
            assert!(
                collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage())
                    .unwrap()
                    .is_none()
            );
            assert!(source.exists());
        }
        let directory = sandbox.desktop().join("Folder.lnk");
        fs::create_dir(&directory).unwrap();
        assert!(
            collect_shortcut(&directory, &sandbox.desktop(), &sandbox.storage())
                .unwrap()
                .is_none()
        );
        let outside = sandbox.0.join("OtherDesktop.lnk");
        fs::write(&outside, b"outside").unwrap();
        assert!(
            collect_shortcut(&outside, &sandbox.desktop(), &sandbox.storage())
                .unwrap()
                .is_none()
        );
        let nested = sandbox.desktop().join("Subfolder");
        fs::create_dir(&nested).unwrap();
        let nested_file = nested.join("Nested.lnk");
        fs::write(&nested_file, b"nested").unwrap();
        assert!(
            collect_shortcut(&nested_file, &sandbox.desktop(), &sandbox.storage())
                .unwrap()
                .is_none()
        );
        assert!(outside.exists() && nested_file.exists());
        assert!(!sandbox.storage().exists());
    }

    #[test]
    fn corrupt_ledger_blocks_collection_before_any_move() {
        let sandbox = Sandbox::new();
        fs::create_dir_all(sandbox.storage()).unwrap();
        let ledger = sandbox.storage().join("collection.json");
        fs::write(&ledger, b"broken ledger").unwrap();
        let source = sandbox.shortcut("Editor.lnk");
        assert!(collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage()).is_err());
        assert!(source.exists());
        assert_eq!(fs::read(ledger).unwrap(), b"broken ledger");
    }

    #[test]
    fn pre_move_recovery_entry_is_idempotent_and_collect_can_resume() {
        let sandbox = Sandbox::new();
        let source = sandbox.shortcut("Editor.lnk");
        fs::create_dir_all(sandbox.storage()).unwrap();
        let entry = CollectedShortcut {
            original: fs::canonicalize(&source)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            stored: fs::canonicalize(sandbox.storage())
                .unwrap()
                .join("Editor.lnk")
                .to_string_lossy()
                .into_owned(),
            position: None,
        };
        save_ledger(&sandbox.storage(), &[entry.clone()]).unwrap();
        restore_shortcut(&entry).unwrap();
        assert!(source.exists());
        let actual = collect_shortcut(&source, &sandbox.desktop(), &sandbox.storage())
            .unwrap()
            .unwrap();
        assert_eq!(load_ledger(&sandbox.storage()).unwrap(), vec![actual]);
    }

    #[test]
    fn cross_volume_copy_path_never_overwrites_or_loses_original() {
        let sandbox = Sandbox::new();
        let source = sandbox.shortcut("Editor.lnk");
        let destination = sandbox.0.join("Copy.lnk");
        fs::write(&destination, b"occupied").unwrap();
        assert!(copy_then_remove(&source, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"occupied");
        assert!(source.exists());
        fs::remove_file(&destination).unwrap();
        copy_then_remove(&source, &destination).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"original shortcut bytes");
    }

    #[test]
    fn failed_source_open_never_removes_an_unrelated_destination() {
        let sandbox = Sandbox::new();
        let source = sandbox.desktop().join("Missing.lnk");
        let destination = sandbox.0.join("Occupied.lnk");
        fs::write(&destination, b"unrelated existing shortcut").unwrap();
        assert!(copy_then_remove(&source, &destination).is_err());
        assert_eq!(
            fs::read(destination).unwrap(),
            b"unrelated existing shortcut"
        );
    }
}
