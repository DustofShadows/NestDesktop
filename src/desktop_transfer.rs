//! Durable ownership for explicitly dropped user-Desktop shortcuts.
//!
//! `collection.json` is the active, pre-move recovery ledger. This module keeps
//! a separate managed list across normal exit/recovery so a later startup can
//! reacquire those same shortcuts, and only those still used by the layout.
use crate::model::{self, Config, Item};
use crate::shortcuts::{self, CollectedShortcut, DesktopPosition};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 5_000;

#[derive(Default, Debug)]
pub struct Report {
    pub collected: usize,
    pub restored: usize,
    pub conflicts: usize,
    /// Active records retained for a later recovery attempt.
    pub remaining: usize,
    /// Restoration/reacquisition errors other than occupied Desktop names.
    pub failures: usize,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Managed {
    version: u32,
    shortcuts: Vec<CollectedShortcut>,
}

fn storage(layout: &Path) -> PathBuf {
    layout.parent().unwrap_or(Path::new(".")).join("shortcuts")
}
fn registry(layout: &Path) -> PathBuf {
    layout
        .parent()
        .unwrap_or(Path::new("."))
        .join("managed-shortcuts.json")
}
fn key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(tail) = value.strip_prefix("\\\\?\\unc\\") {
        format!("\\\\{tail}")
    } else {
        value.strip_prefix("\\\\?\\").unwrap_or(&value).to_string()
    }
}
fn shell(path: &str) -> String {
    if let Some(tail) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{tail}")
    } else {
        path.strip_prefix("\\\\?\\").unwrap_or(path).to_string()
    }
}
fn shortcut(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("lnk" | "url")
    )
}
fn references(config: &Config, target: &str) -> bool {
    fn walk(items: &[Item], wanted: &str) -> bool {
        items
            .iter()
            .any(|i| key(Path::new(&i.target)) == wanted || walk(&i.children, wanted))
    }
    let wanted = key(Path::new(target));
    config.groups.iter().any(|g| walk(&g.items, &wanted))
}
pub fn remap_targets(config: &mut Config, old: &str, new: &str) {
    fn walk(items: &mut [Item], old: &str, new: &str) {
        for item in items {
            if key(Path::new(&item.target)) == old {
                item.target = shell(new);
            }
            walk(&mut item.children, old, new);
        }
    }
    let old = key(Path::new(old));
    for group in &mut config.groups {
        walk(&mut group.items, &old, new);
    }
}
fn eligible(path: &Path, desktop: &Path) -> Result<bool, String> {
    if !shortcut(path) {
        return Ok(false);
    }
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    let parent = fs::canonicalize(parent)
        .map_err(|e| format!("Could not inspect the dropped shortcut folder: {e}"))?;
    let desktop = fs::canonicalize(desktop)
        .map_err(|e| format!("Could not resolve the user Desktop: {e}"))?;
    Ok(parent == desktop)
}
fn validate_bindings(layout: &Path, entries: &[CollectedShortcut]) -> Result<(), String> {
    if entries.len() > MAX_ENTRIES {
        return Err("Too many managed shortcut records.".into());
    }
    let expected = fs::canonicalize(storage(layout))
        .unwrap_or(std::path::absolute(storage(layout)).map_err(|e| e.to_string())?);
    let mut originals = HashSet::new();
    let mut destinations = HashSet::new();
    for entry in entries {
        if entry.position.is_some_and(|position| !position.is_valid()) {
            return Err(
                "Invalid managed Desktop icon position; the list was left unchanged.".into(),
            );
        }
        let original = Path::new(&entry.original);
        let stored = Path::new(&entry.stored);
        let valid = original.is_absolute()
            && stored.is_absolute()
            && shortcut(original)
            && shortcut(stored)
            && !entry.original.contains('\0')
            && !entry.stored.contains('\0')
            && original
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                == stored
                    .extension()
                    .map(|x| x.to_string_lossy().to_lowercase())
            && stored.parent().map(key) == Some(key(&expected))
            && key(original) != key(stored);
        if !valid || !originals.insert(key(original)) || !destinations.insert(key(stored)) {
            return Err(
                "Invalid or duplicate managed shortcut records; the list was left unchanged."
                    .into(),
            );
        }
    }
    Ok(())
}
fn read_binding_bytes(layout: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = registry(layout);
    let mut file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Could not read managed shortcuts: {e}")),
    };
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Managed shortcut list is too large.".into());
    }
    Ok(Some(bytes))
}
fn parse_bindings(layout: &Path, bytes: &[u8]) -> Result<Vec<CollectedShortcut>, String> {
    let managed: Managed = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Managed shortcut list is unreadable and was preserved: {e}"))?;
    if managed.version != 1 {
        return Err("Unsupported managed shortcut list version.".into());
    }
    validate_bindings(layout, &managed.shortcuts)?;
    Ok(managed.shortcuts)
}
pub fn load_bindings(layout: &Path) -> Result<Vec<CollectedShortcut>, String> {
    match read_binding_bytes(layout)? {
        Some(bytes) => parse_bindings(layout, &bytes),
        None => Ok(Vec::new()),
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
fn save_bindings(layout: &Path, entries: &[CollectedShortcut]) -> Result<(), String> {
    validate_bindings(layout, entries)?;
    let path = registry(layout);
    let previous = read_binding_bytes(layout)?;
    if let Some(previous) = previous.as_deref() {
        parse_bindings(layout, previous)?;
    }
    fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&Managed {
        version: 1,
        shortcuts: entries.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Managed shortcut list is too large.".into());
    }
    if previous.as_deref() == Some(bytes.as_slice()) {
        return Ok(());
    }
    let temp = path.with_extension(format!("{}.tmp", model::new_id("save")));
    write_new(&temp, &bytes)?;
    if let Some(previous) = previous {
        let backup = path.with_extension("json.bak");
        let backup_temp = path.with_extension(format!("{}.bak.tmp", model::new_id("save")));
        if let Err(error) = write_new(&backup_temp, &previous)
            .and_then(|_| fs::rename(&backup_temp, &backup).map_err(|e| e.to_string()))
        {
            let _ = fs::remove_file(&temp);
            let _ = fs::remove_file(&backup_temp);
            return Err(error);
        }
    }
    if let Err(error) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(error.to_string());
    }
    Ok(())
}
fn replace_binding(entries: &mut Vec<CollectedShortcut>, entry: CollectedShortcut) {
    let original = key(Path::new(&entry.original));
    entries.retain(|e| key(Path::new(&e.original)) != original);
    entries.push(entry);
}
fn remove_active(layout: &Path, entry: &CollectedShortcut) -> Result<(), String> {
    let mut entries = shortcuts::load_ledger(&storage(layout))?;
    entries.retain(|e| key(Path::new(&e.original)) != key(Path::new(&entry.original)));
    shortcuts::save_ledger(&storage(layout), &entries)
}
fn destination<'a>(
    config: &'a mut Config,
    group: usize,
    parent: Option<&str>,
) -> Result<&'a mut Vec<Item>, String> {
    let group = config
        .groups
        .get_mut(group)
        .ok_or("The destination desktop folder no longer exists.")?;
    if let Some(parent) = parent {
        let item = group
            .find_item_mut(parent)
            .ok_or("The destination nested folder no longer exists.")?;
        if !item.is_folder() {
            return Err("Drop shortcuts into a folder.".into());
        }
        Ok(&mut item.children)
    } else {
        Ok(&mut group.items)
    }
}

