//! One-time, explicitly approved conversion from the shared Windows Desktop.
//!
//! Only the native move into a new private staging directory may need Windows
//! elevation. Every later step runs as the user and recovers forward onto their
//! personal Desktop. The per-profile lifetime lock must cover these calls.
use crate::{desktop_transfer, model::Config, shortcuts::DesktopPosition};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const MAX_JOURNAL: usize = 2 * 1024 * 1024;
const MAX_SHORTCUT: u64 = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 500;

#[derive(Clone, Debug)]
pub struct Conversion {
    pub original: PathBuf,
    pub personal: PathBuf,
    pub position: Option<DesktopPosition>,
}

#[derive(Default, Debug)]
pub struct Report {
    pub converted: Vec<Conversion>,
    pub not_moved: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub pending: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Identity {
    volume: u32,
    index: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Fingerprint {
    length: u64,
    hash: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    id: String,
    original: PathBuf,
    personal: PathBuf,
    fingerprint: Fingerprint,
    original_identity: Identity,
    position: Option<DesktopPosition>,
    native_started: bool,
    native_completed: bool,
    stage_identity: Option<Identity>,
    personal_identity: Option<Identity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    public: PathBuf,
    desktop: PathBuf,
    records: Vec<Record>,
}

pub struct Prepared {
    layout: PathBuf,
    public: PathBuf,
    desktop: PathBuf,
    ids: Vec<String>,
}

pub fn journal_path(layout: &Path) -> PathBuf {
    layout
        .parent()
        .unwrap_or(Path::new("."))
        .join("shared-conversions.json")
}

fn stage_root(layout: &Path) -> PathBuf {
    layout
        .parent()
        .unwrap_or(Path::new("."))
        .join("shared-staging")
}

fn stage_path(layout: &Path, entry: &Record) -> PathBuf {
    stage_root(layout).join(&entry.id)
}

fn key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(tail) = value.strip_prefix(r"\\?\unc\") {
        format!(r"\\{tail}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
}

fn shortcut(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("lnk" | "url")
    )
}

fn reparse(info: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    info.file_type().is_symlink() || info.file_attributes() & 0x400 != 0
}

fn normal_dir(path: &Path) -> Result<PathBuf, String> {
    let info = fs::symlink_metadata(path)
        .map_err(|e| format!("Could not inspect {}: {e}", path.display()))?;
    if !info.is_dir() || reparse(&info) {
        return Err(format!(
            "A conversion folder is not a regular directory: {}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}

fn identity(file: &File) -> Result<Identity, String> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[repr(C)]
    struct Information {
        words: [u32; 13],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: *mut c_void, information: *mut Information) -> i32;
    }
    let mut information = Information { words: [0; 13] };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(format!(
            "Could not identify a conversion file: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(Identity {
        volume: information.words[7],
        index: (information.words[11] as u64) << 32 | information.words[12] as u64,
    })
}

fn inspect(path: &Path) -> Result<(Fingerprint, Identity), String> {
    use std::os::windows::fs::OpenOptionsExt;
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("Could not inspect {}: {e}", path.display()))?;
    if !metadata.is_file() || reparse(&metadata) || metadata.len() > MAX_SHORTCUT {
        return Err(format!(
            "A conversion item is not a regular shortcut of supported size: {}",
            path.display()
        ));
    }
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|e| e.to_string())?;
    let id = identity(&file)?;
    let mut hash = 0xcbf29ce484222325u64;
    let mut length = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        length += n as u64;
        if length > MAX_SHORTCUT {
            return Err("The shortcut grew beyond the conversion size limit.".into());
        }
        for byte in &buffer[..n] {
            hash = (hash ^ *byte as u64).wrapping_mul(0x100000001b3);
        }
    }
    Ok((Fingerprint { length, hash }, id))
}

fn absent(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(format!("Could not inspect a conversion destination: {e}")),
    }
}

fn validate(journal: &Journal) -> Result<(), String> {
    if journal.version != 1
        || journal.records.len() > MAX_RECORDS
        || !journal.public.is_absolute()
        || !journal.desktop.is_absolute()
        || key(&journal.public) == key(&journal.desktop)
    {
        return Err("Invalid shared-shortcut conversion journal.".into());
    }
    let mut ids = HashSet::new();
    let mut originals = HashSet::new();
    let mut personal = HashSet::new();
    for entry in &journal.records {
        if entry.id.is_empty()
            || entry.id.len() > 128
            || !entry
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !ids.insert(entry.id.clone())
            || !originals.insert(key(&entry.original))
            || !personal.insert(key(&entry.personal))
            || entry.original.parent().map(key) != Some(key(&journal.public))
            || entry.personal.parent().map(key) != Some(key(&journal.desktop))
            || !shortcut(&entry.original)
            || !shortcut(&entry.personal)
            || entry
                .original
                .extension()
                .map(|s| s.to_string_lossy().to_lowercase())
                != entry
                    .personal
                    .extension()
                    .map(|s| s.to_string_lossy().to_lowercase())
            || entry.fingerprint.length > MAX_SHORTCUT
            || entry.position.is_some_and(|p| !p.is_valid())
            || (entry.native_completed && !entry.native_started)
            || (!entry.native_started
                && (entry.stage_identity.is_some() || entry.personal_identity.is_some()))
        {
            return Err("Invalid shared-shortcut conversion record; it was preserved.".into());
        }
    }
    Ok(())
}

