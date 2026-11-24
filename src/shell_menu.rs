//! The actual Windows Shell context menu for a file or shortcut.
//!
//! Call `handle_message` before borrowing the application's state in its window
//! procedure: Shell extensions can send menu messages inside TrackPopupMenu's
//! nested message loop. COM references are cloned out of TLS before forwarding
//! a message, so reentrant callbacks never hold a RefCell borrow.

use std::{
    cell::{Cell, RefCell},
    collections::{HashSet, VecDeque},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};
use windows::{
    core::{Interface, PCSTR, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::Com::{CoTaskMemFree, IBindCtx},
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT},
            Shell::{
                Common::ITEMIDLIST, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder,
                SHBindToParent, SHParseDisplayName, CMF_EXTENDEDVERBS, CMF_NORMAL,
                CMIC_MASK_CONTROL_DOWN, CMIC_MASK_PTINVOKE, CMIC_MASK_SHIFT_DOWN,
                CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, SEE_MASK_NOASYNC, SEE_MASK_UNICODE,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, IsWindow, PostMessageW,
                SetForegroundWindow, TrackPopupMenu, HMENU, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL,
                TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM,
                WM_MENUCHAR, WM_NULL,
            },
        },
    },
};

const FIRST_SHELL_ID: u32 = 1;
const LAST_SHELL_ID: u32 = 0x6fff;
const FIRST_APP_ID: u32 = 0x7000;

#[derive(Clone)]
enum ActiveMenu {
    Version3(IContextMenu3),
    Version2(IContextMenu2),
}

thread_local! {
    static ACTIVE: RefCell<Option<ActiveMenu>> = const { RefCell::new(None) };
    static CACHE: RefCell<VecDeque<CachedContext>> = const { RefCell::new(VecDeque::new()) };
}

const CACHE_LIMIT: usize = 8;
const CACHE_AGE: Duration = Duration::from_secs(60);

struct ContextObject {
    context: IContextMenu,
    _parent: IShellFolder,
    _pidl: Pidl,
    primed: Cell<bool>,
}

struct CachedContext {
    key: String,
    owner: isize,
    modified: Option<(SystemTime, u64)>,
    used: Instant,
    object: Rc<ContextObject>,
}

struct Pidl(*mut ITEMIDLIST);
impl Drop for Pidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0.cast())) }
        }
    }
}

struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

struct ActiveScope(Option<ActiveMenu>);
impl ActiveScope {
    fn new(menu: &IContextMenu) -> Self {
        let active = if let Ok(menu) = menu.cast::<IContextMenu3>() {
            Some(ActiveMenu::Version3(menu))
        } else {
            menu.cast::<IContextMenu2>().ok().map(ActiveMenu::Version2)
        };
        Self(ACTIVE.with(|slot| slot.replace(active)))
    }
}
impl Drop for ActiveScope {
    fn drop(&mut self) {
        ACTIVE.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn shell_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
    }
}

unsafe fn build_context(owner: HWND, path: &str) -> Result<Rc<ContextObject>, String> {
    let name = wide(&shell_path(path));
    let mut pidl = Pidl(std::ptr::null_mut());
    SHParseDisplayName(
        PCWSTR(name.as_ptr()),
        None::<&IBindCtx>,
        &mut pidl.0,
        0,
        None,
    )
    .map_err(|error| format!("Windows could not find this shortcut: {error}"))?;
    if pidl.0.is_null() {
        return Err("Windows did not return an item for this shortcut.".into());
    }
    let mut last = std::ptr::null_mut();
    let parent: IShellFolder = SHBindToParent(pidl.0, Some(&mut last))
        .map_err(|error| format!("Windows could not open this shortcut's folder: {error}"))?;
    if last.is_null() {
        return Err("Windows did not return the shortcut's relative item.".into());
    }
    let context: IContextMenu = parent
        .GetUIObjectOf(owner, &[last.cast_const()], None)
        .map_err(|error| format!("Windows could not load this item's context menu: {error}"))?;
    Ok(Rc::new(ContextObject {
        context,
        _parent: parent,
        _pidl: pidl,
        primed: Cell::new(false),
    }))
}

unsafe fn context_for(owner: HWND, path: &str) -> Result<Rc<ContextObject>, String> {
    let path = shell_path(path);
    let key = path.replace('/', "\\").to_lowercase();
    let modified = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok().map(|t| (t, m.len())));
    let now = Instant::now();
    // Release COM objects outside a RefCell borrow; shell extensions may reenter.
    let mut entries = CACHE.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    entries.retain(|entry| {
        now.duration_since(entry.used) < CACHE_AGE
            && !(entry.key == key
                && (entry.owner != owner.0 as isize || entry.modified != modified))
    });
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.key == key && entry.owner == owner.0 as isize)
    {
        let mut entry = entries.remove(index).unwrap();
        entry.used = now;
        let object = entry.object.clone();
        entries.push_front(entry);
        CACHE.with(|slot| *slot.borrow_mut() = entries);
        return Ok(object);
    }
    let result = build_context(owner, &path);
    if let Ok(object) = &result {
        entries.push_front(CachedContext {
            key,
            owner: owner.0 as isize,
            modified,
            used: now,
            object: object.clone(),
        });
        while entries.len() > CACHE_LIMIT {
            entries.pop_back();
        }
    }
    CACHE.with(|slot| *slot.borrow_mut() = entries);
    result
}

/// Release cached handlers after edits/imports or when the folder view closes.
pub fn clear_cache() {
    let old = CACHE.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    drop(old);
}

