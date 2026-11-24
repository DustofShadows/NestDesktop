//! One explicitly approved Public Desktop move through the Windows Shell broker.
//!
//! The caller owns a durable migration journal and a fresh per-item stage.
//! This module never deletes, overwrites, rolls back or borrows App. Even an
//! error can leave a completed file in the stage; the journal must inspect it.
use std::{
    fs,
    os::windows::fs::MetadataExt,
    path::{Component, Path, PathBuf, Prefix},
    sync::{Arc, Mutex},
};
use windows::{
    core::{implement, PCWSTR},
    Win32::{
        Foundation::{E_ABORT, E_ACCESSDENIED, E_FAIL, E_INVALIDARG, HWND},
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL},
        UI::Shell::*,
    },
};

struct MovePaths {
    source: PathBuf,
    stage: PathBuf,
    expected: PathBuf,
}

#[derive(Default)]
struct CallbackState {
    // A successful top-level HRESULT alone does not prove that an item moved.
    post: Option<(i32, Option<PathBuf>)>,
}

#[implement(IFileOperationProgressSink)]
struct MoveSink {
    source: PathBuf,
    stage: PathBuf,
    state: Arc<Mutex<CallbackState>>,
}

#[allow(non_snake_case, unused_variables)]
impl IFileOperationProgressSink_Impl for MoveSink_Impl {
    fn StartOperations(&self) -> windows_core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, hrresult: windows_core::HRESULT) -> windows_core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
        psidestinationfolder: Option<&IShellItem>,
        psznewname: &PCWSTR,
    ) -> windows_core::Result<()> {
        if dwflags & TSF_OVERWRITE_EXIST.0 as u32 != 0 {
            return Err(E_ABORT.into());
        }
        let source =
            unsafe { item_path(psiitem.ok_or_else(|| windows_core::Error::from(E_INVALIDARG))?)? };
        let stage = unsafe {
            item_path(psidestinationfolder.ok_or_else(|| windows_core::Error::from(E_INVALIDARG))?)?
        };
        let source =
            regular_file(&source).map_err(|_| windows_core::Error::from(E_ACCESSDENIED))?;
        let stage =
            normal_directory(&stage).map_err(|_| windows_core::Error::from(E_ACCESSDENIED))?;
        if source != self.source || stage != self.stage {
            return Err(E_ACCESSDENIED.into());
        }
        // A collision arising after the initial validation is not permission to
        // overwrite. Shell may generate a distinct name, which recovery finds
        // inside this one recorded staging directory.
        if !psznewname.is_null() {
            let name = unsafe { psznewname.to_string()? };
            if !name.is_empty()
                && (Path::new(&name).file_name() != self.source.file_name()
                    || Path::new(&name).components().count() != 1)
            {
                return Err(E_ACCESSDENIED.into());
            }
        }
        Ok(())
    }
    fn PostMoveItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
        psidestinationfolder: Option<&IShellItem>,
        psznewname: &PCWSTR,
        hrmove: windows_core::HRESULT,
        psinewlycreated: Option<&IShellItem>,
    ) -> windows_core::Result<()> {
        let actual = unsafe { psinewlycreated.map(|item| item_path(item)).transpose()? };
        if let Some(actual) = actual.as_ref() {
            let actual =
                regular_file(actual).map_err(|_| windows_core::Error::from(E_ACCESSDENIED))?;
            if actual.parent() != Some(self.stage.as_path()) {
                return Err(E_ACCESSDENIED.into());
            }
        }
        self.state
            .lock()
            .map_err(|_| windows_core::Error::from(E_FAIL))?
            .post = Some((hrmove.0, actual));
        Ok(())
    }
    fn PreRenameItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
        psznewname: &PCWSTR,
    ) -> windows_core::Result<()> {
        Err(E_ACCESSDENIED.into())
    }
    fn PostRenameItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
        psznewname: &PCWSTR,
        hrrename: windows_core::HRESULT,
        psinewlycreated: Option<&IShellItem>,
    ) -> windows_core::Result<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
        psidestinationfolder: Option<&IShellItem>,
        psznewname: &PCWSTR,
    ) -> windows_core::Result<()> {
        Err(E_ACCESSDENIED.into())
    }
    fn PostCopyItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
        psidestinationfolder: Option<&IShellItem>,
        psznewname: &PCWSTR,
        hrcopy: windows_core::HRESULT,
        psinewlycreated: Option<&IShellItem>,
    ) -> windows_core::Result<()> {
        Ok(())
    }
    fn PreDeleteItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
    ) -> windows_core::Result<()> {
        Err(E_ACCESSDENIED.into())
    }
    fn PostDeleteItem(
        &self,
        dwflags: u32,
        psiitem: Option<&IShellItem>,
        hrdelete: windows_core::HRESULT,
        psinewlycreated: Option<&IShellItem>,
    ) -> windows_core::Result<()> {
        Ok(())
    }
    fn PreNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: Option<&IShellItem>,
        psznewname: &PCWSTR,
    ) -> windows_core::Result<()> {
        Err(E_ACCESSDENIED.into())
    }
    fn PostNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: Option<&IShellItem>,
        psznewname: &PCWSTR,
        psztemplatename: &PCWSTR,
        dwfileattributes: u32,
        hrnew: windows_core::HRESULT,
        psinewitem: Option<&IShellItem>,
    ) -> windows_core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, iworktotal: u32, iworksofar: u32) -> windows_core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows_core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows_core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows_core::Result<()> {
        Ok(())
    }
}

