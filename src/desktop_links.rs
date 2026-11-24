//! References dragged out of a group become ordinary Desktop shortcuts. Their
//! application, document, or Start menu source is never moved or modified.

use crate::model::{self, Item};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Foundation::{HWND, POINT},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        UI::{
            Shell::{IShellLinkW, ShellLink},
            WindowsAndMessaging::{
                GetAncestor, GetClassNameW, GetParent, GetShellWindow, GetWindowThreadProcessId,
                WindowFromPoint, GA_ROOT,
            },
        },
    },
};

/// `desktop` is the personal Desktop directory, and `staging` must be an
/// app-owned directory. The caller removes the group entry only after this
/// succeeds and the updated layout has been saved.
pub unsafe fn create_reference(
    item: &Item,
    desktop: &Path,
    staging: &Path,
) -> Result<PathBuf, String> {
    if item.target.trim().is_empty()
        || item
            .target
            .chars()
            .any(|c| c == '\0' || c == '\r' || c == '\n')
        || item.arguments.contains('\0')
    {
        return Err("This item has no valid shortcut target.".into());
    }
    if !desktop.is_dir() {
        return Err("The personal Desktop folder is not available.".into());
    }
    let name = file_stem(&item.name);
    let source = Path::new(&item.target);
    if item.arguments.is_empty() {
        let lower = item.target.to_ascii_lowercase();
        if lower.starts_with("https://") || lower.starts_with("http://") {
            // The INI reader used by Internet shortcuts understands UTF-16
            // with a BOM, including non-ASCII URLs.
            let content = format!("[InternetShortcut]\r\nURL={}\r\n", item.target);
            let mut bytes = vec![0xff, 0xfe];
            bytes.extend(content.encode_utf16().flat_map(u16::to_le_bytes));
            return write_reference(desktop, &name, "url", |output| output.write_all(&bytes));
        }
        if let Some(extension) = source.extension().and_then(|ext| ext.to_str()) {
            if source.is_file()
                && (extension.eq_ignore_ascii_case("lnk") || extension.eq_ignore_ascii_case("url"))
            {
                return copy_reference(source, desktop, &name, &extension.to_ascii_lowercase());
            }
        }
    }

    let _com = Apartment::new()?;
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
        .map_err(|error| format!("Windows could not create this shortcut: {error}"))?;
    let (target, arguments) = link_target(item)?;
    let target = wide(&target);
    let arguments = wide(&arguments);
    link.SetPath(PCWSTR(target.as_ptr()))
        .map_err(|error| format!("Windows could not set the shortcut target: {error}"))?;
    link.SetArguments(PCWSTR(arguments.as_ptr()))
        .map_err(|error| format!("Windows could not set the shortcut arguments: {error}"))?;
    let persist = link
        .cast::<IPersistFile>()
        .map_err(|error| format!("Windows could not save this shortcut: {error}"))?;
    let temporary = StagedLink::new(staging)?;
    let path = wide_path(&temporary.file);
    persist
        .Save(PCWSTR(path.as_ptr()), true)
        .map_err(|error| format!("Windows could not save this shortcut: {error}"))?;
    // IPersistFile is allowed to replace its destination, so it only writes
    // inside a newly-created app-owned directory. Desktop publication uses
    // create_new even if another program creates the same name concurrently.
    copy_reference(&temporary.file, desktop, &name, "lnk")
}

fn copy_reference(
    source: &Path,
    desktop: &Path,
    name: &str,
    extension: &str,
) -> Result<PathBuf, String> {
    let mut input = File::open(source)
        .map_err(|error| format!("The source shortcut could not be read: {error}"))?;
    if !input
        .metadata()
        .map_err(|error| format!("The source shortcut could not be checked: {error}"))?
        .is_file()
    {
        return Err("The source shortcut is not a file.".into());
    }
    write_reference(desktop, name, extension, |output| {
        io::copy(&mut input, output).map(|_| ())
    })
}