fn read_journal(layout: &Path) -> Result<Option<Journal>, String> {
    let path = journal_path(layout);
    let info = match fs::symlink_metadata(&path) {
        Ok(info) => info,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !info.is_file() || reparse(&info) || info.len() > MAX_JOURNAL as u64 {
        return Err(
            "The shared-shortcut journal is not a bounded regular file; it was preserved.".into(),
        );
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_JOURNAL as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_JOURNAL {
        return Err("Shared-shortcut journal is too large.".into());
    }
    let journal: Journal = serde_json::from_slice(&bytes)
        .map_err(|e| format!("The shared-shortcut journal is unreadable and was preserved: {e}"))?;
    validate(&journal)?;
    Ok(Some(journal))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    // A failed create_new does not grant ownership of an existing file.
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let result = (|| -> std::io::Result<()> {
        output.write_all(bytes)?;
        output.sync_all()
    })();
    drop(output);
    if let Err(e) = result {
        let _ = fs::remove_file(path);
        return Err(e.to_string());
    }
    Ok(())
}

fn save(layout: &Path, journal: &Journal) -> Result<(), String> {
    validate(journal)?;
    // Never replace invalid or redirected existing recovery data.
    let _ = read_journal(layout)?;
    let path = journal_path(layout);
    let bytes = serde_json::to_vec_pretty(journal).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_JOURNAL {
        return Err("Shared-shortcut journal is too large.".into());
    }
    let temporary = path.with_extension(format!("{}.tmp", crate::model::new_id("shared-save")));
    write_new(&temporary, &bytes)?;
    if let Err(e) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not commit shared-shortcut recovery: {e}"));
    }
    Ok(())
}

fn fresh_personal(
    desktop: &Path,
    original: &Path,
    reserved: &HashSet<String>,
) -> Result<PathBuf, String> {
    let file = original
        .file_name()
        .ok_or("The shortcut has no filename.")?;
    for collision in 0..10_000 {
        let path = if collision == 0 {
            desktop.join(file)
        } else {
            desktop.join(format!(
                "{} ({}).{}",
                original.file_stem().unwrap_or(file).to_string_lossy(),
                collision + 1,
                original.extension().unwrap_or_default().to_string_lossy()
            ))
        };
        if !reserved.contains(&key(&path)) && absent(&path)? {
            return Ok(path);
        }
    }
    Err("Too many shortcuts with the same name on your Desktop.".into())
}

/// Must follow the user's explicit choice for these exact shared shortcuts.
/// This creates only empty private staging directories and a durable journal.
pub fn prepare(
    layout: &Path,
    public: &Path,
    desktop: &Path,
    selected: &[(PathBuf, Option<DesktopPosition>)],
) -> Result<Prepared, String> {
    Config::load(layout)?;
    if !layout.is_file() {
        return Err("Save the folder layout before converting shared shortcuts.".into());
    }
    if selected.is_empty() || selected.len() > MAX_RECORDS {
        return Err("Select between one and 500 shared shortcuts.".into());
    }
    if read_journal(layout)?.is_some_and(|j| !j.records.is_empty()) {
        return Err("An earlier shared-shortcut conversion still needs recovery. No new shared shortcuts were moved.".into());
    }
    let public = normal_dir(public)?;
    let desktop = normal_dir(desktop)?;
    let data = normal_dir(layout.parent().ok_or("The layout folder is unavailable.")?)?;
    if key(&public) == key(&desktop) || key(&data) == key(&desktop) || key(&data) == key(&public) {
        return Err("Shortcut staging must be separate from both Desktops.".into());
    }
    let mut journal = Journal {
        version: 1,
        public: public.clone(),
        desktop: desktop.clone(),
        records: Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut reserved = HashSet::new();
    for (source, position) in selected {
        if !shortcut(source) || position.is_some_and(|p| !p.is_valid()) {
            return Err(
                "Only regular shared Desktop .lnk and .url shortcuts can be made personal.".into(),
            );
        }
        let parent = normal_dir(
            source
                .parent()
                .ok_or("The shortcut folder is unavailable.")?,
        )?;
        if key(&parent) != key(&public) {
            return Err("A selected shortcut is not directly on the shared Desktop.".into());
        }
        let original = parent.join(source.file_name().ok_or("The shortcut has no filename.")?);
        if !seen.insert(key(&original)) {
            continue;
        }
        let (fingerprint, original_identity) = inspect(&original)?;
        let personal = fresh_personal(&desktop, &original, &reserved)?;
        reserved.insert(key(&personal));
        journal.records.push(Record {
            id: crate::model::new_id("shared"),
            original,
            personal,
            fingerprint,
            original_identity,
            position: *position,
            native_started: false,
            native_completed: false,
            stage_identity: None,
            personal_identity: None,
        });
    }
    let root = stage_root(layout);
    if absent(&root)? {
        fs::create_dir(&root).map_err(|e| e.to_string())?;
    }
    normal_dir(&root)?;
    let mut created = Vec::new();
    for record in &journal.records {
        let stage = stage_path(layout, record);
        if let Err(e) = fs::create_dir(&stage) {
            for path in created {
                let _ = fs::remove_dir(path);
            }
            return Err(format!("Could not reserve private shortcut staging: {e}"));
        }
        created.push(stage);
    }
    if let Err(e) = save(layout, &journal) {
        for path in created {
            let _ = fs::remove_dir(path);
        }
        return Err(e);
    }
    Ok(Prepared {
        layout: layout.to_path_buf(),
        public,
        desktop,
        ids: journal.records.iter().map(|e| e.id.clone()).collect(),
    })
}

impl Prepared {
    /// The supplied native call may pump Windows messages. It must own no App
    /// references or callbacks. Recovery inspects the recorded stage even when
    /// native cancellation/error is reported after a partial move.
    pub fn execute(
        self,
        mut move_into_stage: impl FnMut(&Path, &Path) -> Result<(), String>,
    ) -> Result<Report, String> {
        let mut journal =
            read_journal(&self.layout)?.ok_or("Shared-shortcut reservation disappeared.")?;
        if journal
            .records
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>()
            != self.ids
            || key(&journal.public) != key(&self.public)
            || key(&journal.desktop) != key(&self.desktop)
        {
            return Err(
                "The shared-shortcut reservation changed; no new native moves were requested."
                    .into(),
            );
        }
        run(
            &self.layout,
            &self.public,
            &self.desktop,
            &mut journal,
            Some(&mut move_into_stage),
        )
    }
}

/// Cheap and independent of layout/known-folder access when no journal exists.
pub fn recover(layout: &Path, public: &Path, desktop: &Path) -> Result<Report, String> {
    let Some(mut journal) = read_journal(layout)? else {
        return Ok(Report::default());
    };
    if journal.records.is_empty() {
        return Ok(Report::default());
    }
    run(layout, public, desktop, &mut journal, None)
}

type NativeMove<'a> = &'a mut dyn FnMut(&Path, &Path) -> Result<(), String>;

fn run(
    layout: &Path,
    public: &Path,
    desktop: &Path,
    journal: &mut Journal,
    mut native: Option<NativeMove<'_>>,
) -> Result<Report, String> {
    let public = normal_dir(public)?;
    let desktop = normal_dir(desktop)?;
    normal_dir(&stage_root(layout))?;
    if key(&public) != key(&journal.public) || key(&desktop) != key(&journal.desktop) {
        return Err("The shared or personal Desktop changed; conversion records were preserved for recovery.".into());
    }
    let mut report = Report::default();
    let ids: Vec<_> = journal.records.iter().map(|e| e.id.clone()).collect();
    let mut stop_native = false;
    for id in ids {
        let Some(index) = journal.records.iter().position(|e| e.id == id) else {
            continue;
        };
        let mut record = journal.records[index].clone();
        let mut native_error = None;
        if !stop_native && !record.native_started {
            if let Some(operation) = native.as_mut() {
                let stage = stage_path(layout, &record);
                let preflight = (|| -> Result<(), String> {
                    normal_dir(&stage)?;
                    if stage_item(&stage, &record)?.is_some() {
                        return Err("A fresh shortcut staging directory was no longer empty; no native move was requested.".into());
                    }
                    let (fingerprint, identity) = inspect(&record.original)?;
                    if fingerprint != record.fingerprint || identity != record.original_identity {
                        return Err("A selected shared shortcut changed before the move; it was left untouched.".into());
                    }
                    record.native_started = true;
                    if let Err(error) = checkpoint(layout, journal, index, &record) {
                        record.native_started = false;
                        return Err(error);
                    }
                    Ok(())
                })();
                match preflight {
                    Err(error) => {
                        native_error = Some(error);
                        stop_native = true;
                    }
                    Ok(()) => {
                        native_error = operation(&record.original, &stage).err();
                        record.native_completed = true;
                        if let Err(error) = checkpoint(layout, journal, index, &record) {
                            native_error = Some(error);
                        }
                        stop_native |= native_error.is_some();
                    }
                }
            }
        }
        match finish(layout, journal, index, record.clone()) {
            Ok(Some(converted)) => {
                report.converted.push(converted);
                if let Some(error) = native_error {
                    report.warnings.push(format!("Windows reported an interrupted move, but the shortcut was recovered onto your Desktop. {error}"));
                }
            }
            Ok(None) => {
                report.not_moved.push(record.original.clone());
                if let Some(error) = native_error {
                    report.warnings.push(error);
                }
            }
            Err(error) => {
                report.not_moved.push(record.original.clone());
                report.warnings.push(error);
                if let Some(error) = native_error {
                    report.warnings.push(error);
                }
                stop_native = true;
            }
        }
    }
    report.pending = journal.records.len();
    Ok(report)
}

fn checkpoint(
    layout: &Path,
    journal: &mut Journal,
    index: usize,
    record: &Record,
) -> Result<(), String> {
    let mut next = journal.clone();
    next.records[index] = record.clone();
    save(layout, &next)?;
    *journal = next;
    Ok(())
}

fn stage_item(stage: &Path, record: &Record) -> Result<Option<PathBuf>, String> {
    normal_dir(stage)?;
    let mut entries = fs::read_dir(stage).map_err(|e| e.to_string())?;
    let first = entries.next().transpose().map_err(|e| e.to_string())?;
    if entries.next().is_some() {
        return Err(format!(
            "Private shortcut staging contains unexpected extra files; it was preserved: {}",
            stage.display()
        ));
    }
    let Some(entry) = first else {
        return Ok(None);
    };
    let path = entry.path();
    if !shortcut(&path)
        || path.extension().map(|s| s.to_string_lossy().to_lowercase())
            != record
                .original
                .extension()
                .map(|s| s.to_string_lossy().to_lowercase())
        || inspect(&path)?.0 != record.fingerprint
    {
        return Err(format!(
            "A staged shortcut changed or was not recognized; it was preserved: {}",
            stage.display()
        ));
    }
    Ok(Some(path))
}

fn owned_personal(record: &Record) -> Result<bool, String> {
    if absent(&record.personal)? {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(&record.personal).map_err(|e| e.to_string())?;
    if !metadata.is_file() || reparse(&metadata) || metadata.len() > MAX_SHORTCUT {
        return Ok(false);
    }
    let (fingerprint, id) = inspect(&record.personal)?;
    Ok(fingerprint == record.fingerprint
        && (record.personal_identity.as_ref() == Some(&id)
            || record.stage_identity.as_ref() == Some(&id)))
}

fn unchanged_original(record: &Record) -> bool {
    inspect(&record.original).is_ok_and(|(fingerprint, id)| {
        fingerprint == record.fingerprint && id == record.original_identity
    })
}

fn finish(
    layout: &Path,
    journal: &mut Journal,
    index: usize,
    mut record: Record,
) -> Result<Option<Conversion>, String> {
    let stage = stage_path(layout, &record);
    let staged = stage_item(&stage, &record)?;
    if !record.native_started {
        if staged.is_some() {
            return Err(
                "An unstarted conversion contains unexpected staged data; it was preserved.".into(),
            );
        }
        retire(layout, journal, index, &stage)?;
        return Ok(None);
    }
    if let Some(staged) = staged {
        if unchanged_original(&record) {
            // A cancelled cross-volume native move can leave a second copy.
            // Only the unchanged identified original proves this copy is ours
            // to discard; another writer's replacement is never touched.
            if !record.native_completed {
                return Err("A native shared-shortcut move may still be running; its reservation was retained.".into());
            }
            fs::remove_file(&staged).map_err(|e| {
                format!(
                    "A cancelled move left a private copy at {}; the original is still shared. {e}",
                    staged.display()
                )
            })?;
            retire(layout, journal, index, &stage)?;
            return Ok(None);
        }
        if record.stage_identity.is_none() {
            record.stage_identity = Some(inspect(&staged)?.1);
            checkpoint(layout, journal, index, &record)?;
        }
        if owned_personal(&record)? {
            // A fully flushed cross-volume destination may precede removing
            // its private source. Verify both identities/content first.
            if inspect(&staged)?.1 != *record.stage_identity.as_ref().unwrap() {
                return Err(
                    "The staged shortcut identity changed; both files were preserved.".into(),
                );
            }
            fs::remove_file(&staged).map_err(|e| format!("The shortcut is visible on your Desktop, but its recovery copy could not be removed: {e}"))?;
        } else {
            if record.personal_identity.is_some() && !absent(&record.personal)? {
                resume_copy(&record, &staged)?;
            } else {
                if !absent(&record.personal)? {
                    let reserved: HashSet<_> = journal
                        .records
                        .iter()
                        .filter(|e| e.id != record.id)
                        .map(|e| key(&e.personal))
                        .collect();
                    record.personal =
                        fresh_personal(&journal.desktop, &record.original, &reserved)?;
                    checkpoint(layout, journal, index, &record)?;
                }
                deliver(layout, journal, index, &mut record, &staged)?;
            }
        }
    } else if !owned_personal(&record)? {
        if unchanged_original(&record) && (!record.native_started || record.native_completed) {
            retire(layout, journal, index, &stage)?;
            return Ok(None);
        }
        return Err(format!("Shared-shortcut recovery is waiting for the native move or its recorded file. Recovery records remain at {}.", journal_path(layout).display()));
    }
    if !owned_personal(&record)? {
        return Err("The converted shortcut could not be verified on your personal Desktop; recovery records were kept.".into());
    }
    // Persist aliases before forgetting how the former Public path maps to the
    // personal file. This preserves item IDs, names, arguments and nesting.
    if !layout.is_file() {
        return Err("The converted shortcut is on your personal Desktop. Its layout is missing, so the recovery record was retained.".into());
    }
    let mut config = Config::load(layout)?;
    desktop_transfer::remap_targets(
        &mut config,
        &record.original.to_string_lossy(),
        &record.personal.to_string_lossy(),
    );
    config.save(layout)?;
    let converted = Conversion {
        original: record.original.clone(),
        personal: record.personal.clone(),
        position: record.position,
    };
    retire(layout, journal, index, &stage)?;
    Ok(Some(converted))
}

fn deliver(
    layout: &Path,
    journal: &mut Journal,
    index: usize,
    record: &mut Record,
    staged: &Path,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
    }
    let source: Vec<_> = staged.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<_> = record
        .personal
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe { MoveFileW(source.as_ptr(), target.as_ptr()) } != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() != Some(17) {
        return Err(format!("The shortcut remains safely staged; an existing Desktop file was not overwritten. {error}"));
    }
    // Cross-volume: identify and journal our create_new output before copying
    // or deleting its source. A partial output is preserved, never mistaken for
    // an unrelated same-name file during a later recovery.
    use std::os::windows::fs::OpenOptionsExt;
    let mut input = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(staged)
        .map_err(|e| e.to_string())?;
    if identity(&input)?
        != *record
            .stage_identity
            .as_ref()
            .ok_or("The staging identity was not recorded.")?
    {
        return Err("The staging identity changed; no bytes were copied.".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&record.personal)
        .map_err(|e| e.to_string())?;
    record.personal_identity = Some(identity(&output)?);
    checkpoint(layout, journal, index, record)?;
    std::io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|e| {
            format!("The shortcut remains staged with an unfinished personal copy: {e}")
        })?;
    drop(output);
    drop(input);
    if !owned_personal(record)? {
        return Err(
            "The copied personal shortcut could not be verified; its staging copy was kept.".into(),
        );
    }
    fs::remove_file(staged).map_err(|e| {
        format!("The shortcut is on your Desktop; its private recovery copy was kept: {e}")
    })
}

/// Resume only an identified output whose bytes are still an exact prefix of
/// the unchanged source. We append, never truncate or overwrite existing bytes.
fn resume_copy(record: &Record, staged: &Path) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    for path in [staged, record.personal.as_path()] {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || reparse(&metadata) {
            return Err(
                "An interrupted conversion path changed; both files were preserved.".into(),
            );
        }
    }
    let mut input = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(staged)
        .map_err(|e| e.to_string())?;
    let mut output = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(1)
        .open(&record.personal)
        .map_err(|e| e.to_string())?;
    if Some(&identity(&input)?) != record.stage_identity.as_ref()
        || Some(&identity(&output)?) != record.personal_identity.as_ref()
    {
        return Err(
            "An interrupted conversion file was replaced; both files were preserved.".into(),
        );
    }
    let length = output.metadata().map_err(|e| e.to_string())?.len();
    if length > record.fingerprint.length {
        return Err("An interrupted personal shortcut changed; both files were preserved.".into());
    }
    let mut remaining = length;
    let mut source_buffer = [0u8; 64 * 1024];
    let mut output_buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let count = remaining.min(source_buffer.len() as u64) as usize;
        input
            .read_exact(&mut source_buffer[..count])
            .map_err(|e| e.to_string())?;
        output
            .read_exact(&mut output_buffer[..count])
            .map_err(|e| e.to_string())?;
        if source_buffer[..count] != output_buffer[..count] {
            return Err(
                "An interrupted personal shortcut was edited; both files were preserved.".into(),
            );
        }
        remaining -= count as u64;
    }
    output.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    std::io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    drop(output);
    drop(input);
    if !owned_personal(record)? {
        return Err(
            "The resumed personal shortcut could not be verified; staging was preserved.".into(),
        );
    }
    fs::remove_file(staged).map_err(|e| e.to_string())
}