/// Explicit incoming drop/collection. False means a non-user-Desktop file: the
/// caller can add a reference without moving that file. A successful true result
/// already saved the layout. A failed save restores a newly moved shortcut.
pub fn collect_into(
    config: &mut Config,
    layout: &Path,
    desktop: &Path,
    path: &Path,
    group: usize,
    parent: Option<&str>,
) -> Result<bool, String> {
    collect_into_with_position(config, layout, desktop, path, group, parent, None)
}

pub fn collect_into_with_position(
    config: &mut Config,
    layout: &Path,
    desktop: &Path,
    path: &Path,
    group: usize,
    parent: Option<&str>,
    position: Option<DesktopPosition>,
) -> Result<bool, String> {
    config.validate()?;
    if !eligible(path, desktop)? {
        return Ok(false);
    }
    let old_bindings = load_bindings(layout)?;
    let position = position.or_else(|| {
        old_bindings
            .iter()
            .find(|entry| key(Path::new(&entry.original)) == key(path))
            .and_then(|entry| entry.position)
    });
    let old_ledger = shortcuts::load_ledger(&storage(layout))?;
    let already = old_ledger
        .iter()
        .find(|e| key(Path::new(&e.original)) == key(path) && Path::new(&e.stored).is_file());
    let target = already
        .map(|e| e.stored.clone())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let mut candidate = config.clone();
    let items = destination(&mut candidate, group, parent)?;
    if !items
        .iter()
        .any(|i| key(Path::new(&i.target)) == key(Path::new(&target)))
    {
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "App".into());
        items.push(Item::shortcut(name, shell(&target)));
    }
    candidate.validate()?;
    let moved_now = path.exists() && already.is_none();
    let Some(entry) =
        shortcuts::collect_shortcut_with_position(path, desktop, &storage(layout), position)?
    else {
        return Ok(false);
    };
    remap_targets(&mut candidate, &entry.original, &entry.stored);
    let mut next_bindings = old_bindings.clone();
    replace_binding(&mut next_bindings, entry.clone());
    let result = save_bindings(layout, &next_bindings).and_then(|_| candidate.save(layout));
    if let Err(error) = result {
        let mut rollback = Vec::new();
        if moved_now {
            match shortcuts::restore_shortcut(&entry) {
                Ok(()) => {
                    if let Err(e) = shortcuts::save_ledger(&storage(layout), &old_ledger) {
                        rollback.push(e);
                    }
                }
                Err(e) => rollback.push(e),
            }
        }
        if let Err(e) = save_bindings(layout, &old_bindings) {
            rollback.push(e);
        }
        return Err(if rollback.is_empty() {
            format!(
                "The folder could not be saved; the shortcut was returned to the Desktop. {error}"
            )
        } else {
            format!(
                "{error}. Recovery records were retained: {}",
                rollback.join("; ")
            )
        });
    }
    *config = candidate;
    Ok(true)
}

/// Collect an eligible selection as one durable transaction. The candidate is
/// validated before the pre-move ledger, and each physical move can notify the
/// Shell without waiting for the final ownership/layout writes.
pub fn collect_many_into_with_positions(
    config: &mut Config,
    layout: &Path,
    desktop: &Path,
    paths: &[(PathBuf, Option<DesktopPosition>)],
    group: usize,
    parent: Option<&str>,
    mut on_collected: impl FnMut(&CollectedShortcut),
) -> Result<usize, String> {
    config.validate()?;
    if paths.is_empty() {
        return Ok(0);
    }
    let mut old_bindings = load_bindings(layout)?;
    let paths: Vec<_> = paths
        .iter()
        .map(|(path, position)| {
            let position = position.or_else(|| {
                old_bindings
                    .iter()
                    .find(|entry| key(Path::new(&entry.original)) == key(path))
                    .and_then(|entry| entry.position)
            });
            (path.clone(), position)
        })
        .collect();
    let plan = shortcuts::plan_collection(&paths, desktop, &storage(layout))?;
    let mut candidate = config.clone();
    for entry in plan.entries() {
        let items = destination(&mut candidate, group, parent)?;
        if !items.iter().any(|item| {
            key(Path::new(&item.target)) == key(Path::new(&entry.stored))
                || key(Path::new(&item.target)) == key(Path::new(&entry.original))
        }) {
            let name = Path::new(&entry.original)
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "App".into());
            items.push(Item::shortcut(name, shell(&entry.stored)));
        }
        remap_targets(&mut candidate, &entry.original, &entry.stored);
    }
    candidate.validate()?;
    // Distinguish a new precommit reservation from a legacy collection whose
    // owner list did not yet exist. An interrupted new batch must not acquire
    // ownership merely because an old layout contained an ordinary alias.
    if !registry(layout).exists() {
        // Preserve ownership of genuine old-version collections, while an
        // ordinary layout alias to a newly selected source does not acquire it.
        for entry in plan.previous_records() {
            if Path::new(&entry.stored).is_file()
                && (references(config, &entry.stored)
                    || (!Path::new(&entry.original).exists()
                        && references(config, &entry.original)))
            {
                replace_binding(&mut old_bindings, entry.clone());
            }
        }
        save_bindings(layout, &old_bindings)?;
    }
    let moved = plan.execute(|entry| on_collected(entry))?;
    let mut bindings = old_bindings.clone();
    for entry in plan.entries() {
        replace_binding(&mut bindings, entry.clone());
    }
    if let Err(error) = save_bindings(layout, &bindings).and_then(|_| candidate.save(layout)) {
        let mut failures = Vec::new();
        if let Err(rollback) =
            shortcuts::rollback_collection(&storage(layout), plan.previous_records(), &moved)
        {
            failures.push(rollback);
        }
        if let Err(rollback) = save_bindings(layout, &old_bindings) {
            failures.push(rollback);
        }
        return Err(if failures.is_empty() {
            format!("The batch could not be saved; newly moved shortcuts were returned to the Desktop. {error}")
        } else {
            format!(
                "{error}. Recovery records were retained: {}",
                failures.join("; ")
            )
        });
    }
    *config = candidate;
    Ok(plan.entries().len())
}

/// Restore all active owned shortcuts. Retaining bindings is for normal Quit or
/// crash recovery; forgetting is the explicit Restore/uninstall preference.
/// An unreadable managed list never prevents a valid recovery ledger restoring
/// files, but is preserved and reported in warnings.
pub fn restore_all(config: &mut Config, layout: &Path, forget: bool) -> Result<Report, String> {
    restore_all_with_callback(config, layout, forget, |_| {})
}

