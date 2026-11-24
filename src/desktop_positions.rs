//! Event-driven Explorer notifications and original Desktop icon placement.
//!
//! One apartment/view connection serves a transfer batch. No desktop scanning,
//! remote-process memory, setting changes, sleeps or idle work are involved.
use crate::shortcuts::DesktopPosition;
use std::{
    collections::HashSet,
    fs,
    marker::PhantomData,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
};
use windows::{
    core::{w, Interface, PCWSTR, VARIANT},
    Win32::{
        Foundation::{POINT, S_FALSE, S_OK},
        Graphics::Gdi::ScreenToClient,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IServiceProvider,
            CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
        },
        UI::{
            Shell::{
                Common::ITEMIDLIST, IFolderView, IShellBrowser, IShellFolder, IShellWindows,
                SHChangeNotify, SHGetNameFromIDList, SID_STopLevelBrowser, ShellWindows,
                SHCNE_CREATE, SHCNE_DELETE, SHCNF_FLUSHNOWAIT, SHCNF_PATHW, SIGDN_FILESYSPATH,
                SVSI_NOTAKEFOCUS, SVSI_POSITIONITEM, SWC_DESKTOP, SWFO_NEEDDISPATCH,
            },
            WindowsAndMessaging::FindWindowExW,
        },
    },
};

// Documented outside _SVSIF; preserve the user's existing selection/focus.
const SVSI_NOSTATECHANGE: u32 = 0x8000_0000;

struct Apartment;
impl Apartment {
    unsafe fn new() -> Result<Self, String> {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|error| format!("The Desktop Shell apartment is unavailable: {error}"))?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

struct Pidl(*mut ITEMIDLIST);
impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) }
    }
}

/// The caller can retry `pending` once or on a short bounded worker schedule if
/// Explorer has not yet inserted a just-restored file. No retry runs here.
#[derive(Default, Debug)]
pub struct PlacementReport {
    pub positioned: usize,
    pub pending: Vec<(PathBuf, DesktopPosition)>,
    pub auto_arrange: bool,
    pub warnings: Vec<String>,
}

/// COM interfaces deliberately precede the apartment guard in drop order.
/// Keep the session on its creating thread, including in recovery workers.
pub struct DesktopSession {
    view: IFolderView,
    folder: IShellFolder,
    desktop: PathBuf,
    _apartment: Apartment,
    _same_thread: PhantomData<Rc<()>>,
}