fn write_reference(
    desktop: &Path,
    name: &str,
    extension: &str,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<PathBuf, String> {
    for number in 1..=10_000 {
        let filename = if number == 1 {
            format!("{name}.{extension}")
        } else {
            format!("{name} ({number}).{extension}")
        };
        let path = desktop.join(filename);
        let mut output = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists || path.exists() => continue,
            Err(error) => {
                return Err(format!(
                    "The Desktop shortcut could not be created: {error}"
                ))
            }
        };
        let result = write(&mut output).and_then(|_| output.sync_all());
        drop(output);
        return match result {
            Ok(()) => Ok(path),
            Err(error) => {
                let _ = fs::remove_file(&path);
                Err(format!(
                    "The Desktop shortcut could not be written: {error}"
                ))
            }
        };
    }
    Err("There are too many Desktop shortcuts with this name.".into())
}

/// A bounded, single Windows path component; suffix space remains for collision
/// numbers and the extension, and a UTF-16 surrogate pair is never split.
fn file_stem(name: &str) -> String {
    let name = name.trim();
    let name = if name.to_ascii_lowercase().ends_with(".lnk")
        || name.to_ascii_lowercase().ends_with(".url")
    {
        &name[..name.len() - 4]
    } else {
        name
    };
    let mut cleaned = String::new();
    let mut length = 0;
    for character in name.chars() {
        let character = if character.is_control() || "<>:\"/\\|?*".contains(character) {
            '_'
        } else {
            character
        };
        length += character.len_utf16();
        if length > 160 {
            break;
        }
        cleaned.push(character);
    }
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);
    if cleaned.is_empty() {
        return "Shortcut".into();
    }
    let device = cleaned
        .split('.')
        .next()
        .unwrap_or(cleaned)
        .trim_end()
        .to_uppercase();
    let reserved = matches!(
        device.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        device.strip_prefix(prefix).is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned.into()
    }
}

unsafe fn link_target(item: &Item) -> Result<(String, String), String> {
    if item
        .target
        .to_ascii_lowercase()
        .starts_with("shell:appsfolder")
    {
        let mut directory = vec![0u16; 32768];
        let length = GetWindowsDirectoryW(directory.as_mut_ptr(), directory.len() as u32) as usize;
        if length == 0 || length >= directory.len() {
            return Err("The Windows folder could not be found.".into());
        }
        let explorer = PathBuf::from(String::from_utf16_lossy(&directory[..length]))
            .join("explorer.exe")
            .to_string_lossy()
            .into_owned();
        let mut arguments = quote_argument(&item.target);
        if !item.arguments.is_empty() {
            arguments.push(' ');
            arguments.push_str(&item.arguments);
        }
        Ok((explorer, arguments))
    } else {
        Ok((shell_path(&item.target), item.arguments.clone()))
    }
}

fn quote_argument(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
        } else {
            quoted.extend(std::iter::repeat_n(
                '\\',
                slashes * if character == '"' { 2 } else { 1 },
            ));
            if character == '"' {
                quoted.push('\\');
            }
            quoted.push(character);
            slashes = 0;
        }
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

struct StagedLink {
    directory: PathBuf,
    file: PathBuf,
}

impl StagedLink {
    fn new(staging: &Path) -> Result<Self, String> {
        fs::create_dir_all(staging)
            .map_err(|error| format!("Shortcut staging is not available: {error}"))?;
        let directory = staging.join(model::new_id("desktop-link"));
        fs::create_dir(&directory)
            .map_err(|error| format!("Shortcut staging could not be created: {error}"))?;
        Ok(Self {
            file: directory.join("reference.lnk"),
            directory,
        })
    }
}

impl Drop for StagedLink {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
        let _ = fs::remove_dir(&self.directory);
    }
}

struct Apartment;

