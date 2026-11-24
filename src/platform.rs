//! Thin Windows integration for NestDesktop.
//!
//! Desktop attachment uses Explorer's `SHELLDLL_DefView` child window. Explorer
//! does not expose a supported third-party desktop-widget hosting contract, so
//! this is deliberately isolated here. Recreate the tile windows after an
//! Explorer restart (`TaskbarCreated`) and reapply their saved positions.
//! Nothing in this module replaces Explorer, injects code, hides desktop icons,
//! moves shortcuts, or changes the taskbar.

use std::collections::{HashSet, VecDeque};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::HICON;

type RawHandle = *mut c_void;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
struct ShellFileInfo {
    icon: RawHandle,
    icon_index: i32,
    attributes: u32,
    display_name: [u16; 260],
    type_name: [u16; 80],
}

#[repr(C)]
struct OpenFileName {
    struct_size: u32,
    owner: RawHandle,
    instance: RawHandle,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    default_extension: *const u16,
    custom_data: isize,
    hook: *const c_void,
    template_name: *const u16,
    reserved: *mut c_void,
    reserved_dw: u32,
    flags_ex: u32,
}

#[link(name = "user32")]
extern "system" {
    fn SystemParametersInfoW(action: u32, parameter: u32, value: *mut c_void, flags: u32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn FindWindowW(class_name: *const u16, window_name: *const u16) -> RawHandle;
    fn FindWindowExW(
        parent: RawHandle,
        after: RawHandle,
        class_name: *const u16,
        window_name: *const u16,
    ) -> RawHandle;
    fn EnumWindows(
        callback: unsafe extern "system" fn(RawHandle, isize) -> i32,
        parameter: isize,
    ) -> i32;
    fn SetParent(child: RawHandle, parent: RawHandle) -> RawHandle;
    fn GetWindowLongPtrW(window: RawHandle, index: i32) -> isize;
    fn SetWindowLongPtrW(window: RawHandle, index: i32, value: isize) -> isize;
    fn SetWindowPos(
        window: RawHandle,
        after: RawHandle,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn ScreenToClient(window: RawHandle, point: *mut POINT) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        owner: RawHandle,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> RawHandle;
    fn SHGetFileInfoW(
        path: *const u16,
        file_attributes: u32,
        info: *mut ShellFileInfo,
        info_size: u32,
        flags: u32,
    ) -> usize;
    fn SHGetKnownFolderPath(
        folder: *const Guid,
        flags: u32,
        token: RawHandle,
        path: *mut *mut u16,
    ) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemFree(memory: *const c_void);
}

#[link(name = "comdlg32")]
extern "system" {
    fn GetOpenFileNameW(dialog: *mut OpenFileName) -> i32;
    fn GetSaveFileNameW(dialog: *mut OpenFileName) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        key: RawHandle,
        subkey: *const u16,
        reserved: u32,
        class: *mut u16,
        options: u32,
        access: u32,
        security: *const c_void,
        result: *mut RawHandle,
        disposition: *mut u32,
    ) -> i32;
    fn RegOpenKeyExW(
        key: RawHandle,
        subkey: *const u16,
        options: u32,
        access: u32,
        result: *mut RawHandle,
    ) -> i32;
    fn RegSetValueExW(
        key: RawHandle,
        name: *const u16,
        reserved: u32,
        value_type: u32,
        data: *const u8,
        data_size: u32,
    ) -> i32;
    fn RegDeleteValueW(key: RawHandle, name: *const u16) -> i32;
    fn RegQueryValueExW(
        key: RawHandle,
        name: *const u16,
        reserved: *mut u32,
        value_type: *mut u32,
        data: *mut u8,
        data_size: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: RawHandle) -> i32;
}

/// Work area of the primary display, in physical screen coordinates.
/// The process must opt into PerMonitorV2 DPI awareness before creating windows.
pub fn primary_work_area() -> RECT {
    unsafe {
        let mut area = RECT::default();
        if SystemParametersInfoW(0x0030, 0, &mut area as *mut RECT as *mut c_void, 0) == 0 {
            area = RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(0),
                bottom: GetSystemMetrics(1),
            };
        }
        area
    }
}