/// Caller must be on an initialized COM STA thread and must have durably
/// recorded `source` and `stage` before calling. The supplied owner receives
/// Windows' elevation UI; only the user can approve it.
///
/// An Err never authorizes deleting or forgetting stage contents. The Shell
/// can complete a move before a later cancellation/failure is reported.
pub unsafe fn move_shortcut(
    owner: HWND,
    source: &Path,
    public: &Path,
    stage: &Path,
) -> Result<(), String> {
    let paths = validate_paths(source, public, stage)?;
    let state = Arc::new(Mutex::new(CallbackState::default()));
    let sink: IFileOperationProgressSink = MoveSink {
        source: paths.source.clone(),
        stage: paths.stage.clone(),
        state: state.clone(),
    }
    .into();
    let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)
        .map_err(|error| format!("Windows could not prepare the shared shortcut move: {error}"))?;
    operation
        .SetOwnerWindow(owner)
        .map_err(|error| error.to_string())?;
    // SetOperationFlags replaces the default undo/confirmation settings. Never
    // add FOF_NOCONFIRMATION/FOF_NO_UI (Yes to All), KEEPNEWERFILE or overwrite.
    operation
        .SetOperationFlags(
            FOF_SILENT
                | FOF_NOERRORUI
                | FOFX_SHOWELEVATIONPROMPT
                | FOFX_EARLYFAILURE
                | FOF_RENAMEONCOLLISION
                | FOFX_PRESERVEFILEEXTENSIONS
                | FOF_NO_CONNECTED_ELEMENTS,
        )
        .map_err(|error| error.to_string())?;
    let source_name = shell_name(&paths.source)?;
    let stage_name = shell_name(&paths.stage)?;
    let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(source_name.as_ptr()), None)
        .map_err(|error| format!("Windows could not read the selected shortcut: {error}"))?;
    let folder: IShellItem = SHCreateItemFromParsingName(PCWSTR(stage_name.as_ptr()), None)
        .map_err(|error| format!("Windows could not read the shortcut staging folder: {error}"))?;
    operation
        .MoveItem(&item, &folder, PCWSTR::null(), &sink)
        .map_err(|error| error.to_string())?;
    let performed = operation.PerformOperations();
    // Query this on BOTH success and failure. A canceled operation can return
    // S_OK; callback E_ABORT can also return a failure with aborted=false.
    let aborted = operation.GetAnyOperationsAborted();
    drop(operation);
    drop(sink);

    let post = state
        .lock()
        .map_err(|_| "The shared shortcut result could not be read.".to_string())?
        .post
        .take();
    let source_absent = matches!(fs::symlink_metadata(&paths.source), Err(error) if error.kind() == std::io::ErrorKind::NotFound);
    let expected = regular_file(&paths.expected).ok();
    let confirmed = post.as_ref().is_some_and(|(result, actual)| {
        *result >= 0
            && actual.as_ref().and_then(|path| regular_file(path).ok()) == expected
            && expected.is_some()
    });
    // Physical completion wins over a later batch cancellation. The durable
    // caller independently verifies bytes before committing a personal target.
    if source_absent && expected.is_some() && confirmed {
        return Ok(());
    }
    if let Err(error) = performed {
        return Err(format!("Windows did not complete the shared shortcut move. Any staged shortcut is retained for recovery. {error}"));
    }
    match aborted {
        Ok(value) if value.as_bool() => Err("The shared shortcut move was canceled. Any staged shortcut is retained for recovery.".into()),
        Err(error) => Err(format!("Windows could not confirm the shared shortcut move. Any staged shortcut is retained for recovery. {error}")),
        _ => Err("The shared shortcut move needs recovery verification; any staged shortcut has been retained.".into()),
    }
}