impl Apartment {
    unsafe fn new() -> Result<Self, String> {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|error| format!("The Windows Shell interface is not available: {error}"))?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

#[cfg(test)]
struct Pidl(*mut windows::Win32::UI::Shell::Common::ITEMIDLIST);

#[cfg(test)]
impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(self.0.cast())) }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn shell_path(value: &str) -> String {
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(value).into()
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    // Shell APIs do not consistently accept extended-length path prefixes.
    wide(&shell_path(&path.to_string_lossy()))
}

unsafe fn window_class(window: HWND) -> String {
    let mut buffer = [0u16; 128];
    let length = GetClassNameW(window, &mut buffer).max(0) as usize;
    String::from_utf16_lossy(&buffer[..length])
}

unsafe fn desktop_surface(window: HWND) -> bool {
    let mut root = window;
    match window_class(root).as_str() {
        "SysListView32" => {
            let Ok(parent) = GetParent(root) else {
                return false;
            };
            if window_class(parent) != "SHELLDLL_DefView" {
                return false;
            }
            root = match GetParent(parent) {
                Ok(parent) => parent,
                Err(_) => return false,
            };
        }
        "SHELLDLL_DefView" => {
            root = match GetParent(root) {
                Ok(parent) => parent,
                Err(_) => return false,
            };
        }
        "Progman" | "WorkerW" => (),
        _ => return false,
    }
    if !matches!(window_class(root).as_str(), "Progman" | "WorkerW")
        || GetAncestor(root, GA_ROOT) != root
    {
        return false;
    }
    let shell = GetShellWindow();
    if shell.is_invalid() {
        return false;
    }
    let mut shell_pid = 0;
    let mut root_pid = 0;
    GetWindowThreadProcessId(shell, Some(&mut shell_pid));
    GetWindowThreadProcessId(root, Some(&mut root_pid));
    shell_pid != 0 && shell_pid == root_pid
}

/// A File Explorer folder also contains SysListView32/SHELLDLL_DefView. Only
/// the Shell process's top-level Progman/WorkerW ancestry denotes the Desktop.
pub unsafe fn point_is_desktop(point: POINT) -> bool {
    desktop_surface(WindowFromPoint(point))
}