unsafe extern "system" fn find_desktop_view(top: RawHandle, parameter: isize) -> i32 {
    let class = wide("SHELLDLL_DefView");
    let view = FindWindowExW(top, std::ptr::null_mut(), class.as_ptr(), std::ptr::null());
    if !view.is_null() {
        *(parameter as *mut RawHandle) = view;
        0
    } else {
        1
    }
}

/// Find the actual Explorer desktop view, whether currently under Progman or a
/// WorkerW. We do not send Explorer's undocumented "create WorkerW" message.
pub unsafe fn desktop_host() -> Option<HWND> {
    let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
    if !progman.is_null() {
        let view = FindWindowExW(
            progman,
            std::ptr::null_mut(),
            wide("SHELLDLL_DefView").as_ptr(),
            std::ptr::null(),
        );
        if !view.is_null() {
            return Some(HWND(view));
        }
    }
    let mut found: RawHandle = std::ptr::null_mut();
    EnumWindows(find_desktop_view, &mut found as *mut RawHandle as isize);
    if found.is_null() {
        None
    } else {
        Some(HWND(found))
    }
}

/// Convert a physical screen position into the Explorer desktop view's client
/// coordinates. Needed when secondary monitors extend left or above primary.
pub unsafe fn desktop_coordinates(screen_x: i32, screen_y: i32) -> (i32, i32) {
    let mut point = POINT {
        x: screen_x,
        y: screen_y,
    };
    if let Some(host) = desktop_host() {
        ScreenToClient(host.0, &mut point);
    }
    (point.x, point.y)
}

/// Make a small tile a desktop child, above the ordinary desktop icon list.
///
/// Set positions after this call: child coordinates are relative to the desktop
/// view. This deliberately returns a clear failure instead of creating an
/// always-on-top window that would cover the user's applications.
pub unsafe fn attach_to_desktop(window: HWND) -> Result<(), String> {
    let host = desktop_host().ok_or_else(|| {
        "Windows Explorer's desktop is not available. Try again after Explorer has started."
            .to_string()
    })?;
    let old_style = GetWindowLongPtrW(window.0, -16);
    let new_style = (old_style & !(0x8000_0000u32 as isize)) | 0x4000_0000 | 0x0400_0000;
    SetLastError(0);
    if SetWindowLongPtrW(window.0, -16, new_style) == 0 && GetLastError() != 0 {
        return Err(format!(
            "Unable to prepare desktop tile (Windows error {}).",
            GetLastError()
        ));
    }
    SetLastError(0);
    let previous = SetParent(window.0, host.0);
    let error = GetLastError();
    if previous.is_null() && error != 0 {
        SetWindowLongPtrW(window.0, -16, old_style);
        return Err(format!(
            "Unable to attach tile to the Windows desktop (Windows error {error})."
        ));
    }
    // HWND_TOP; preserve size/position, do not activate, and refresh the frame.
    if SetWindowPos(
        window.0,
        std::ptr::null_mut(),
        0,
        0,
        0,
        0,
        0x0001 | 0x0002 | 0x0010 | 0x0020,
    ) == 0
    {
        return Err(format!(
            "Unable to place desktop tile (Windows error {}).",
            GetLastError()
        ));
    }
    Ok(())
}