impl DesktopSession {
    pub fn connect() -> Result<Self, String> {
        unsafe {
            let apartment = Apartment::new()?;
            let desktop = fs::canonicalize(crate::platform::desktop_directory())
                .map_err(|error| format!("The personal Desktop is unavailable: {error}"))?;
            let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)
                .map_err(|error| format!("Explorer is unavailable: {error}"))?;
            let empty = VARIANT::default();
            let mut desktop_handle = 0;
            let dispatch = windows
                .FindWindowSW(
                    &empty,
                    &empty,
                    SWC_DESKTOP,
                    &mut desktop_handle,
                    SWFO_NEEDDISPATCH,
                )
                .map_err(|error| format!("The Desktop view is unavailable: {error}"))?;
            let provider = dispatch
                .cast::<IServiceProvider>()
                .map_err(|error| error.to_string())?;
            let browser: IShellBrowser = provider
                .QueryService(&SID_STopLevelBrowser)
                .map_err(|error| error.to_string())?;
            let view = browser
                .QueryActiveShellView()
                .and_then(|shell_view| shell_view.cast::<IFolderView>())
                .map_err(|error| format!("Explorer cannot expose Desktop positions: {error}"))?;
            let folder = view
                .GetFolder::<IShellFolder>()
                .map_err(|error| error.to_string())?;
            Ok(Self {
                view,
                folder,
                desktop,
                _apartment: apartment,
                _same_thread: PhantomData,
            })
        }
    }

    /// Capture before removing a personal Desktop shortcut.
    pub fn capture(&self, path: &Path) -> Result<Option<DesktopPosition>, String> {
        self.capture_from(path, &self.desktop)
    }

    /// One-time shared conversion may preserve the shared icon's current slot.
    /// Resolving an identically named personal item must not be mistaken for it.
    pub fn capture_public(
        &self,
        path: &Path,
        public: &Path,
    ) -> Result<Option<DesktopPosition>, String> {
        let public = fs::canonicalize(public).map_err(|error| error.to_string())?;
        self.capture_from(path, &public)
    }

    fn capture_from(&self, path: &Path, root: &Path) -> Result<Option<DesktopPosition>, String> {
        let Some(path) = personal_item(root, path)? else {
            return Ok(None);
        };
        unsafe {
            let pidl = self.pidl_for(&path)?;
            let point = self.view.GetItemPosition(pidl.0).map_err(|error| {
                format!("The original Desktop position is unavailable: {error}")
            })?;
            let position = DesktopPosition {
                x: point.x,
                y: point.y,
            };
            if !position.is_valid() {
                return Err("Explorer returned an invalid Desktop position.".into());
            }
            Ok(Some(position))
        }
    }

    /// Coordinates already belong to the view; do not translate them as screen
    /// points. One SelectAndPositionItems call handles every resolved file.
    pub fn restore_positions(
        &self,
        entries: &[(PathBuf, DesktopPosition)],
    ) -> Result<PlacementReport, String> {
        self.apply_positions(entries, true)
    }

    /// Retry a drag-out destination already converted to view coordinates.
    /// Explorer may snap the requested point onto its icon grid.
    pub fn restore_drop_positions(
        &self,
        entries: &[(PathBuf, DesktopPosition)],
    ) -> Result<PlacementReport, String> {
        self.apply_positions(entries, false)
    }

    fn apply_positions(
        &self,
        entries: &[(PathBuf, DesktopPosition)],
        require_exact: bool,
    ) -> Result<PlacementReport, String> {
        let mut report = PlacementReport::default();
        if entries.is_empty() {
            return Ok(report);
        }
        unsafe {
            if self.auto_arrange()? {
                report.auto_arrange = true;
                report.warnings.push(
                    "Windows Auto arrange is enabled; Windows controls Desktop icon positions."
                        .into(),
                );
                return Ok(report);
            }
            let mut seen = HashSet::new();
            let mut pidls = Vec::new();
            let mut points = Vec::new();
            let mut accepted = Vec::new();
            for (path, position) in entries {
                if !position.is_valid() {
                    report
                        .warnings
                        .push("An invalid saved Desktop position was ignored.".into());
                    continue;
                }
                if !seen.insert(path_key(path)) {
                    continue;
                }
                match personal_item(&self.desktop, path) {
                    Ok(Some(path)) => match self.pidl_for(&path) {
                        Ok(pidl) => {
                            // ParseDisplayName can succeed before Explorer has
                            // inserted a newly-created file into its view. Do
                            // not count a no-op positioning call as success.
                            if self.view.GetItemPosition(pidl.0).is_err() {
                                report.pending.push((path, *position));
                                continue;
                            }
                            pidls.push(pidl);
                            points.push(POINT {
                                x: position.x,
                                y: position.y,
                            });
                            accepted.push((path, *position));
                        }
                        Err(_) => report.pending.push((path, *position)),
                    },
                    Ok(None) => report.warnings.push(
                        "A saved icon belongs to another Desktop and was not positioned.".into(),
                    ),
                    Err(_) => report.pending.push((path.clone(), *position)),
                }
            }
            if pidls.is_empty() {
                return Ok(report);
            }
            let children: Vec<_> = pidls
                .iter()
                .map(|pidl| pidl.0 as *const ITEMIDLIST)
                .collect();
            let flags = SVSI_POSITIONITEM.0 as u32 | SVSI_NOTAKEFOCUS.0 as u32 | SVSI_NOSTATECHANGE;
            match self.view.SelectAndPositionItems(
                children.len() as u32,
                children.as_ptr(),
                Some(points.as_ptr()),
                flags,
            ) {
                Ok(()) => {
                    for ((path, position), pidl) in accepted.into_iter().zip(&pidls) {
                        let actual = self.view.GetItemPosition(pidl.0).ok();
                        if placement_observed(position, actual, require_exact) {
                            report.positioned += 1;
                        } else {
                            report.pending.push((path, position));
                        }
                    }
                    if !report.pending.is_empty() {
                        report.warnings.push(
                            "Explorer has not applied all requested Desktop positions yet.".into(),
                        );
                    }
                }
                Err(error) => {
                    report.pending.extend(accepted);
                    report.warnings.push(format!(
                        "Explorer has not applied the saved Desktop positions yet: {error}"
                    ));
                }
            }
        }
        Ok(report)
    }

    pub fn auto_arrange(&self) -> Result<bool, String> {
        // windows 0.58's convenience method erases the S_FALSE distinction.
        let status = unsafe { (Interface::vtable(&self.view).GetAutoArrange)(self.view.as_raw()) };
        auto_arrange_state(status)
    }

    unsafe fn pidl_for(&self, path: &Path) -> Result<Pidl, String> {
        let name: Vec<u16> = path
            .file_name()
            .ok_or("The Desktop item has no filename.")?
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut raw = std::ptr::null_mut();
        self.folder
            .ParseDisplayName(
                None,
                None,
                PCWSTR(name.as_ptr()),
                None,
                &mut raw,
                std::ptr::null_mut(),
            )
            .map_err(|error| format!("The Desktop item is not in Explorer yet: {error}"))?;
        let pidl = Pidl(raw);
        if pidl.0.is_null() {
            return Err("Explorer returned an empty Desktop item identifier.".into());
        }
        // The Desktop folder is the Shell namespace root, so this child PIDL
        // also resolves as absolute. Do not position a shared namesake.
        let actual =
            SHGetNameFromIDList(pidl.0, SIGDN_FILESYSPATH).map_err(|error| error.to_string())?;
        let actual_string = actual.to_string();
        CoTaskMemFree(Some(actual.0.cast()));
        let actual = fs::canonicalize(PathBuf::from(
            actual_string.map_err(|error| error.to_string())?,
        ))
        .map_err(|error| error.to_string())?;
        if path_key(&actual) != path_key(path) {
            return Err(
                "The Desktop namespace resolved a different item with this filename.".into(),
            );
        }
        Ok(pidl)
    }
}