/// Notify the caller as each file becomes available, before slower metadata
/// commits. The recovery ledger remains authoritative if a later save fails.
pub fn restore_all_with_callback(
    config: &mut Config,
    layout: &Path,
    forget: bool,
    mut on_restored: impl FnMut(&CollectedShortcut),
) -> Result<Report, String> {
    let entries = shortcuts::load_ledger(&storage(layout))?;
    let mut report = Report::default();
    let legacy_registry_missing = !registry(layout).exists();
    let mut managed = match load_bindings(layout) {
        Ok(v) => Some(v),
        Err(e) => {
            report.warnings.push(e);
            None
        }
    };
    if !forget {
        if let Some(bindings) = managed.as_mut() {
            if legacy_registry_missing {
                for entry in &entries {
                    if Path::new(&entry.stored).is_file()
                        && (references(config, &entry.stored)
                            || (!Path::new(&entry.original).exists()
                                && references(config, &entry.original)))
                    {
                        replace_binding(bindings, entry.clone());
                    }
                }
            }
            // Preserve ownership before returning files, including legacy
            // collections made before the managed registry existed. Once the
            // registry exists, unmatched active entries are interrupted drops
            // or explicitly forgotten ownership and must not be adopted again.
            if let Err(error) = save_bindings(layout, bindings) {
                report.warnings.push(error);
                managed = None;
            }
        }
    }
    let mut candidate = config.clone();
    let mut remaining = Vec::new();
    for entry in entries {
        let returning_file = Path::new(&entry.stored).is_file();
        match shortcuts::restore_shortcut(&entry) {
            Ok(()) => {
                if returning_file {
                    on_restored(&entry);
                }
                remap_targets(&mut candidate, &entry.stored, &entry.original);
                report.restored += 1;
            }
            Err(error) => {
                let conflict = Path::new(&entry.original).exists();
                report.conflicts += usize::from(conflict);
                report.failures += usize::from(!conflict);
                report.warnings.push(error);
                remaining.push(entry);
            }
        }
    }
    // If this save fails, keep the entire active ledger. A subsequent recovery
    // replays already-restored entries and repairs their durable target paths.
    *config = candidate;
    config.save(layout)?;
    if forget && managed.is_some() {
        save_bindings(layout, &[])?;
    }
    report.remaining = remaining.len();
    shortcuts::save_ledger(&storage(layout), &remaining)?;
    Ok(report)
}

/// Return shortcuts after a durable layout edit removes their final reference.
/// This never collects visible Desktop files or edits the layout. Aliases using
/// either path keep ownership; conflicts retain recovery records for later.
pub fn restore_orphans(config: &Config, layout: &Path) -> Result<Report, String> {
    restore_orphans_with_callback(config, layout, |_| {})
}

pub fn restore_orphans_with_callback(
    config: &Config,
    layout: &Path,
    mut on_restored: impl FnMut(&CollectedShortcut),
) -> Result<Report, String> {
    config.validate()?;
    let bindings = load_bindings(layout)?;
    let active = shortcuts::load_ledger(&storage(layout))?;
    let mut entries = active.clone();
    for entry in &bindings {
        if references(config, &entry.original) || references(config, &entry.stored) {
            continue;
        }
        if let Some(existing) = entries
            .iter()
            .find(|existing| key(Path::new(&existing.original)) == key(Path::new(&entry.original)))
        {
            if key(Path::new(&existing.stored)) != key(Path::new(&entry.stored))
                && Path::new(&entry.stored).exists()
            {
                return Err(
                    "Shortcut ownership and recovery records disagree; both were preserved.".into(),
                );
            }
        } else {
            entries.push(entry.clone());
        }
    }
    if entries != active {
        // A managed record also protects a shortcut if an old active ledger
        // was absent. Make its recovery record durable before returning it.
        shortcuts::save_ledger(&storage(layout), &entries)?;
    }
    let mut report = Report::default();
    let mut remaining = Vec::new();
    let mut retired = HashSet::new();
    for entry in entries {
        if references(config, &entry.original) || references(config, &entry.stored) {
            remaining.push(entry);
            continue;
        }
        let returning_file = Path::new(&entry.stored).is_file();
        match shortcuts::restore_shortcut(&entry) {
            Ok(()) => {
                if returning_file {
                    on_restored(&entry);
                }
                retired.insert(key(Path::new(&entry.original)));
                report.restored += 1;
            }
            Err(error) => {
                let conflict = Path::new(&entry.original).exists();
                report.conflicts += usize::from(conflict);
                report.failures += usize::from(!conflict);
                report.warnings.push(error);
                remaining.push(entry);
            }
        }
    }
    report.remaining = remaining.len();
    if !retired.is_empty() {
        let bindings = bindings
            .into_iter()
            .filter(|entry| !retired.contains(&key(Path::new(&entry.original))))
            .collect::<Vec<_>>();
        // If either save fails, the pre-restoration active ledger still
        // describes every returned file and can be safely replayed on exit.
        save_bindings(layout, &bindings)?;
        shortcuts::save_ledger(&storage(layout), &remaining)?;
    }
    Ok(report)
}

/// Startup reacquires only explicitly managed entries still referenced by the
/// layout. Call after recovery and after the watchdog is ready. Orphan stored
/// entries are returned, never silently abandoned or reacquired without a use.
pub fn reconcile_startup(
    config: &mut Config,
    layout: &Path,
    desktop: &Path,
) -> Result<Report, String> {
    reconcile_startup_with_positions(config, layout, desktop, &[])
}