/// Launch through the Windows shell so shortcuts and Store-app shortcuts retain
/// their normal activation behaviour. Requires no elevated privileges.
pub unsafe fn launch(target: &str, arguments: &str) -> Result<(), String> {
    if target.trim().is_empty() || target.contains('\0') || arguments.contains('\0') {
        return Err("This item has no valid application or shortcut target.".to_string());
    }
    let operation = wide("open");
    let file = wide(target);
    let args = wide(arguments);
    let result = ShellExecuteW(
        std::ptr::null_mut(),
        operation.as_ptr(),
        file.as_ptr(),
        if arguments.is_empty() {
            std::ptr::null()
        } else {
            args.as_ptr()
        },
        std::ptr::null(),
        1,
    ) as isize;
    if result <= 32 {
        let explanation = match result {
            2 | 3 => "The application or shortcut could not be found.",
            5 => "Windows denied access to this application.",
            8 => "Windows does not have enough memory to open this item.",
            26 => "Another application is currently using this item.",
            31 => "Windows has no application associated with this file type.",
            _ => "Windows could not open this item.",
        };
        Err(format!("{explanation} (Shell error {result})"))
    } else {
        Ok(())
    }
}

pub unsafe fn open_folder(path: &Path) -> Result<(), String> {
    launch(&path.to_string_lossy(), "")
}

/// Returns an owned icon. The caller must eventually call DestroyIcon; retain a
/// bounded cache rather than looking up icons during every paint.
pub unsafe fn icon_for_path(path: &str, size: i32) -> Option<HICON> {
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    let mut info: ShellFileInfo = std::mem::zeroed();
    let flags = 0x0000_0100 | if size <= 20 { 0x0000_0001 } else { 0 };
    let result = SHGetFileInfoW(
        wide(path).as_ptr(),
        0,
        &mut info,
        std::mem::size_of::<ShellFileInfo>() as u32,
        flags,
    );
    if result == 0 || info.icon.is_null() {
        None
    } else {
        Some(HICON(info.icon))
    }
}

const DESKTOP: Guid = Guid {
    data1: 0xb4bfcc3a,
    data2: 0xdb2c,
    data3: 0x424c,
    data4: [0xb0, 0x29, 0x7f, 0xe9, 0x9a, 0x87, 0xc6, 0x41],
};
const PUBLIC_DESKTOP: Guid = Guid {
    data1: 0xc4aa340d,
    data2: 0xf20f,
    data3: 0x4863,
    data4: [0xaf, 0xef, 0xf8, 0x7e, 0xf2, 0xe6, 0xba, 0x25],
};
const PROGRAMS: Guid = Guid {
    data1: 0xa77f5d77,
    data2: 0x2e2b,
    data3: 0x44c3,
    data4: [0xa6, 0xa2, 0xab, 0xa6, 0x01, 0x05, 0x4a, 0x51],
};
const COMMON_PROGRAMS: Guid = Guid {
    data1: 0x0139d44e,
    data2: 0x6afe,
    data3: 0x49f2,
    data4: [0x86, 0x90, 0x3d, 0xaf, 0xca, 0xe6, 0xff, 0xb8],
};

fn known_folder(folder: &Guid) -> Option<PathBuf> {
    unsafe {
        let mut value: *mut u16 = std::ptr::null_mut();
        if SHGetKnownFolderPath(folder, 0, std::ptr::null_mut(), &mut value) < 0 || value.is_null()
        {
            return None;
        }
        let mut length = 0;
        while length < 32768 && *value.add(length) != 0 {
            length += 1;
        }
        let result = if length < 32768 {
            Some(PathBuf::from(String::from_utf16_lossy(
                std::slice::from_raw_parts(value, length),
            )))
        } else {
            None
        };
        CoTaskMemFree(value as *const c_void);
        result
    }
}

pub fn desktop_directory() -> PathBuf {
    known_folder(&DESKTOP).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Desktop")
    })
}

pub fn public_desktop_directory() -> Option<PathBuf> {
    known_folder(&PUBLIC_DESKTOP).or_else(|| {
        std::env::var_os("PUBLIC")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join("Desktop"))
    })
}