fn placement_observed(expected: DesktopPosition, actual: Option<POINT>, exact: bool) -> bool {
    actual.is_some_and(|point| {
        DesktopPosition {
            x: point.x,
            y: point.y,
        }
        .is_valid()
            && (!exact || (point.x == expected.x && point.y == expected.y))
    })
}

fn auto_arrange_state(status: windows::core::HRESULT) -> Result<bool, String> {
    if status == S_OK {
        Ok(true)
    } else if status == S_FALSE {
        Ok(false)
    } else {
        Err(format!(
            "Explorer's Auto arrange state is unavailable: {status}"
        ))
    }
}

fn personal_item(desktop: &Path, path: &Path) -> Result<Option<PathBuf>, String> {
    let resolved = fs::canonicalize(path)
        .map_err(|error| format!("The Desktop item is not available: {error}"))?;
    Ok((resolved.parent().map(path_key) == Some(path_key(desktop))).then_some(resolved))
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(tail) = value.strip_prefix(r"\\?\unc\") {
        format!(r"\\{tail}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
    }
}

/// Notify after the corresponding successful filesystem move. Asynchronous
/// flush starts delivery promptly without waiting for every Shell extension.
pub fn notify_removed(paths: &[PathBuf]) {
    notify(paths, false);
}
pub fn notify_created(paths: &[PathBuf]) {
    notify(paths, true);
}

fn notify(paths: &[PathBuf], created: bool) {
    let mut seen = HashSet::new();
    for path in paths {
        if !seen.insert(path_key(path)) {
            continue;
        }
        let path = path.to_string_lossy();
        let shell = if let Some(tail) = path.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{tail}")
        } else {
            path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
        };
        if shell.contains('\0') {
            continue;
        }
        let wide: Vec<_> = shell.encode_utf16().chain(Some(0)).collect();
        unsafe {
            SHChangeNotify(
                if created { SHCNE_CREATE } else { SHCNE_DELETE },
                SHCNF_PATHW | SHCNF_FLUSHNOWAIT,
                Some(wide.as_ptr().cast()),
                None,
            );
        }
    }
}