fn validate_paths(source: &Path, public: &Path, stage: &Path) -> Result<MovePaths, String> {
    let public = normal_directory(public)?;
    let source = regular_file(source)?;
    let stage = normal_directory(stage)?;
    if source.parent() != Some(public.as_path())
        || !matches!(
            source
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("lnk" | "url")
        )
    {
        return Err("Only selected shortcuts directly on the shared Desktop can be moved.".into());
    }
    if stage.starts_with(&public) || public.starts_with(&stage) {
        return Err("Shared shortcut staging must be separate from the shared Desktop.".into());
    }
    if fs::read_dir(&stage)
        .map_err(|error| format!("The shortcut staging folder cannot be inspected: {error}"))?
        .next()
        .transpose()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err(
            "The shortcut staging folder is not empty; no shared shortcut was moved.".into(),
        );
    }
    let expected = stage.join(
        source
            .file_name()
            .ok_or("The shared shortcut has no filename.")?,
    );
    Ok(MovePaths {
        source,
        stage,
        expected,
    })
}

fn normal_directory(path: &Path) -> Result<PathBuf, String> {
    let info = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !info.is_dir() || info.file_attributes() & 0x400 != 0 {
        return Err("Shortcut folders must be normal local directories.".into());
    }
    let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err("Shared shortcut moves require local folders.".into());
    }
    Ok(path)
}
fn regular_file(path: &Path) -> Result<PathBuf, String> {
    let info = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !info.is_file() || info.file_attributes() & 0x400 != 0 {
        return Err("The selected shortcut is not a regular file.".into());
    }
    fs::canonicalize(path).map_err(|error| error.to_string())
}
fn shell_name(path: &Path) -> Result<Vec<u16>, String> {
    let value = path
        .to_str()
        .ok_or("The shortcut path cannot be represented for Windows Shell.")?;
    let value = value
        .strip_prefix(r"\\?\")
        .unwrap_or(value)
        .replace('/', "\\");
    if value.contains('\0') {
        return Err("The shortcut path is invalid.".into());
    }
    Ok(value.encode_utf16().chain(Some(0)).collect())
}
unsafe fn item_path(item: &IShellItem) -> windows_core::Result<PathBuf> {
    let value = item.GetDisplayName(SIGDN_FILESYSPATH)?;
    let result = value.to_string();
    CoTaskMemFree(Some(value.0.cast()));
    Ok(PathBuf::from(result?))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(crate::model::new_id("shared-shell-test"));
            fs::create_dir_all(root.join("Public")).unwrap();
            fs::create_dir_all(root.join("Stage")).unwrap();
            Self(root)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn rejects_documents_and_nested_sources_before_shell_operation() {
        let root = Scratch::new();
        let public = root.0.join("Public");
        let stage = root.0.join("Stage");
        fs::create_dir_all(public.join("Nested")).unwrap();
        for name in ["Document.txt", "Nested/App.lnk"] {
            let source = public.join(name);
            fs::write(&source, b"unchanged synthetic bytes").unwrap();
            assert!(unsafe { move_shortcut(HWND::default(), &source, &public, &stage) }.is_err());
            assert_eq!(fs::read(source).unwrap(), b"unchanged synthetic bytes");
        }
        assert_eq!(fs::read_dir(stage).unwrap().count(), 0);
    }
    #[test]
    fn rejects_occupied_stage_without_touching_either_file() {
        let root = Scratch::new();
        let public = root.0.join("Public");
        let stage = root.0.join("Stage");
        let source = public.join("App.lnk");
        fs::write(&source, b"selected synthetic bytes").unwrap();
        fs::write(stage.join("App.lnk"), b"occupied synthetic bytes").unwrap();
        assert!(unsafe { move_shortcut(HWND::default(), &source, &public, &stage) }.is_err());
        assert_eq!(fs::read(source).unwrap(), b"selected synthetic bytes");
        assert_eq!(
            fs::read(stage.join("App.lnk")).unwrap(),
            b"occupied synthetic bytes"
        );
    }
    #[test]
    fn validates_only_a_direct_shortcut_and_separate_empty_stage() {
        let root = Scratch::new();
        let public = root.0.join("Public");
        let stage = root.0.join("Stage");
        let source = public.join("App.url");
        fs::write(&source, b"synthetic; never launched").unwrap();
        let paths = validate_paths(&source, &public, &stage).unwrap();
        assert_eq!(
            paths.expected,
            fs::canonicalize(&stage).unwrap().join("App.url")
        );
        assert_eq!(fs::read(source).unwrap(), b"synthetic; never launched");
    }
}