/// Fresh view coordinates apply only when recollecting a visible original.
/// An already-collected shortcut keeps its existing durable position.
pub fn reconcile_startup_with_positions(
    config: &mut Config,
    layout: &Path,
    desktop: &Path,
    positions: &[(String, DesktopPosition)],
) -> Result<Report, String> {
    config.validate()?;
    if positions.iter().any(|(_, position)| !position.is_valid()) {
        return Err("Invalid current Desktop icon position; no shortcuts were moved.".into());
    }
    let bindings = load_bindings(layout)?;
    let _ = shortcuts::load_ledger(&storage(layout))?;
    let mut next = Vec::new();
    let mut report = Report::default();
    let mut candidate = config.clone();
    let mut moved = Vec::new();
    let mut retired = Vec::new();
    for entry in bindings {
        let used = references(&candidate, &entry.original) || references(&candidate, &entry.stored);
        if !used {
            if Path::new(&entry.stored).is_file() {
                match shortcuts::restore_shortcut(&entry) {
                    Ok(()) => {
                        retired.push(entry);
                        report.restored += 1;
                    }
                    Err(error) => {
                        let conflict = Path::new(&entry.original).exists();
                        report.conflicts += usize::from(conflict);
                        report.failures += usize::from(!conflict);
                        report.warnings.push(error);
                        next.push(entry);
                    }
                }
            }
            // Original already visible or both paths gone: do not collect an
            // unreferenced Desktop item just because an interrupted drop left a record.
            continue;
        }
        if !eligible(Path::new(&entry.original), desktop)? {
            report.warnings.push(
                "A managed shortcut belongs to a different Desktop folder and was left untouched."
                    .into(),
            );
            next.push(entry);
            continue;
        }
        if Path::new(&entry.stored).is_file() {
            let mut active = shortcuts::load_ledger(&storage(layout))?;
            if !active
                .iter()
                .any(|e| key(Path::new(&e.original)) == key(Path::new(&entry.original)))
            {
                active.push(entry.clone());
                shortcuts::save_ledger(&storage(layout), &active)?;
            }
            if !Path::new(&entry.original).exists() {
                remap_targets(&mut candidate, &entry.original, &entry.stored);
            } else {
                report.conflicts += 1;
                report.warnings.push("An original Desktop name is occupied; its stored shortcut was kept without overwriting either file.".into());
            }
            next.push(entry);
            continue;
        }
        if Path::new(&entry.original).is_file() {
            let position = positions
                .iter()
                .find(|(path, _)| key(Path::new(path)) == key(Path::new(&entry.original)))
                .map(|(_, position)| *position)
                .or(entry.position);
            match shortcuts::collect_shortcut_with_position(
                Path::new(&entry.original),
                desktop,
                &storage(layout),
                position,
            ) {
                Ok(Some(collected)) => {
                    remap_targets(&mut candidate, &entry.stored, &collected.stored);
                    remap_targets(&mut candidate, &entry.original, &collected.stored);
                    moved.push(collected.clone());
                    next.push(collected);
                    report.collected += 1;
                }
                Ok(None) => {
                    next.push(entry);
                    report.failures += 1;
                    report.warnings.push(
                        "A managed shortcut could not be reacquired and remains on the Desktop."
                            .into(),
                    );
                }
                Err(error) => {
                    next.push(entry);
                    report.failures += 1;
                    report.warnings.push(error);
                }
            }
        } else {
            report.failures += 1;
            report.warnings.push(format!(
                "Managed shortcut is unavailable: {}",
                entry.original
            ));
            next.push(entry);
        }
    }
    if let Err(error) = candidate.save(layout) {
        for entry in &moved {
            let _ = shortcuts::restore_shortcut(entry);
        }
        return Err(format!("Could not save the recovered layout; newly collected shortcuts were returned where possible. {error}"));
    }
    *config = candidate;
    save_bindings(layout, &next)?;
    for entry in retired {
        remove_active(layout, &entry)?;
    }
    report.remaining = shortcuts::load_ledger(&storage(layout))?.len();
    Ok(report)
}