/// Prepare only the app the user has paused on. The menu itself is always
/// queried afresh on right-click so dynamic shell commands remain current.
pub unsafe fn warm_path(owner: HWND, path: &str) -> Result<(), String> {
    if owner.0.is_null()
        || !IsWindow(owner).as_bool()
        || path.trim().is_empty()
        || path.contains('\0')
    {
        return Ok(());
    }
    let object = context_for(owner, path)?;
    if object.primed.get() {
        return Ok(());
    }
    let menu = Menu(CreatePopupMenu().map_err(|e| e.to_string())?);
    let _active = ActiveScope::new(&object.context);
    object
        .context
        .QueryContextMenu(menu.0, 0, FIRST_SHELL_ID, LAST_SHELL_ID, CMF_NORMAL)
        .map_err(|e| e.to_string())?;
    object.primed.set(true);
    Ok(())
}

/// Forward owner-drawn/dynamic Shell submenu messages from the menu's owner.
/// Returns None if there is no active handler or it did not handle the message.
pub unsafe fn handle_message(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if !matches!(
        msg,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        return None;
    }
    // Nonzero wParam identifies an owner-drawn control rather than a menu.
    if matches!(msg, WM_DRAWITEM | WM_MEASUREITEM) && wp.0 != 0 {
        return None;
    }
    let active = ACTIVE.with(|slot| slot.borrow().clone());
    match active? {
        ActiveMenu::Version3(menu) => {
            let mut result = LRESULT(0);
            menu.HandleMenuMsg2(msg, wp, lp, Some(&mut result)).ok()?;
            Some(result)
        }
        ActiveMenu::Version2(menu) if msg != WM_MENUCHAR => {
            menu.HandleMenuMsg(msg, wp, lp).ok()?;
            Some(LRESULT(if matches!(msg, WM_DRAWITEM | WM_MEASUREITEM) {
                1
            } else {
                0
            }))
        }
        _ => None,
    }
}

/// Show the normal Windows context menu for `path` itself, including .lnk files.
///
/// `extra` may append NestDesktop commands with unique IDs in 0x7000..=0xffff.
/// A selected extra command is returned. A cancelled menu or a Shell command
/// successfully invoked through IContextMenu returns None.
///
/// The caller must initialize COM on the calling UI thread and forward the
/// owner's menu messages with `handle_message` while this function runs.
pub unsafe fn show(
    owner: HWND,
    path: &str,
    point: POINT,
    extra: &[(u32, &str)],
) -> Result<Option<u32>, String> {
    if owner.0.is_null() || !IsWindow(owner).as_bool() {
        return Err("The Windows context menu needs a valid owner window.".into());
    }
    if path.trim().is_empty() || path.contains('\0') {
        return Err("This shortcut has no valid Shell path.".into());
    }
    let mut ids = HashSet::new();
    for &(id, label) in extra {
        if !(FIRST_APP_ID..=0xffff).contains(&id) || !ids.insert(id) || label.contains('\0') {
            return Err("Invalid application context-menu entry.".into());
        }
    }

    let object = context_for(owner, path)?;
    let context = &object.context;
    let menu = Menu(
        CreatePopupMenu()
            .map_err(|error| format!("The context menu could not be created: {error}"))?,
    );
    // Drop the active scope before destroying the menu or releasing COM items.
    let _active = ActiveScope::new(&context);
    let shifted = GetKeyState(VK_SHIFT.0 as i32) < 0;
    let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
    context
        .QueryContextMenu(
            menu.0,
            0,
            FIRST_SHELL_ID,
            LAST_SHELL_ID,
            CMF_NORMAL | if shifted { CMF_EXTENDEDVERBS } else { 0 },
        )
        .map_err(|error| format!("Windows could not populate this item's context menu: {error}"))?;
    object.primed.set(true);
    if !extra.is_empty() {
        AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null())
            .map_err(|error| format!("Could not add the menu separator: {error}"))?;
        for &(id, label) in extra {
            let label = wide(label);
            AppendMenuW(menu.0, MF_STRING, id as usize, PCWSTR(label.as_ptr()))
                .map_err(|error| format!("Could not add the application menu command: {error}"))?;
        }
    }
    let _ = SetForegroundWindow(owner);
    let selected = TrackPopupMenu(
        menu.0,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        owner,
        None,
    )
    .0 as u32;
    let _ = PostMessageW(owner, WM_NULL, WPARAM(0), LPARAM(0));
    if selected == 0 {
        return Ok(None);
    }
    if selected >= FIRST_APP_ID {
        return if ids.contains(&selected) {
            Ok(Some(selected))
        } else {
            Err("Windows returned an unknown menu command.".into())
        };
    }
    if selected > LAST_SHELL_ID {
        return Err("Windows returned an invalid Shell command.".into());
    }
    // The Shell expects an offset from QueryContextMenu's first ID. Supply the
    // integer verb in both fields so Unicode and older handlers receive it.
    let offset = (selected - FIRST_SHELL_ID) as usize;
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: SEE_MASK_UNICODE
            | SEE_MASK_NOASYNC
            | CMIC_MASK_PTINVOKE
            | if shifted { CMIC_MASK_SHIFT_DOWN } else { 0 }
            | if control { CMIC_MASK_CONTROL_DOWN } else { 0 },
        hwnd: owner,
        lpVerb: PCSTR(offset as *const u8),
        lpVerbW: PCWSTR(offset as *const u16),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: point,
        ..Default::default()
    };
    context
        .InvokeCommand((&info as *const CMINVOKECOMMANDINFOEX).cast::<CMINVOKECOMMANDINFO>())
        .map_err(|error| format!("The Windows menu command could not be completed: {error}"))?;
    Ok(None)
}