/// Display-only source hints. Resolve the four known folders once per picker
/// refresh; the catalogue names and targets remain unchanged for search/add.
pub fn catalogue_labels(catalogue: &[(String, String)]) -> Vec<String> {
    let mut origins = Vec::new();
    if let Some(path) = public_desktop_directory() {
        origins.push((path, "Shared desktop"));
    }
    origins.push((desktop_directory(), "Desktop"));
    if let Some(path) = known_folder(&PROGRAMS) {
        origins.push((path, "Your Start menu"));
    }
    if let Some(path) = known_folder(&COMMON_PROGRAMS) {
        origins.push((path, "Shared Start menu"));
    }
    catalogue_labels_from_roots(catalogue, &origins)
}

fn catalogue_path_key(path: &Path) -> String {
    let path = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(tail) = path.strip_prefix(r"\\?\unc\") {
        format!(r"\\{tail}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

fn catalogue_labels_from_roots(
    catalogue: &[(String, String)],
    origins: &[(PathBuf, &str)],
) -> Vec<String> {
    let mut counts = std::collections::HashMap::new();
    for (name, _) in catalogue {
        *counts.entry(name.to_lowercase()).or_insert(0usize) += 1;
    }
    let roots: Vec<_> = origins
        .iter()
        .map(|(path, hint)| {
            (
                format!("{}\\", catalogue_path_key(path).trim_end_matches('\\')),
                *hint,
            )
        })
        .collect();
    catalogue
        .iter()
        .map(|(name, path)| {
            let key = catalogue_path_key(Path::new(path));
            let origin = roots
                .iter()
                .find(|(root, _)| key.starts_with(root))
                .map(|(_, hint)| *hint);
            if origin == Some("Shared desktop") || counts[&name.to_lowercase()] > 1 {
                let fallback = Path::new(path)
                    .parent()
                    .and_then(Path::file_name)
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Other folder".into());
                format!("{name} · {}", origin.unwrap_or(&fallback))
            } else {
                name.clone()
            }
        })
        .collect()
}

/// Read shortcut names and paths, never launch them or change the desktop.
/// Traversal is bounded, and reparse directories are excluded.
pub fn discover_shortcuts() -> Vec<(String, String)> {
    let mut roots = vec![desktop_directory()];
    if let Some(path) = public_desktop_directory() {
        roots.push(path);
    }
    for id in [&PROGRAMS, &COMMON_PROGRAMS] {
        if let Some(path) = known_folder(id) {
            roots.push(path);
        }
    }
    discover_from_roots(&roots, 5000)
}

struct DiscoveryRoot {
    pending: VecDeque<(PathBuf, usize)>,
    current: Option<(std::fs::ReadDir, usize)>,
}

impl DiscoveryRoot {
    fn new(path: PathBuf) -> Self {
        Self {
            pending: VecDeque::from([(path, 0)]),
            current: None,
        }
    }

    fn next_entry(&mut self) -> Option<(std::io::Result<std::fs::DirEntry>, usize)> {
        loop {
            if let Some((entries, depth)) = self.current.as_mut() {
                if let Some(entry) = entries.next() {
                    return Some((entry, *depth));
                }
            }
            self.current = None;
            let (directory, depth) = self.pending.pop_front()?;
            if let Ok(entries) = std::fs::read_dir(directory) {
                self.current = Some((entries, depth));
            }
        }
    }
}

/// Interleave one entry per root, breadth-first within each root. A busy
/// personal Desktop cannot consume the whole budget before Public Desktop or
/// Start menu shortcuts are visited. Directory-entry errors also cost budget.
fn discover_from_roots(roots: &[PathBuf], budget: usize) -> Vec<(String, String)> {
    use std::os::windows::fs::MetadataExt;
    let mut scans: Vec<_> = roots.iter().cloned().map(DiscoveryRoot::new).collect();
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    let mut visited = 0usize;
    let budget = budget.min(5000);
    while visited < budget && results.len() < 500 {
        let mut progressed = false;
        for scan in &mut scans {
            if visited >= budget || results.len() >= 500 {
                break;
            }
            let Some((entry, depth)) = scan.next_entry() else {
                continue;
            };
            progressed = true;
            visited += 1;
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if depth < 6
                    && entry
                        .metadata()
                        .map(|info| info.file_attributes() & 0x400 == 0)
                        .unwrap_or(false)
                {
                    scan.pending.push_back((path, depth + 1));
                }
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(extension.as_str(), "lnk" | "url" | "exe" | "appref-ms") {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            let key = name.to_lowercase();
            if key.contains("uninstall")
                || key.contains("deinstall")
                || key.contains("readme")
                || key.contains("read me")
            {
                continue;
            }
            // Keep distinct same-name paths for conservative imported relinking.
            let target = path.to_string_lossy().into_owned();
            if seen.insert(target.replace('/', "\\").to_lowercase()) {
                results.push((name, target));
            }
        }
        if !progressed {
            break;
        }
    }
    results.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    results
}

/// Standard Windows picker. Cancel returns an empty vector. The caller handles
/// collection or references; this dialog itself does not move or copy files.
pub unsafe fn choose_files(owner: HWND) -> Vec<String> {
    choose_files_impl(owner, false)
}

pub unsafe fn choose_desktop_shortcuts(owner: HWND) -> Vec<String> {
    choose_files_impl(owner, true)
}

unsafe fn choose_files_impl(owner: HWND, collect: bool) -> Vec<String> {
    let mut buffer = vec![0u16; 32768];
    let filter: Vec<u16> = if collect {
        "Desktop shortcuts\0*.lnk;*.url\0\0"
    } else {
        "Applications and shortcuts\0*.lnk;*.exe;*.url;*.appref-ms\0All files\0*.*\0\0"
    }
    .encode_utf16()
    .collect();
    let title = wide(if collect {
        "Move desktop shortcuts into this app folder — restore anytime"
    } else {
        "Add applications or shortcuts to this folder"
    });
    let initial = wide(&desktop_directory().to_string_lossy());
    let mut dialog: OpenFileName = std::mem::zeroed();
    dialog.struct_size = std::mem::size_of::<OpenFileName>() as u32;
    dialog.owner = owner.0;
    dialog.filter = filter.as_ptr();
    dialog.filter_index = 1;
    dialog.file = buffer.as_mut_ptr();
    dialog.max_file = buffer.len() as u32;
    dialog.initial_dir = initial.as_ptr();
    dialog.title = title.as_ptr();
    dialog.flags =
        0x0008_0000 | 0x0000_0200 | 0x0000_1000 | 0x0000_0800 | 0x0000_0008 | 0x0200_0000;
    if GetOpenFileNameW(&mut dialog) == 0 {
        return Vec::new();
    }
    let parts: Vec<String> = buffer
        .split(|v| *v == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    if parts.len() == 1 {
        return parts;
    }
    let Some(base) = parts.first() else {
        return Vec::new();
    };
    parts[1..]
        .iter()
        .map(|name| Path::new(base).join(name).to_string_lossy().into_owned())
        .collect()
}

/// A standard JSON import/export picker. The exporter receives the confirmed
/// path and writes it; the picker itself never creates or overwrites a file.
pub unsafe fn choose_layout_path(owner: HWND, save: bool) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    if save {
        let suggested = wide("NestDesktop-layout.json");
        buffer[..suggested.len()].copy_from_slice(&suggested);
    }
    let filter: Vec<u16> = "NestDesktop layout (*.json)\0*.json\0\0"
        .encode_utf16()
        .collect();
    let title = wide(if save {
        "Export desktop folders"
    } else {
        "Import desktop folders"
    });
    let extension = wide("json");
    let mut dialog: OpenFileName = std::mem::zeroed();
    dialog.struct_size = std::mem::size_of::<OpenFileName>() as u32;
    dialog.owner = owner.0;
    dialog.filter = filter.as_ptr();
    dialog.filter_index = 1;
    dialog.file = buffer.as_mut_ptr();
    dialog.max_file = buffer.len() as u32;
    dialog.title = title.as_ptr();
    dialog.default_extension = extension.as_ptr();
    // Explorer, no working-directory changes, existing directory, no recents.
    dialog.flags = 0x0008_0000 | 0x0000_0008 | 0x0000_0800 | 0x0200_0000;
    let result = if save {
        dialog.flags |= 0x0000_0002; // Confirm replacement of an existing export.
        GetSaveFileNameW(&mut dialog)
    } else {
        dialog.flags |= 0x0000_1000;
        GetOpenFileNameW(&mut dialog)
    };
    if result == 0 {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0)?;
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
}

pub fn startup_enabled() -> bool {
    unsafe {
        let current_user = (0x8000_0001u32 as i32 as isize) as RawHandle;
        let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let name = wide("NestDesktop");
        let mut key = std::ptr::null_mut();
        if RegOpenKeyExW(current_user, subkey.as_ptr(), 0, 0x0001, &mut key) != 0 {
            return false;
        }
        let mut value_type = 0;
        let mut size = 0;
        let status = RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut size,
        );
        RegCloseKey(key);
        status == 0 && matches!(value_type, 1 | 2) && size > 2
    }
}

/// Opt-in sign-in startup for the current user. No service, task scheduler,
/// elevation, or machine-wide registry entries are needed.
pub fn set_startup(executable: &Path, enabled: bool) -> Result<(), String> {
    unsafe {
        let current_user = (0x8000_0001u32 as i32 as isize) as RawHandle;
        let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let name = wide("NestDesktop");
        let mut key = std::ptr::null_mut();
        if enabled {
            if !executable.is_absolute() || !executable.is_file() {
                return Err(
                    "The startup application must be an existing absolute executable path."
                        .to_string(),
                );
            }
            let status = RegCreateKeyExW(
                current_user,
                subkey.as_ptr(),
                0,
                std::ptr::null_mut(),
                0,
                0x0002,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            );
            if status != 0 {
                return Err(format!(
                    "Unable to enable sign-in startup (Windows error {status})."
                ));
            }
            let command = wide(&format!(
                "\"{}\" --background",
                executable.to_string_lossy()
            ));
            let status = RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                1,
                command.as_ptr() as *const u8,
                (command.len() * 2) as u32,
            );
            RegCloseKey(key);
            if status != 0 {
                return Err(format!(
                    "Unable to save sign-in startup (Windows error {status})."
                ));
            }
        } else {
            let status = RegOpenKeyExW(current_user, subkey.as_ptr(), 0, 0x0002, &mut key);
            if status == 2 {
                return Ok(());
            }
            if status != 0 {
                return Err(format!(
                    "Unable to change sign-in startup (Windows error {status})."
                ));
            }
            let status = RegDeleteValueW(key, name.as_ptr());
            RegCloseKey(key);
            if status != 0 && status != 2 {
                return Err(format!(
                    "Unable to disable sign-in startup (Windows error {status})."
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use std::fs;

    #[test]
    fn picker_labels_distinguish_shared_desktop_from_same_name_start_menu() {
        let origins = vec![
            (PathBuf::from(r"C:\Users\Public\Desktop"), "Shared desktop"),
            (PathBuf::from(r"C:\Users\Example\Desktop"), "Desktop"),
            (
                PathBuf::from(r"C:\ProgramData\Programs"),
                "Shared Start menu",
            ),
        ];
        let catalogue = vec![
            (
                "Adobe Acrobat".into(),
                r"\\?\c:\users\PUBLIC\desktop\Adobe Acrobat.lnk".into(),
            ),
            (
                "ADOBE ACROBAT".into(),
                r"C:\ProgramData\Programs\Adobe\Acrobat.lnk".into(),
            ),
            (
                "Battle.net".into(),
                r"C:\Users\Public\Desktop\Battle.net.lnk".into(),
            ),
        ];
        let before = catalogue.clone();
        assert_eq!(
            catalogue_labels_from_roots(&catalogue, &origins),
            vec![
                "Adobe Acrobat · Shared desktop",
                "ADOBE ACROBAT · Shared Start menu",
                "Battle.net · Shared desktop",
            ]
        );
        assert_eq!(catalogue, before);
    }

    #[test]
    fn picker_labels_use_parent_for_other_duplicates_and_require_root_boundary() {
        let origins = vec![(PathBuf::from(r"C:\Users\Public\Desktop"), "Shared desktop")];
        let catalogue = vec![
            ("Example".into(), r"D:\Tools\First\Example.exe".into()),
            ("Example".into(), r"D:\Tools\Second\Example.exe".into()),
            (
                "Unique".into(),
                r"C:\Users\Example\Desktop\Unique.lnk".into(),
            ),
            (
                "Unrelated".into(),
                r"C:\Users\Public\DesktopOther\Unrelated.lnk".into(),
            ),
        ];
        assert_eq!(
            catalogue_labels_from_roots(&catalogue, &origins),
            vec!["Example · First", "Example · Second", "Unique", "Unrelated",]
        );
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let parent = std::env::current_dir().unwrap().join("target");
            fs::create_dir_all(&parent).unwrap();
            let root = parent.join(crate::model::new_id("shortcut-discovery-test"));
            fs::create_dir(&root).unwrap();
            Self(fs::canonicalize(root).unwrap())
        }
        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"Synthetic; never launched").unwrap();
            path
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let expected_parent =
                fs::canonicalize(std::env::current_dir().unwrap().join("target")).unwrap();
            assert_eq!(self.0.parent(), Some(expected_parent.as_path()));
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("shortcut-discovery-test"));
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn busy_desktop_cannot_starve_public_or_nested_start_menu_shortcuts() {
        let scratch = Scratch::new();
        for index in 0..24 {
            scratch.file(&format!("Desktop/Photos/photo-{index}.txt"));
        }
        let acrobat = scratch.file("PublicDesktop/Adobe Acrobat.lnk");
        let calculator = scratch.file("StartMenu/Vendor/Calculator.lnk");
        let found = discover_from_roots(
            &[
                scratch.0.join("Desktop"),
                scratch.0.join("PublicDesktop"),
                scratch.0.join("StartMenu"),
            ],
            6,
        );
        let targets: HashSet<_> = found
            .into_iter()
            .map(|(_, path)| PathBuf::from(path))
            .collect();
        assert_eq!(targets, HashSet::from([acrobat, calculator]));
    }

    #[test]
    fn discovery_budget_is_global_and_zero_budget_does_not_enumerate() {
        let scratch = Scratch::new();
        let roots: Vec<_> = ["First", "Second", "Third"]
            .into_iter()
            .map(|name| scratch.0.join(name))
            .collect();
        for root in &roots {
            for index in 0..8 {
                scratch.file(&format!(
                    "{}/App {index}.lnk",
                    root.file_name().unwrap().to_string_lossy()
                ));
            }
        }
        assert!(discover_from_roots(&roots, 0).is_empty());
        assert_eq!(discover_from_roots(&roots, 5).len(), 5);
    }

    #[test]
    fn duplicate_names_keep_distinct_paths_and_existing_filters() {
        let scratch = Scratch::new();
        let personal = scratch.file("Desktop/Example.lnk");
        let shared = scratch.file("PublicDesktop/Example.lnk");
        let website = scratch.file("StartMenu/Website.URL");
        scratch.file("StartMenu/Uninstall Example.lnk");
        scratch.file("StartMenu/Read me.url");
        scratch.file("Desktop/Document.txt");
        let found = discover_from_roots(
            &[
                scratch.0.join("Desktop"),
                scratch.0.join("PublicDesktop"),
                scratch.0.join("StartMenu"),
            ],
            50,
        );
        assert_eq!(
            found.iter().filter(|(name, _)| name == "Example").count(),
            2
        );
        let targets: HashSet<_> = found
            .into_iter()
            .map(|(_, path)| PathBuf::from(path))
            .collect();
        assert_eq!(targets, HashSet::from([personal, shared, website]));
    }
}