fn retire(layout: &Path, journal: &mut Journal, index: usize, stage: &Path) -> Result<(), String> {
    let mut next = journal.clone();
    next.records.remove(index);
    save(layout, &next)?;
    *journal = next;
    // Non-recursive cleanup only. Unexpected files are never removed.
    let _ = fs::remove_dir(stage);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Item;
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let root = std::env::temp_dir().join(crate::model::new_id("shared-conversion-test"));
            for name in ["Public", "Desktop", "Data"] {
                fs::create_dir_all(root.join(name)).unwrap();
            }
            Config::default()
                .save(&root.join("Data/layout.json"))
                .unwrap();
            Self(root)
        }
        fn layout(&self) -> PathBuf {
            self.0.join("Data/layout.json")
        }
        fn public(&self) -> PathBuf {
            self.0.join("Public")
        }
        fn desktop(&self) -> PathBuf {
            self.0.join("Desktop")
        }
        fn source(&self) -> PathBuf {
            let path = self.public().join("Example.lnk");
            fs::write(&path, b"synthetic shortcut bytes; never executed").unwrap();
            path
        }
        fn prepare(&self, source: &Path) -> Prepared {
            prepare(
                &self.layout(),
                &self.public(),
                &self.desktop(),
                &[(
                    source.to_path_buf(),
                    Some(DesktopPosition { x: 116, y: 184 }),
                )],
            )
            .unwrap()
        }
        fn recover(&self) -> Report {
            recover(&self.layout(), &self.public(), &self.desktop()).unwrap()
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reservation_precedes_native_move_and_aliases_keep_ids_and_coordinates() {
        let s = Sandbox::new();
        let source = s.source();
        let mut config = Config::load(&s.layout()).unwrap();
        let alias = Item::shortcut("Custom app name", source.to_string_lossy().into_owned());
        let id = alias.id.clone();
        config.groups[0].items.push(alias);
        config.save(&s.layout()).unwrap();
        let prepared = s.prepare(&source);
        let report = prepared
            .execute(|path, stage| {
                let journal = read_journal(&s.layout()).unwrap().unwrap();
                assert_eq!(journal.records.len(), 1);
                assert!(journal.records[0].native_started);
                assert!(!journal.records[0].native_completed);
                fs::rename(path, stage.join(path.file_name().unwrap())).map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(report.pending, 0);
        assert_eq!(report.converted.len(), 1);
        assert!(!source.exists());
        let item = Config::load(&s.layout())
            .unwrap()
            .find_item(&id)
            .unwrap()
            .clone();
        assert_eq!(item.id, id);
        assert_eq!(item.name, "Custom app name");
        assert_eq!(
            key(Path::new(&item.target)),
            key(&report.converted[0].personal)
        );
        assert_eq!(
            report.converted[0].position,
            Some(DesktopPosition { x: 116, y: 184 })
        );
        assert_eq!(
            fs::read(&report.converted[0].personal).unwrap(),
            b"synthetic shortcut bytes; never executed"
        );
    }

    #[test]
    fn cancellation_preserves_shared_shortcut_and_retires_known_completed_reservation() {
        let s = Sandbox::new();
        let source = s.source();
        let report = s
            .prepare(&source)
            .execute(|_, _| Err("User cancelled Windows move".into()))
            .unwrap();
        assert!(source.exists());
        assert_eq!(report.not_moved.len(), 1);
        assert!(report.converted.is_empty());
        assert_eq!(report.pending, 0);
        assert_eq!(fs::read_dir(s.desktop()).unwrap().count(), 0);
    }

    #[test]
    fn native_error_after_move_still_recovers_forward_and_renamed_file_is_recognized() {
        let s = Sandbox::new();
        let source = s.source();
        let report = s
            .prepare(&source)
            .execute(|path, stage| {
                fs::rename(path, stage.join("Example (2).lnk")).unwrap();
                Err("Interrupted callback".into())
            })
            .unwrap();
        assert_eq!(report.converted.len(), 1);
        assert_eq!(report.pending, 0);
        assert!(!report.warnings.is_empty());
        assert!(report.converted[0].personal.exists());
    }

    #[test]
    fn existing_and_racing_personal_names_are_never_overwritten() {
        let s = Sandbox::new();
        let source = s.source();
        fs::write(s.desktop().join("Example.lnk"), b"existing personal bytes").unwrap();
        let prepared = s.prepare(&source);
        fs::write(
            s.desktop().join("Example (2).lnk"),
            b"racing personal bytes",
        )
        .unwrap();
        let report = prepared
            .execute(|path, stage| {
                fs::rename(path, stage.join(path.file_name().unwrap())).map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(
            fs::read(s.desktop().join("Example.lnk")).unwrap(),
            b"existing personal bytes"
        );
        assert_eq!(
            fs::read(s.desktop().join("Example (2).lnk")).unwrap(),
            b"racing personal bytes"
        );
        assert_eq!(
            report.converted[0].personal.file_name().unwrap(),
            "Example (3).lnk"
        );
    }

    #[test]
    fn crash_after_native_move_before_callback_recovers_the_recorded_stage() {
        let s = Sandbox::new();
        let source = s.source();
        let _prepared = s.prepare(&source);
        let mut journal = read_journal(&s.layout()).unwrap().unwrap();
        let stage = stage_path(&s.layout(), &journal.records[0]);
        journal.records[0].native_started = true;
        save(&s.layout(), &journal).unwrap();
        fs::rename(&source, stage.join("Example.lnk")).unwrap();
        let report = s.recover();
        assert_eq!(report.converted.len(), 1);
        assert_eq!(report.pending, 0);
    }

    #[test]
    fn crash_during_native_prompt_retains_reservation_until_file_arrives() {
        let s = Sandbox::new();
        let source = s.source();
        let _prepared = s.prepare(&source);
        let mut journal = read_journal(&s.layout()).unwrap().unwrap();
        let stage = stage_path(&s.layout(), &journal.records[0]);
        journal.records[0].native_started = true;
        save(&s.layout(), &journal).unwrap();
        let report = s.recover();
        assert_eq!(report.pending, 1);
        assert!(source.exists());
        fs::rename(&source, stage.join("Example.lnk")).unwrap();
        assert_eq!(s.recover().converted.len(), 1);
    }

    #[test]
    fn crash_after_atomic_personal_delivery_is_identified_without_reclaiming_foreign_file() {
        let s = Sandbox::new();
        let source = s.source();
        let _prepared = s.prepare(&source);
        let mut journal = read_journal(&s.layout()).unwrap().unwrap();
        let mut entry = journal.records[0].clone();
        let stage = stage_path(&s.layout(), &entry);
        entry.native_started = true;
        fs::rename(&source, stage.join("Example.lnk")).unwrap();
        entry.stage_identity = Some(inspect(&stage.join("Example.lnk")).unwrap().1);
        journal.records[0] = entry.clone();
        save(&s.layout(), &journal).unwrap();
        fs::rename(stage.join("Example.lnk"), &entry.personal).unwrap();
        let report = s.recover();
        assert_eq!(report.converted.len(), 1);
        assert_eq!(report.pending, 0);
    }

    #[test]
    fn changed_staged_bytes_and_extra_files_keep_recovery_records() {
        let s = Sandbox::new();
        let source = s.source();
        let report = s
            .prepare(&source)
            .execute(|path, stage| {
                fs::rename(path, stage.join("Example.lnk")).unwrap();
                fs::write(stage.join("unexpected.txt"), b"foreign").unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(report.pending, 1);
        assert!(report.converted.is_empty());
        assert_eq!(fs::read_dir(s.desktop()).unwrap().count(), 0);
        let journal = read_journal(&s.layout()).unwrap().unwrap();
        let stage = stage_path(&s.layout(), &journal.records[0]);
        assert_eq!(fs::read(stage.join("unexpected.txt")).unwrap(), b"foreign");
        fs::remove_file(stage.join("unexpected.txt")).unwrap();
        fs::write(stage.join("Example.lnk"), b"changed staged bytes").unwrap();
        assert_eq!(s.recover().pending, 1);
    }

    #[test]
    fn corrupt_layout_never_loses_converted_personal_file_or_alias_record() {
        let s = Sandbox::new();
        let source = s.source();
        let prepared = s.prepare(&source);
        let report = prepared
            .execute(|path, stage| {
                fs::rename(path, stage.join("Example.lnk")).unwrap();
                fs::write(s.layout(), b"{invalid layout").unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(report.pending, 1);
        assert!(s.desktop().join("Example.lnk").exists());
        assert_eq!(fs::read(s.layout()).unwrap(), b"{invalid layout");
        assert!(read_journal(&s.layout()).unwrap().unwrap().records[0]
            .stage_identity
            .is_some());
    }

    #[test]
    fn invalid_selection_and_corrupt_journal_prevent_native_work() {
        let s = Sandbox::new();
        let source = s.source();
        let foreign = s.desktop().join("Other.lnk");
        fs::write(&foreign, b"other").unwrap();
        assert!(prepare(
            &s.layout(),
            &s.public(),
            &s.desktop(),
            &[(source.clone(), None), (foreign, None)]
        )
        .is_err());
        assert!(!journal_path(&s.layout()).exists());
        assert!(source.exists());
        fs::write(journal_path(&s.layout()), b"corrupt journal").unwrap();
        assert!(prepare(
            &s.layout(),
            &s.public(),
            &s.desktop(),
            &[(source.clone(), None)]
        )
        .is_err());
        assert!(s.recover_err());
        assert!(source.exists());
    }
    impl Sandbox {
        fn recover_err(&self) -> bool {
            recover(&self.layout(), &self.public(), &self.desktop()).is_err()
        }
    }

    #[test]
    fn no_journal_recovery_needs_no_layout_or_desktop_access() {
        let root = std::env::temp_dir().join(crate::model::new_id("shared-missing-test"));
        assert_eq!(
            recover(
                &root.join("Missing/layout.json"),
                &root.join("NoPublic"),
                &root.join("NoDesktop")
            )
            .unwrap()
            .pending,
            0
        );
    }

    #[test]
    fn failed_new_checkpoint_never_deletes_an_existing_file() {
        let s = Sandbox::new();
        let existing = s.0.join("Existing checkpoint.tmp");
        fs::write(&existing, b"unrelated existing bytes").unwrap();
        assert!(write_new(&existing, b"new checkpoint").is_err());
        assert_eq!(fs::read(existing).unwrap(), b"unrelated existing bytes");
    }

    fn interrupted_copy(s: &Sandbox, prefix: &[u8]) -> (Record, PathBuf) {
        let source = s.source();
        let _prepared = s.prepare(&source);
        let mut journal = read_journal(&s.layout()).unwrap().unwrap();
        let mut entry = journal.records[0].clone();
        let staged = stage_path(&s.layout(), &entry).join("Example.lnk");
        fs::rename(source, &staged).unwrap();
        entry.native_started = true;
        entry.native_completed = true;
        entry.stage_identity = Some(inspect(&staged).unwrap().1);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&entry.personal)
            .unwrap();
        entry.personal_identity = Some(identity(&output).unwrap());
        journal.records[0] = entry.clone();
        save(&s.layout(), &journal).unwrap();
        output.write_all(prefix).unwrap();
        output.sync_all().unwrap();
        drop(output);
        (entry, staged)
    }

    #[test]
    fn interrupted_copy_resumes_only_the_recorded_prefix_and_keeps_aliases() {
        let s = Sandbox::new();
        let (entry, staged) = interrupted_copy(&s, b"synthetic short");
        let report = s.recover();
        assert_eq!(report.converted.len(), 1);
        assert_eq!(report.pending, 0);
        assert!(!staged.exists());
        assert_eq!(
            fs::read(entry.personal).unwrap(),
            b"synthetic shortcut bytes; never executed"
        );
    }

    #[test]
    fn replaced_or_edited_partial_personal_file_is_never_overwritten() {
        let s = Sandbox::new();
        let (entry, staged) = interrupted_copy(&s, b"synthetic short");
        let former = entry.personal.with_extension("saved");
        fs::rename(&entry.personal, &former).unwrap();
        fs::write(&entry.personal, b"synthetic short").unwrap();
        let report = s.recover();
        assert_eq!(report.pending, 1);
        assert!(staged.exists());
        assert_eq!(fs::read(&entry.personal).unwrap(), b"synthetic short");
        // Same pathname and same prefix do not grant ownership: identity differs.
        fs::remove_file(&entry.personal).unwrap();
        fs::rename(former, &entry.personal).unwrap();
        fs::write(&entry.personal, b"user edited partial file").unwrap();
        assert_eq!(s.recover().pending, 1);
        assert_eq!(
            fs::read(&entry.personal).unwrap(),
            b"user edited partial file"
        );
        assert!(staged.exists());
    }

    #[test]
    fn foreign_same_bytes_at_reserved_personal_path_are_preserved() {
        let s = Sandbox::new();
        let source = s.source();
        let prepared = s.prepare(&source);
        let entry = read_journal(&s.layout()).unwrap().unwrap().records[0].clone();
        let bytes = fs::read(&source).unwrap();
        fs::write(&entry.personal, &bytes).unwrap();
        let foreign_id = inspect(&entry.personal).unwrap().1;
        let report = prepared
            .execute(|path, stage| {
                fs::rename(path, stage.join("Example.lnk")).map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(report.converted.len(), 1);
        assert_ne!(key(&report.converted[0].personal), key(&entry.personal));
        assert_eq!(inspect(&entry.personal).unwrap().1, foreign_id);
        assert_eq!(fs::read(entry.personal).unwrap(), bytes);
    }

    #[test]
    fn changed_unstarted_second_source_does_not_lose_first_conversion_report() {
        let s = Sandbox::new();
        let first = s.source();
        let first_original = fs::canonicalize(&first).unwrap();
        let second = s.public().join("Second.url");
        let third = s.public().join("Third.lnk");
        fs::write(&second, b"original second shortcut").unwrap();
        fs::write(&third, b"untouched third shortcut").unwrap();
        let prepared = prepare(
            &s.layout(),
            &s.public(),
            &s.desktop(),
            &[(first, None), (second.clone(), None), (third.clone(), None)],
        )
        .unwrap();
        let mut calls = 0;
        let report = prepared
            .execute(|path, stage| {
                calls += 1;
                fs::rename(path, stage.join(path.file_name().unwrap())).unwrap();
                fs::write(&second, b"changed second shortcut").unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(report.converted.len(), 1);
        assert_eq!(report.converted[0].original, first_original);
        assert_eq!(
            key(&report.converted[0].personal),
            key(&s.desktop().join("Example.lnk"))
        );
        assert_eq!(
            fs::read(&report.converted[0].personal).unwrap(),
            b"synthetic shortcut bytes; never executed"
        );
        assert!(!report.warnings.is_empty());
        assert_eq!(
            report.not_moved,
            vec![
                fs::canonicalize(second.clone()).unwrap(),
                fs::canonicalize(third.clone()).unwrap()
            ]
        );
        assert_eq!(report.pending, 0);
        assert_eq!(fs::read(second).unwrap(), b"changed second shortcut");
        assert_eq!(fs::read(third).unwrap(), b"untouched third shortcut");
    }

    #[test]
    fn corrupt_record_cannot_escape_private_stage_or_personal_desktop() {
        let s = Sandbox::new();
        let source = s.source();
        let _prepared = s.prepare(&source);
        let mut journal = read_journal(&s.layout()).unwrap().unwrap();
        journal.records[0].id = "..\\outside".into();
        fs::write(
            journal_path(&s.layout()),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        assert!(s.recover_err());
        assert!(source.exists());
        journal.records[0].id = "safe-record".into();
        journal.records[0].personal = s.0.join("Outside.lnk");
        fs::write(
            journal_path(&s.layout()),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        assert!(s.recover_err());
        assert!(source.exists());
        assert!(!s.0.join("Outside.lnk").exists());
    }

    #[test]
    fn duplicate_selection_is_one_native_move_and_original_replacement_is_preserved() {
        let s = Sandbox::new();
        let source = s.source();
        let mut calls = 0;
        let prepared = prepare(
            &s.layout(),
            &s.public(),
            &s.desktop(),
            &[(source.clone(), None), (source.clone(), None)],
        )
        .unwrap();
        let report = prepared
            .execute(|path, stage| {
                calls += 1;
                fs::rename(path, stage.join("Example.lnk")).unwrap();
                fs::write(path, b"replacement shared file").unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(report.converted.len(), 1);
        assert_eq!(fs::read(source).unwrap(), b"replacement shared file");
    }
}