#[link(name = "kernel32")]
extern "system" {
    fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{
        System::Com::{CoTaskMemFree, STGM_READ},
        UI::Shell::{SHGetNameFromIDList, SIGDN_FILESYSPATH},
    };

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(model::new_id("nest-desktop-links-test"));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("Desktop")).unwrap();
            Self(root)
        }
        fn desktop(&self) -> PathBuf {
            self.0.join("Desktop")
        }
        fn staging(&self) -> PathBuf {
            self.0.join("staging")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            // Each test owns only this freshly-created synthetic directory.
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn item(name: &str, target: &Path, arguments: &str) -> Item {
        Item {
            id: model::new_id("test"),
            name: name.into(),
            target: target.to_string_lossy().into(),
            arguments: arguments.into(),
            children: Vec::new(),
        }
    }

    #[test]
    fn shortcut_copy_never_overwrites_or_changes_source() {
        let scratch = Scratch::new();
        let source = scratch.0.join("source.LNK");
        fs::write(&source, b"opaque existing shortcut bytes").unwrap();
        fs::write(scratch.desktop().join("Test.lnk"), b"keep existing").unwrap();
        fs::create_dir(scratch.desktop().join("Test (2).lnk")).unwrap();
        let entry = item("Test", &source, "");
        let result =
            unsafe { create_reference(&entry, &scratch.desktop(), &scratch.staging()) }.unwrap();
        assert_eq!(result.file_name().unwrap(), "Test (3).lnk");
        assert_eq!(fs::read(result).unwrap(), b"opaque existing shortcut bytes");
        assert_eq!(fs::read(source).unwrap(), b"opaque existing shortcut bytes");
        assert_eq!(
            fs::read(scratch.desktop().join("Test.lnk")).unwrap(),
            b"keep existing"
        );
    }

    #[test]
    fn filenames_are_valid_bounded_and_not_devices() {
        assert_eq!(file_stem(" <Test>:/\\|?*\n. "), "_Test________");
        assert_eq!(file_stem(" .. "), "Shortcut");
        assert_eq!(file_stem("Foo.LNK"), "Foo");
        for name in ["NUL", "con.txt", "COM1", "COM¹", "LPT9", "CONOUT$"] {
            assert!(file_stem(name).starts_with('_'));
        }
        assert_eq!(file_stem("COM10"), "COM10");
        let long = file_stem(&"😀".repeat(100));
        assert_eq!(long.encode_utf16().count(), 160);
    }

    #[test]
    fn url_is_written_without_launching_and_rejects_ini_injection() {
        let scratch = Scratch::new();
        let mut entry = item("Website", Path::new("https://example.invalid/über.lnk"), "");
        let result =
            unsafe { create_reference(&entry, &scratch.desktop(), &scratch.staging()) }.unwrap();
        assert_eq!(result.extension().unwrap(), "url");
        let bytes = fs::read(result).unwrap();
        assert_eq!(&bytes[..2], &[0xff, 0xfe]);
        let text = String::from_utf16(
            &bytes[2..]
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            text,
            "[InternetShortcut]\r\nURL=https://example.invalid/über.lnk\r\n"
        );
        entry.target.push_str("\r\nIconFile=unexpected");
        assert!(
            unsafe { create_reference(&entry, &scratch.desktop(), &scratch.staging()) }.is_err()
        );
        assert_eq!(fs::read_dir(scratch.desktop()).unwrap().count(), 1);
    }

    #[test]
    fn shell_link_preserves_target_arguments_and_document() {
        let scratch = Scratch::new();
        let document = scratch.0.join("Untouched document.txt");
        fs::write(&document, b"document stays here").unwrap();
        let entry = item("Document", &document, "--example \"two words\"");
        let result =
            unsafe { create_reference(&entry, &scratch.desktop(), &scratch.staging()) }.unwrap();
        assert_eq!(fs::read(&document).unwrap(), b"document stays here");
        assert_eq!(fs::read_dir(scratch.staging()).unwrap().count(), 0);
        unsafe {
            let _com = Apartment::new().unwrap();
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            let persist = link.cast::<IPersistFile>().unwrap();
            let name = wide_path(&result);
            persist.Load(PCWSTR(name.as_ptr()), STGM_READ).unwrap();
            let mut arguments = [0u16; 1024];
            link.GetArguments(&mut arguments).unwrap();
            assert_eq!(
                String::from_utf16_lossy(
                    &arguments[..arguments.iter().position(|c| *c == 0).unwrap()]
                ),
                entry.arguments
            );
            let pidl = Pidl(link.GetIDList().unwrap());
            let target = SHGetNameFromIDList(pidl.0, SIGDN_FILESYSPATH).unwrap();
            let actual = target.to_string().unwrap();
            CoTaskMemFree(Some(target.0.cast()));
            assert_eq!(
                fs::canonicalize(actual).unwrap(),
                fs::canonicalize(document).unwrap()
            );
        }
    }

    #[test]
    fn appsfolder_reference_uses_quoted_explorer_namespace() {
        let entry = item("App", Path::new(r"shell:AppsFolder\Example.App!App"), "");
        let (target, arguments) = unsafe { link_target(&entry) }.unwrap();
        assert_eq!(Path::new(&target).file_name().unwrap(), "explorer.exe");
        assert_eq!(arguments, r#""shell:AppsFolder\Example.App!App""#);
        assert_eq!(quote_argument("a\\\"b\\"), "\"a\\\\\\\"b\\\\\"");
    }

    #[test]
    fn failed_write_removes_only_its_new_destination() {
        let scratch = Scratch::new();
        let existing = scratch.desktop().join("Keep.lnk");
        fs::write(&existing, b"existing").unwrap();
        let result = write_reference(&scratch.desktop(), "Keep", "lnk", |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("simulated failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(existing).unwrap(), b"existing");
        assert_eq!(fs::read_dir(scratch.desktop()).unwrap().count(), 1);
    }
}