/// Outgoing drop for a collected file. Restores its original (never overwrites),
/// forgets ownership globally, remaps every other alias to the visible Desktop
/// file, and removes only the dragged entry. False lets the caller export an
/// ordinary application/reference as a new Desktop shortcut instead.
pub fn release_to_desktop(
    config: &mut Config,
    layout: &Path,
    item_id: &str,
    desktop: &Path,
) -> Result<bool, String> {
    let Some(item) = config.find_item(item_id).cloned() else {
        return Err("The dragged item no longer exists.".into());
    };
    if item.is_folder() {
        return Ok(false);
    }
    let mut bindings = load_bindings(layout)?;
    let active = shortcuts::load_ledger(&storage(layout))?;
    let entry = bindings
        .iter()
        .chain(active.iter())
        .find(|e| {
            key(Path::new(&item.target)) == key(Path::new(&e.stored))
                || key(Path::new(&item.target)) == key(Path::new(&e.original))
        })
        .cloned();
    let Some(entry) = entry else {
        return Ok(false);
    };
    if !eligible(Path::new(&entry.original), desktop)? {
        return Err("This shortcut's original belongs to a different Desktop location.".into());
    }
    let mut candidate = config.clone();
    remap_targets(&mut candidate, &entry.stored, &entry.original);
    candidate.remove_item(item_id);
    candidate.validate()?;
    shortcuts::restore_shortcut(&entry)?;
    bindings.retain(|e| key(Path::new(&e.original)) != key(Path::new(&entry.original)));
    // Forget after returning the file, before the model commit. If interrupted
    // here, recovery sees a missing stored file and never adopts this as a new
    // active binding. A failed save leaves a visible original and replay ledger.
    *config = candidate;
    save_bindings(layout, &bindings)?;
    config.save(layout)?;
    remove_active(layout, &entry)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Group;
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let p = std::env::temp_dir().join(model::new_id("desktop-transfer-test"));
            fs::create_dir_all(p.join("Desktop")).unwrap();
            fs::create_dir_all(p.join("Profile")).unwrap();
            Self(p)
        }
        fn desktop(&self) -> PathBuf {
            self.0.join("Desktop")
        }
        fn layout(&self) -> PathBuf {
            self.0.join("Profile/layout.json")
        }
        fn file(&self, name: &str) -> PathBuf {
            let p = self.desktop().join(name);
            fs::write(&p, b"shortcut bytes").unwrap();
            p
        }
        fn config(&self) -> Config {
            let c = Config {
                version: 1,
                groups: vec![Group::new("Apps")],
                settings: Default::default(),
            };
            c.save(&self.layout()).unwrap();
            c
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn batch_commits_once_preserves_aliases_and_retains_positions_across_restart() {
        let s = Sandbox::new();
        let first = s.file("First.lnk");
        let second = s.file("Second.url");
        let mut config = s.config();
        let alias = Item::shortcut("Existing alias", first.to_string_lossy());
        let alias_id = alias.id.clone();
        config.groups[0].items.push(alias);
        config.save(&s.layout()).unwrap();
        let before = config.clone();
        let positions = [
            DesktopPosition { x: -116, y: 92 },
            DesktopPosition { x: 348, y: 184 },
        ];
        let paths = [
            (first.clone(), Some(positions[0])),
            (second.clone(), Some(positions[1])),
            (first.clone(), None),
        ];
        let mut moved = Vec::new();
        let accepted = collect_many_into_with_positions(
            &mut config,
            &s.layout(),
            &s.desktop(),
            &paths,
            0,
            None,
            |entry| {
                // Every planned file and its coordinates are durable before the
                // first notification, while layout/ownership are still unchanged.
                assert_eq!(
                    shortcuts::load_ledger(&storage(&s.layout())).unwrap().len(),
                    2
                );
                assert_eq!(Config::load(&s.layout()).unwrap(), before);
                assert!(load_bindings(&s.layout()).unwrap().is_empty());
                assert!(!Path::new(&entry.original).exists());
                moved.push(entry.clone());
            },
        )
        .unwrap();
        assert_eq!(accepted, 2);
        assert_eq!(moved.len(), 2);
        assert_eq!(config.groups[0].items.len(), 2);
        assert_eq!(config.groups[0].items[0].id, alias_id);
        assert_eq!(load_bindings(&s.layout()).unwrap(), moved);
        assert_eq!(
            moved
                .iter()
                .map(|entry| entry.position.unwrap())
                .collect::<Vec<_>>(),
            positions
        );
        restore_all(&mut config, &s.layout(), false).unwrap();
        assert!(first.is_file() && second.is_file());
        let report = reconcile_startup(&mut config, &s.layout(), &s.desktop()).unwrap();
        assert_eq!(report.collected, 2);
        let saved = load_bindings(&s.layout()).unwrap();
        assert_eq!(
            saved
                .iter()
                .map(|entry| entry.position.unwrap())
                .collect::<Vec<_>>(),
            positions
        );
        let unchanged = config.clone();
        let mut notifications = 0;
        collect_many_into_with_positions(
            &mut config,
            &s.layout(),
            &s.desktop(),
            &paths,
            0,
            None,
            |_| notifications += 1,
        )
        .unwrap();
        assert_eq!(notifications, 0);
        assert_eq!(config, unchanged);
        assert_eq!(load_bindings(&s.layout()).unwrap(), saved);
    }

    #[test]
    fn invalid_batch_candidate_is_rejected_before_wal_ownership_or_file_moves() {
        let s = Sandbox::new();
        let first = s.file("First.lnk");
        let invalid = s.file(&format!("{}.lnk", "x".repeat(129)));
        let mut config = s.config();
        let before = config.clone();
        let layout = fs::read(s.layout()).unwrap();
        let mut notified = 0;
        let failed = collect_many_into_with_positions(
            &mut config,
            &s.layout(),
            &s.desktop(),
            &[(first.clone(), None), (invalid.clone(), None)],
            0,
            None,
            |_| notified += 1,
        );
        assert!(failed.unwrap_err().contains("Names must contain"));
        assert_eq!(notified, 0);
        assert_eq!(config, before);
        assert_eq!(fs::read(s.layout()).unwrap(), layout);
        assert!(first.is_file() && invalid.is_file());
        assert!(!registry(&s.layout()).exists());
        assert!(!storage(&s.layout()).join("collection.json").exists());
    }

    #[test]
    fn batch_move_failure_keeps_old_ownership_layout_and_existing_files() {
        let s = Sandbox::new();
        let old = s.file("Old.lnk");
        let first = s.file("First.lnk");
        let second = s.file("Second.lnk");
        let held = s.0.join("Second-held.lnk");
        let mut config = s.config();
        collect_into(&mut config, &s.layout(), &s.desktop(), &old, 0, None).unwrap();
        let before = config.clone();
        let previous = load_bindings(&s.layout()).unwrap();
        let mut notified = 0;
        let failed = collect_many_into_with_positions(
            &mut config,
            &s.layout(),
            &s.desktop(),
            &[(first.clone(), None), (second.clone(), None)],
            0,
            None,
            |_| {
                notified += 1;
                fs::rename(&second, &held).unwrap();
            },
        );
        assert!(failed.is_err());
        assert_eq!(notified, 1);
        assert_eq!(config, before);
        assert_eq!(Config::load(&s.layout()).unwrap(), before);
        assert_eq!(load_bindings(&s.layout()).unwrap(), previous);
        assert_eq!(
            shortcuts::load_ledger(&storage(&s.layout())).unwrap(),
            previous
        );
        assert!(Path::new(&previous[0].stored).is_file());
        assert!(!old.exists());
        assert_eq!(fs::read(&first).unwrap(), b"shortcut bytes");
        assert_eq!(fs::read(&held).unwrap(), b"shortcut bytes");
    }

    #[test]
    fn batch_commit_failure_returns_new_files_and_preserves_old_ownership() {
        let s = Sandbox::new();
        let old = s.file("Old.lnk");
        let first = s.file("First.lnk");
        let second = s.file("Second.url");
        let mut config = s.config();
        collect_into(&mut config, &s.layout(), &s.desktop(), &old, 0, None).unwrap();
        let before = config.clone();
        let previous = load_bindings(&s.layout()).unwrap();
        fs::write(s.layout(), b"malformed layout preserved on save failure").unwrap();
        let mut notified = Vec::new();
        let failed = collect_many_into_with_positions(
            &mut config,
            &s.layout(),
            &s.desktop(),
            &[
                (first.clone(), Some(DesktopPosition { x: 0, y: 92 })),
                (second.clone(), None),
            ],
            0,
            None,
            |entry| notified.push(entry.clone()),
        );
        assert!(failed.is_err());
        assert_eq!(notified.len(), 2);
        assert_eq!(config, before);
        assert_eq!(
            fs::read(s.layout()).unwrap(),
            b"malformed layout preserved on save failure"
        );
        assert_eq!(load_bindings(&s.layout()).unwrap(), previous);
        assert_eq!(
            shortcuts::load_ledger(&storage(&s.layout())).unwrap(),
            previous
        );
        assert!(Path::new(&previous[0].stored).is_file());
        assert!(!old.exists());
        assert_eq!(fs::read(&first).unwrap(), b"shortcut bytes");
        assert_eq!(fs::read(&second).unwrap(), b"shortcut bytes");
        assert!(notified
            .iter()
            .all(|entry| !Path::new(&entry.stored).exists()));
    }

    #[test]
    fn failed_batch_ledger_cleanup_retains_replayable_wal_after_returning_files() {
        let s = Sandbox::new();
        let first = s.file("First.lnk");
        let second = s.file("Second.lnk");
        let mut config = s.config();
        let before = config.clone();
        fs::write(s.layout(), b"invalid layout blocks the final commit").unwrap();
        let ledger = storage(&s.layout()).join("collection.json");
        let mut writable = None;
        let mut notified = 0;
        let failed = collect_many_into_with_positions(
            &mut config,
            &s.layout(),
            &s.desktop(),
            &[(first.clone(), None), (second.clone(), None)],
            0,
            None,
            |_| {
                notified += 1;
                if notified == 2 {
                    let permissions = fs::metadata(&ledger).unwrap().permissions();
                    let mut readonly = permissions.clone();
                    readonly.set_readonly(true);
                    fs::set_permissions(&ledger, readonly).unwrap();
                    writable = Some(permissions);
                }
            },
        );
        fs::set_permissions(&ledger, writable.unwrap()).unwrap();
        assert!(failed
            .unwrap_err()
            .contains("Recovery records were retained"));
        assert_eq!(config, before);
        assert!(first.is_file() && second.is_file());
        let reserved = shortcuts::load_ledger(&storage(&s.layout())).unwrap();
        assert_eq!(reserved.len(), 2);
        assert!(reserved
            .iter()
            .all(|entry| !Path::new(&entry.stored).exists()));
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
        // Once storage is writable again, replay consumes the still-valid WAL
        // without notifying a second physical return or acquiring ownership.
        fs::write(s.layout(), serde_json::to_vec_pretty(&before).unwrap()).unwrap();
        let mut returned = 0;
        restore_all_with_callback(&mut config, &s.layout(), false, |_| returned += 1).unwrap();
        assert_eq!(returned, 0);
        assert!(shortcuts::load_ledger(&storage(&s.layout()))
            .unwrap()
            .is_empty());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
    }

    #[test]
    fn interrupted_batch_wal_does_not_acquire_an_existing_ordinary_alias() {
        let s = Sandbox::new();
        let first = s.file("First.lnk");
        let second = s.file("Second.lnk");
        let mut config = s.config();
        let alias = Item::shortcut("Ordinary alias", first.to_string_lossy());
        let id = alias.id.clone();
        config.groups[0].items.push(alias);
        config.save(&s.layout()).unwrap();
        let before = config.clone();
        let mut observed = None;
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = collect_many_into_with_positions(
                &mut config,
                &s.layout(),
                &s.desktop(),
                &[
                    (first.clone(), Some(DesktopPosition { x: 0, y: 276 })),
                    (second.clone(), None),
                ],
                0,
                None,
                |entry| {
                    observed = Some((
                        entry.clone(),
                        shortcuts::load_ledger(&storage(&s.layout())).unwrap(),
                        registry(&s.layout()).is_file(),
                        load_bindings(&s.layout()).unwrap(),
                    ));
                    panic!("simulated interruption after the first physical move");
                },
            );
        }));
        assert!(interrupted.is_err());
        let (moved, reserved, marker, ownership) = observed.unwrap();
        assert_eq!(reserved.len(), 2);
        assert_eq!(moved.position, Some(DesktopPosition { x: 0, y: 276 }));
        assert!(marker && ownership.is_empty());
        assert_eq!(config, before);
        let mut returned = Vec::new();
        restore_all_with_callback(&mut config, &s.layout(), false, |entry| {
            returned.push(entry.clone())
        })
        .unwrap();
        assert_eq!(returned, [moved]);
        assert!(first.is_file() && second.is_file());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
        assert_eq!(
            reconcile_startup(&mut config, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            0
        );
        assert_eq!(config.groups[0].items[0].id, id);
        assert_eq!(config, before);
    }

    #[test]
    fn interrupted_batch_preserves_genuine_legacy_ownership_without_adopting_new_files() {
        let s = Sandbox::new();
        let old = s.file("Legacy.lnk");
        let first = s.file("First.lnk");
        let second = s.file("Second.lnk");
        let mut config = s.config();
        let old_entry = shortcuts::collect_shortcut(&old, &s.desktop(), &storage(&s.layout()))
            .unwrap()
            .unwrap();
        let alias = Item::shortcut("Legacy", shell(&old_entry.stored));
        let id = alias.id.clone();
        config.groups[0].items.push(alias);
        config.save(&s.layout()).unwrap();
        assert!(!registry(&s.layout()).exists());
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = collect_many_into_with_positions(
                &mut config,
                &s.layout(),
                &s.desktop(),
                &[(first.clone(), None), (second.clone(), None)],
                0,
                None,
                |_| panic!("simulated interruption"),
            );
        }));
        assert!(interrupted.is_err());
        assert_eq!(load_bindings(&s.layout()).unwrap(), [old_entry]);
        restore_all(&mut config, &s.layout(), false).unwrap();
        assert_eq!(
            reconcile_startup(&mut config, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            1
        );
        assert!(!old.exists());
        assert!(first.is_file() && second.is_file());
        assert_eq!(config.groups[0].items[0].id, id);
        assert_eq!(config.groups[0].items.len(), 1);
        assert_eq!(load_bindings(&s.layout()).unwrap().len(), 1);
    }

    #[test]
    fn collection_quit_reacquisition_and_release_preserve_bytes_and_layout() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        let position = DesktopPosition { x: 116, y: 276 };
        assert!(collect_into_with_position(
            &mut c,
            &s.layout(),
            &s.desktop(),
            &original,
            0,
            None,
            Some(position)
        )
        .unwrap());
        assert!(!original.exists());
        let id = c.groups[0].items[0].id.clone();
        let stored = c.groups[0].items[0].target.clone();
        assert_eq!(fs::read(&stored).unwrap(), b"shortcut bytes");
        let report = restore_all(&mut c, &s.layout(), false).unwrap();
        assert_eq!(report.restored, 1);
        assert!(original.exists());
        assert_eq!(load_bindings(&s.layout()).unwrap().len(), 1);
        assert_eq!(
            load_bindings(&s.layout()).unwrap()[0].position,
            Some(position)
        );
        c = Config::load(&s.layout()).unwrap();
        assert_eq!(
            reconcile_startup(&mut c, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            1
        );
        assert!(!original.exists());
        assert_eq!(
            shortcuts::load_ledger(&storage(&s.layout())).unwrap()[0].position,
            Some(position)
        );
        assert_eq!(
            load_bindings(&s.layout()).unwrap()[0].position,
            Some(position)
        );
        assert_eq!(c.groups[0].items[0].id, id);
        assert!(release_to_desktop(&mut c, &s.layout(), &id, &s.desktop()).unwrap());
        assert_eq!(fs::read(&original).unwrap(), b"shortcut bytes");
        assert!(c.groups[0].items.is_empty());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
        assert!(shortcuts::load_ledger(&storage(&s.layout()))
            .unwrap()
            .is_empty());
        assert_eq!(
            reconcile_startup(&mut c, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            0
        );
        assert!(original.exists());
    }
    #[test]
    fn restored_callback_precedes_layout_commit_and_failed_commit_retains_recovery() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut config = s.config();
        let position = DesktopPosition { x: 120, y: 276 };
        collect_into_with_position(
            &mut config,
            &s.layout(),
            &s.desktop(),
            &original,
            0,
            None,
            Some(position),
        )
        .unwrap();
        let stored_target = config.groups[0].items[0].target.clone();
        let layout = s.layout();
        let writable = fs::metadata(&layout).unwrap().permissions();
        let mut readonly = writable.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&layout, readonly).unwrap();
        let mut observed = None;
        let failed = restore_all_with_callback(&mut config, &layout, false, |entry| {
            observed = Some((
                Path::new(&entry.original).is_file(),
                Path::new(&entry.stored).exists(),
                Config::load(&layout).unwrap().groups[0].items[0]
                    .target
                    .clone(),
                entry.position,
            ));
        });
        fs::set_permissions(&layout, writable).unwrap();
        assert!(failed.is_err());
        assert_eq!(observed, Some((true, false, stored_target, Some(position))));
        assert_eq!(shortcuts::load_ledger(&storage(&layout)).unwrap().len(), 1);
        assert_eq!(fs::read(&original).unwrap(), b"shortcut bytes");
        restore_all(&mut config, &layout, false).unwrap();
        assert!(shortcuts::load_ledger(&storage(&layout))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn orphan_callback_precedes_ownership_cleanup_even_when_the_cleanup_fails() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut config = s.config();
        collect_into(&mut config, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        config.groups[0].items.clear();
        config.save(&s.layout()).unwrap();
        let list = registry(&s.layout());
        let writable = fs::metadata(&list).unwrap().permissions();
        let mut readonly = writable.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&list, readonly).unwrap();
        let mut observed = None;
        let failed = restore_orphans_with_callback(&config, &s.layout(), |entry| {
            observed = Some((
                Path::new(&entry.original).is_file(),
                shortcuts::load_ledger(&storage(&s.layout())).unwrap().len(),
                load_bindings(&s.layout()).unwrap().len(),
            ));
        });
        fs::set_permissions(&list, writable).unwrap();
        assert!(failed.is_err());
        assert_eq!(observed, Some((true, 1, 1)));
        assert_eq!(
            shortcuts::load_ledger(&storage(&s.layout())).unwrap().len(),
            1
        );
        restore_orphans(&config, &s.layout()).unwrap();
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
    }

    #[test]
    fn idempotent_restore_does_not_request_repositioning_for_a_visible_replacement() {
        for orphan in [false, true] {
            let s = Sandbox::new();
            let original = s.file("Example.lnk");
            let mut config = s.config();
            collect_into_with_position(
                &mut config,
                &s.layout(),
                &s.desktop(),
                &original,
                0,
                None,
                Some(DesktopPosition { x: 0, y: 184 }),
            )
            .unwrap();
            let record = shortcuts::load_ledger(&storage(&s.layout()))
                .unwrap()
                .remove(0);
            // Simulate a returned file with stale recovery metadata, then an
            // external replacement. Replaying metadata must not move its icon.
            shortcuts::restore_shortcut(&record).unwrap();
            fs::write(&original, b"replacement shortcut").unwrap();
            let mut callbacks = 0;
            let report = if orphan {
                config.groups[0].items.clear();
                config.save(&s.layout()).unwrap();
                restore_orphans_with_callback(&config, &s.layout(), |_| callbacks += 1)
            } else {
                restore_all_with_callback(&mut config, &s.layout(), false, |_| callbacks += 1)
            }
            .unwrap();
            assert_eq!(report.restored, 1);
            assert_eq!(callbacks, 0);
            assert_eq!(fs::read(&original).unwrap(), b"replacement shortcut");
        }
    }

    #[test]
    fn fresh_startup_positions_backfill_legacy_and_override_only_visible_originals() {
        for initial_position in [None, Some(DesktopPosition { x: 0, y: 92 })] {
            let s = Sandbox::new();
            let original = s.file("Example.lnk");
            let mut config = s.config();
            collect_into_with_position(
                &mut config,
                &s.layout(),
                &s.desktop(),
                &original,
                0,
                None,
                initial_position,
            )
            .unwrap();
            restore_all(&mut config, &s.layout(), false).unwrap();
            let fresh = DesktopPosition { x: 696, y: 276 };
            let positions = [(original.to_string_lossy().into_owned(), fresh)];
            assert_eq!(
                reconcile_startup_with_positions(
                    &mut config,
                    &s.layout(),
                    &s.desktop(),
                    &positions
                )
                .unwrap()
                .collected,
                1
            );
            assert_eq!(load_bindings(&s.layout()).unwrap()[0].position, Some(fresh));
            assert_eq!(
                shortcuts::load_ledger(&storage(&s.layout())).unwrap()[0].position,
                Some(fresh)
            );
            let unrelated = [(
                original.to_string_lossy().into_owned(),
                DesktopPosition { x: 928, y: 460 },
            )];
            assert_eq!(
                reconcile_startup_with_positions(
                    &mut config,
                    &s.layout(),
                    &s.desktop(),
                    &unrelated
                )
                .unwrap()
                .collected,
                0
            );
            assert_eq!(load_bindings(&s.layout()).unwrap()[0].position, Some(fresh));
            restore_all(&mut config, &s.layout(), false).unwrap();
            reconcile_startup(&mut config, &s.layout(), &s.desktop()).unwrap();
            assert_eq!(load_bindings(&s.layout()).unwrap()[0].position, Some(fresh));
        }
    }

    #[test]
    fn unchanged_managed_saves_preserve_the_previous_distinct_backup() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        save_bindings(&s.layout(), &[]).unwrap();
        let empty = fs::read(registry(&s.layout())).unwrap();
        collect_into_with_position(
            &mut c,
            &s.layout(),
            &s.desktop(),
            &original,
            0,
            None,
            Some(DesktopPosition { x: 0, y: 184 }),
        )
        .unwrap();
        let records = load_bindings(&s.layout()).unwrap();
        let path = registry(&s.layout());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        save_bindings(&s.layout(), &records).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read(path.with_extension("json.bak")).unwrap(), empty);
        restore_all(&mut c, &s.layout(), false).unwrap();
        assert_eq!(load_bindings(&s.layout()).unwrap(), records);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read(path.with_extension("json.bak")).unwrap(), empty);
        let mut invalid = records;
        invalid[0].position = Some(DesktopPosition { x: 1_000_001, y: 0 });
        assert!(save_bindings(&s.layout(), &invalid).is_err());
    }

    #[test]
    fn an_occupied_desktop_name_is_never_overwritten_or_untracked() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        let id = c.groups[0].items[0].id.clone();
        let stored = c.groups[0].items[0].target.clone();
        fs::write(&original, b"new unrelated shortcut").unwrap();
        assert!(release_to_desktop(&mut c, &s.layout(), &id, &s.desktop()).is_err());
        let report = restore_all(&mut c, &s.layout(), false).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(fs::read(original).unwrap(), b"new unrelated shortcut");
        assert_eq!(fs::read(stored).unwrap(), b"shortcut bytes");
        assert_eq!(load_bindings(&s.layout()).unwrap().len(), 1);
        assert_eq!(
            shortcuts::load_ledger(&storage(&s.layout())).unwrap().len(),
            1
        );
    }
    #[test]
    fn nested_drop_remaps_aliases_and_release_removes_only_dragged_item() {
        let s = Sandbox::new();
        let original = s.file("Example.url");
        let mut c = s.config();
        let alias = Item::shortcut("Alias", original.to_string_lossy());
        let alias_id = alias.id.clone();
        let folder = Item::folder("Tools");
        let folder_id = folder.id.clone();
        c.groups[0].items.extend([alias, folder]);
        c.save(&s.layout()).unwrap();
        collect_into(
            &mut c,
            &s.layout(),
            &s.desktop(),
            &original,
            0,
            Some(&folder_id),
        )
        .unwrap();
        let child = c.find_item(&folder_id).unwrap().children[0].clone();
        assert_eq!(c.find_item(&alias_id).unwrap().target, child.target);
        assert!(release_to_desktop(&mut c, &s.layout(), &child.id, &s.desktop()).unwrap());
        assert!(c.find_item(&child.id).is_none());
        assert!(c.find_item(&alias_id).is_some());
        assert_eq!(
            key(Path::new(&c.find_item(&alias_id).unwrap().target)),
            key(&original)
        );
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
    }
    #[test]
    fn failed_layout_save_returns_newly_moved_file_and_preserves_previous_config() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        let before = c.clone();
        fs::write(s.layout(), b"malformed existing layout").unwrap();
        assert!(collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).is_err());
        assert_eq!(c, before);
        assert_eq!(fs::read(original).unwrap(), b"shortcut bytes");
        assert!(shortcuts::load_ledger(&storage(&s.layout()))
            .unwrap()
            .is_empty());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
        assert_eq!(fs::read(s.layout()).unwrap(), b"malformed existing layout");
    }
    #[test]
    fn orphan_binding_is_returned_instead_of_hiding_a_shortcut_without_a_group() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        c.groups[0].items.clear();
        c.save(&s.layout()).unwrap();
        let r = reconcile_startup(&mut c, &s.layout(), &s.desktop()).unwrap();
        assert_eq!(r.restored, 1);
        assert!(original.exists());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
    }
    #[test]
    fn corrupt_managed_file_blocks_collection_but_does_not_block_active_recovery() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        fs::write(registry(&s.layout()), b"bad managed file").unwrap();
        let r = restore_all(&mut c, &s.layout(), false).unwrap();
        assert_eq!(r.restored, 1);
        assert!(!r.warnings.is_empty());
        assert!(original.exists());
        assert!(reconcile_startup(&mut c, &s.layout(), &s.desktop()).is_err());
        assert_eq!(
            fs::read(registry(&s.layout())).unwrap(),
            b"bad managed file"
        );
    }
    #[test]
    fn only_direct_user_desktop_shortcuts_move_and_explicit_restore_forgets_them() {
        let s = Sandbox::new();
        let mut c = s.config();
        let document = s.file("Document.txt");
        let executable = s.file("App.exe");
        let public = s.0.join("PublicDesktop");
        fs::create_dir(&public).unwrap();
        let shared = public.join("Shared.lnk");
        fs::write(&shared, b"shared").unwrap();
        for p in [&document, &executable, &shared] {
            assert!(!collect_into(&mut c, &s.layout(), &s.desktop(), p, 0, None).unwrap());
            assert!(p.exists());
        }
        let original = s.file("App.lnk");
        collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        restore_all(&mut c, &s.layout(), true).unwrap();
        assert!(original.exists());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
        assert_eq!(
            reconcile_startup(&mut c, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            0
        );
    }

    #[test]
    fn a_precommit_crash_returns_the_shortcut_without_reacquiring_an_orphan() {
        let s = Sandbox::new();
        let original = s.file("Interrupted.lnk");
        let mut c = s.config();
        save_bindings(&s.layout(), &[]).unwrap();
        let entry = shortcuts::collect_shortcut(&original, &s.desktop(), &storage(&s.layout()))
            .unwrap()
            .unwrap();
        // Crash after the file moves but before ownership/layout are saved.
        restore_all(&mut c, &s.layout(), false).unwrap();
        assert!(original.exists());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());

        shortcuts::collect_shortcut(&original, &s.desktop(), &storage(&s.layout()))
            .unwrap()
            .unwrap();
        save_bindings(&s.layout(), &[entry]).unwrap();
        // Crash after ownership is saved but before the new group entry.
        restore_all(&mut c, &s.layout(), false).unwrap();
        let report = reconcile_startup(&mut c, &s.layout(), &s.desktop()).unwrap();
        assert_eq!(report.collected, 0);
        assert!(original.exists());
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
    }

    #[test]
    fn legacy_collections_migrate_once_without_changing_item_ids() {
        let s = Sandbox::new();
        let original = s.file("Legacy.lnk");
        let mut c = s.config();
        let entry = shortcuts::collect_shortcut(&original, &s.desktop(), &storage(&s.layout()))
            .unwrap()
            .unwrap();
        let item = Item::shortcut("Legacy", shell(&entry.stored));
        let id = item.id.clone();
        c.groups[0].items.push(item);
        c.save(&s.layout()).unwrap();
        assert!(!registry(&s.layout()).exists());
        restore_all(&mut c, &s.layout(), false).unwrap();
        assert_eq!(load_bindings(&s.layout()).unwrap().len(), 1);
        assert_eq!(
            reconcile_startup(&mut c, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            1
        );
        assert_eq!(c.groups[0].items[0].id, id);
    }

    #[test]
    fn an_explicit_restore_stays_forgotten_after_a_name_conflict_is_resolved() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        fs::write(&original, b"occupied").unwrap();
        let report = restore_all(&mut c, &s.layout(), true).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.remaining, 1);
        assert_eq!(report.failures, 0);
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
        fs::remove_file(&original).unwrap();
        restore_all(&mut c, &s.layout(), false).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"shortcut bytes");
        assert_eq!(
            reconcile_startup(&mut c, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            0
        );
        assert!(original.exists());
    }

    #[test]
    fn orphan_cleanup_keeps_aliases_then_returns_the_last_removed_shortcut() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        let first = c.groups[0].items[0].id.clone();
        // An alias imported with the original path must also keep ownership.
        let alias = Item::shortcut("Alias", original.to_string_lossy());
        let alias_id = alias.id.clone();
        let mut nested = Item::folder("Nested");
        nested.children.push(alias);
        c.groups[0].items.push(nested);
        c.remove_item(&first);
        c.save(&s.layout()).unwrap();
        assert_eq!(restore_orphans(&c, &s.layout()).unwrap().restored, 0);
        assert!(!original.exists());
        assert_eq!(load_bindings(&s.layout()).unwrap().len(), 1);
        c.remove_item(&alias_id);
        c.save(&s.layout()).unwrap();
        assert_eq!(restore_orphans(&c, &s.layout()).unwrap().restored, 1);
        assert_eq!(fs::read(original).unwrap(), b"shortcut bytes");
        assert!(load_bindings(&s.layout()).unwrap().is_empty());
        assert!(shortcuts::load_ledger(&storage(&s.layout()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn orphan_cleanup_does_not_recapture_still_referenced_visible_shortcuts() {
        let s = Sandbox::new();
        let first = s.file("First.lnk");
        let second = s.file("Second.lnk");
        let mut c = s.config();
        collect_into(&mut c, &s.layout(), &s.desktop(), &first, 0, None).unwrap();
        collect_into(&mut c, &s.layout(), &s.desktop(), &second, 0, None).unwrap();
        restore_all(&mut c, &s.layout(), false).unwrap();
        c.groups[0].items.remove(0);
        c.save(&s.layout()).unwrap();
        restore_orphans(&c, &s.layout()).unwrap();
        assert!(first.exists());
        assert!(second.exists());
        let bindings = load_bindings(&s.layout()).unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(key(Path::new(&bindings[0].original)), key(&second));
        assert!(shortcuts::load_ledger(&storage(&s.layout()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn orphan_conflicts_and_failed_metadata_save_preserve_recovery_records() {
        let s = Sandbox::new();
        let original = s.file("Example.lnk");
        let mut c = s.config();
        collect_into(&mut c, &s.layout(), &s.desktop(), &original, 0, None).unwrap();
        c.groups[0].items.clear();
        c.save(&s.layout()).unwrap();
        fs::write(&original, b"occupied").unwrap();
        assert_eq!(restore_orphans(&c, &s.layout()).unwrap().conflicts, 1);
        assert_eq!(fs::read(&original).unwrap(), b"occupied");
        assert_eq!(load_bindings(&s.layout()).unwrap().len(), 1);
        fs::remove_file(&original).unwrap();
        let list = registry(&s.layout());
        let writable = fs::metadata(&list).unwrap().permissions();
        let mut readonly = writable.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&list, readonly).unwrap();
        let failed = restore_orphans(&c, &s.layout());
        fs::set_permissions(&list, writable).unwrap();
        assert!(failed.is_err());
        assert_eq!(fs::read(&original).unwrap(), b"shortcut bytes");
        assert_eq!(
            shortcuts::load_ledger(&storage(&s.layout())).unwrap().len(),
            1
        );
        assert_eq!(load_bindings(&s.layout()).unwrap().len(), 1);
        restore_orphans(&c, &s.layout()).unwrap();
        assert!(shortcuts::load_ledger(&storage(&s.layout()))
            .unwrap()
            .is_empty());
    }
}