/// Convert a drag destination independently of COM. If a cached Shell view is
/// stale, the caller can queue these coordinates and reconnect on its retry.
pub fn screen_position(mut point: POINT) -> Result<DesktopPosition, String> {
    unsafe {
        let list = crate::platform::desktop_host()
            .and_then(|host| FindWindowExW(host, None, w!("SysListView32"), None).ok())
            .ok_or("The Desktop icon grid is unavailable.")?;
        if !ScreenToClient(list, &mut point).as_bool() {
            return Err("The Desktop drop position could not be converted.".into());
        }
    }
    let position = DesktopPosition {
        x: point.x,
        y: point.y,
    };
    if !position.is_valid() {
        return Err("The Desktop drop position is invalid.".into());
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_arrange_preserves_the_hresult_false_distinction() {
        assert_eq!(auto_arrange_state(S_OK).unwrap(), true);
        assert_eq!(auto_arrange_state(S_FALSE).unwrap(), false);
        assert!(auto_arrange_state(windows::core::HRESULT(0x80004005u32 as i32)).is_err());
    }

    #[test]
    fn placement_requires_an_observed_item_and_exact_saved_coordinates() {
        let saved = DesktopPosition { x: 120, y: 240 };
        let snapped = POINT { x: 116, y: 232 };
        assert!(!placement_observed(saved, None, true));
        assert!(!placement_observed(saved, None, false));
        assert!(!placement_observed(saved, Some(snapped), true));
        assert!(placement_observed(
            saved,
            Some(POINT { x: 120, y: 240 }),
            true
        ));
        assert!(placement_observed(saved, Some(snapped), false));
        assert!(!placement_observed(
            saved,
            Some(POINT { x: i32::MAX, y: 0 }),
            false
        ));
    }

    #[test]
    #[ignore = "Requires an interactive Explorer Desktop; reads positions only"]
    fn native_desktop_capture_read_only() {
        let session = DesktopSession::connect().expect("Connect to the interactive Desktop view");
        let auto_arrange = session
            .auto_arrange()
            .expect("Read Auto arrange without changing it");
        let mut available = 0;
        for item in fs::read_dir(&session.desktop).expect("Read the personal Desktop directory") {
            let Ok(item) = item else {
                continue;
            };
            let path = item.path();
            if !matches!(
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("lnk" | "url")
            ) {
                continue;
            }
            available += 1;
            if let Ok(Some(position)) = session.capture(&path) {
                println!("{{\"connected\":true,\"auto_arrange\":{auto_arrange},\"captured\":true,\"x\":{},\"y\":{}}}", position.x, position.y);
                return;
            }
        }
        panic!("No capturable personal Desktop shortcut was available ({available} candidates); no files or positions were changed.");
    }

    #[test]
    fn path_comparison_handles_shell_prefixes_without_losing_unc_roots() {
        assert_eq!(
            path_key(Path::new(r"\\?\C:\Users\Example\Desktop\App.lnk")),
            path_key(Path::new(r"c:/users/example/desktop/app.lnk"))
        );
        assert_eq!(
            path_key(Path::new(r"\\?\UNC\host\share\App.url")),
            path_key(Path::new(r"\\host\share\app.url"))
        );
        assert_ne!(
            path_key(Path::new(r"C:\Users\Public\Desktop\App.lnk")),
            path_key(Path::new(r"C:\Users\Example\Desktop\App.lnk"))
        );
    }

    #[test]
    fn synthetic_personal_desktop_accepts_only_its_direct_items() {
        let root = std::env::temp_dir().join(crate::model::new_id("desktop-positions-test"));
        fs::create_dir_all(root.join("Desktop")).unwrap();
        fs::create_dir_all(root.join("PublicDesktop")).unwrap();
        fs::create_dir_all(root.join("Desktop").join("Nested")).unwrap();
        let desktop = fs::canonicalize(root.join("Desktop")).unwrap();
        for path in [
            root.join("Desktop/App.lnk"),
            root.join("PublicDesktop/App.lnk"),
            root.join("Desktop/Nested/App.lnk"),
        ] {
            fs::write(path, b"synthetic; not a launch target").unwrap();
        }
        assert!(personal_item(&desktop, &root.join("Desktop/App.lnk"))
            .unwrap()
            .is_some());
        assert!(personal_item(&desktop, &root.join("PublicDesktop/App.lnk"))
            .unwrap()
            .is_none());
        assert!(
            personal_item(&desktop, &root.join("Desktop/Nested/App.lnk"))
                .unwrap()
                .is_none()
        );
        assert!(personal_item(&desktop, &root.join("Desktop/Missing.lnk")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
