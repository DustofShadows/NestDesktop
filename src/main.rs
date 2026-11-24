#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]
mod actions_v2;
mod brand;
mod color_ui;
mod creation;
mod desktop_grid;
mod desktop_links;
mod desktop_positions;
mod desktop_refresh;
mod desktop_transfer;
mod interaction;
mod layout;
mod manager_scroll;
mod model;
mod native_color;
#[cfg(test)]
mod paint_message_tests;
mod paint_retry;
mod palette;
mod palette_view;
mod platform;
mod popup_alpha_check;
mod recovery;
mod render;
mod reorder_ui;
mod runtime_v2;
mod settings_ui;
mod shared_conversion;
mod shared_shell;
mod shared_ui;
mod shell_menu;
mod shortcuts;
mod snap_preview;
mod surface;
mod transfer;
mod transparency_check;
mod views;
use model::{Config, Group, Item};
use render::Canvas;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, LibraryLoader::*},
        UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
};
const WM_TRAY: u32 = WM_APP + 1;
const WM_CATALOGUE: u32 = WM_APP + 2;
const WM_REFRESH: u32 = WM_APP + 3;
const WM_DO: u32 = WM_APP + 4;
const WM_REQUEST: u32 = WM_APP + 5;
const WM_TITLE_FINISH: u32 = WM_APP + 6;
const WM_DELAY_FINISH: u32 = WM_APP + 7;
const ID_SEARCH: usize = 600;
const ID_LIST: usize = 601;
const ID_INPUT: usize = 602;
const ID_TITLE: usize = 605;
const ID_HOVER_DELAY: usize = 606;
const ID_COLOR_HEX: usize = 607;
const PAINT_RETRY_TIMER: usize = 12;
const POSITION_RETRY_TIMER: usize = 13;
const MANAGER: isize = 1;
const POPUP: isize = 2;
const PICKER: isize = 3;
const PROMPT: isize = 4;
const TITLE_HOST: isize = 5;
const COLOR_PANEL: isize = 6;
const TILE: isize = 1000;
thread_local! {
 static APP:RefCell<Option<App>>=const{RefCell::new(None)};
 // Native modal APIs dispatch messages while App is already borrowed. Keep
 // application-owned messages here until the outer dispatch has returned.
 static DEFERRED:RefCell<Vec<(HWND,u32,WPARAM,LPARAM)>>=const{RefCell::new(Vec::new())};
 static TASKBAR_MESSAGE:Cell<u32>=const{Cell::new(0)};
}
#[derive(Clone)]
enum Action {
    Open(usize),
    Menu(usize),
    NewGroup,
    AddApps,
    Browse,
    NewSubfolder,
    Back,
    Close,
    Manage,
    Settings,
    Home,
    Import,
    Export,
    Startup,
    DataFolder,
    Uninstall,
    Quit,
    Launch(String),
    PickAdd,
    PromptOk,
    PromptCancel,
    SetColor(String),
    RenameGroup,
    RenameItem(String),
    RemoveGroup,
    RemoveItem(String),
    MoveItem(String),
    PageBack,
    PageNext,
    ClosePicker,
    RevealDesktop,
    CollectDesktop,
    RestoreDesktop,
    ToggleCounts,
    ToggleSnap,
    ToggleHover,
    CustomColor,
    ChooseColor,
    Palette(palette_view::Control),
    TogglePin,
    EditTitle,
}
#[derive(Clone)]
struct Hit {
    r: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
    action: Action,
}
#[derive(Clone)]
enum PromptKind {
    NewGroup,
    RenameGroup(usize),
    RenameItem(String),
    NewFolder,
}
struct Prompt {
    kind: PromptKind,
    input: HWND,
}
struct Drag {
    group: usize,
    start: POINT,
    origin: (i32, i32),
    moved: bool,
}
struct ItemDrag {
    id: String,
    group: usize,
    start: POINT,
    moved: bool,
}
struct App {
    pending_creation: Option<creation::Pending>,
    watchdog: Option<recovery::Watchdog>,
    config: Config,
    path: PathBuf,
    controller: HWND,
    manager: HWND,
    popup: HWND,
    color_window: HWND,
    color_hex_edit: HWND,
    color_session: Option<color_ui::ColorSession>,
    color_drag: Option<palette_view::Control>,
    color_pressed: Option<palette_view::Control>,
    color_sync: bool,
    picker: HWND,
    prompt: HWND,
    tiles: Vec<HWND>,
    canvases: HashMap<isize, Canvas>,
    presentations: HashMap<isize, u64>,
    paint_retries: paint_retry::RetryQueue,
    paint_retry_timer_active: bool,
    tile_mouse_events: u64,
    fail_first_tile_paint: bool,
    injected_paint_failures: usize,
    desktop_session: Option<desktop_positions::DesktopSession>,
    pending_desktop_positions: Vec<(PathBuf, shortcuts::DesktopPosition, bool)>,
    position_retry_attempts: u8,
    quitting: bool,
    hits: HashMap<isize, Vec<Hit>>,
    hover: HashMap<isize, usize>,
    group: usize,
    trail: Vec<String>,
    search: HWND,
    query: String,
    page: usize,
    focused: usize,
    catalogue: Vec<(String, String)>,
    catalogue_ready: bool,
    picker_list: HWND,
    picker_search: HWND,
    picker_map: Vec<usize>,
    prompt_state: Option<Prompt>,
    drag: Option<Drag>,
    item_drag: Option<ItemDrag>,
    scale: f32,
    taskbar_created: u32,
    settings: bool,
    manager_scroll: i32,
    manager_scrollbar: HWND,
    manager_wheel_delta: i32,
    font: HFONT,
    brush: HBRUSH,
    status: String,
    context_item: Option<String>,
    desktop_retries: u8,
    popup_pinned: bool,
    hovered_group: Option<usize>,
    hover_progress: f32,
    hover_started: Option<Instant>,
    hover_opened: bool,
    open_animation_progress: f32,
    animation: Option<runtime_v2::ExpandAnimation>,
    title_edit: HWND,
    hover_delay_edit: HWND,
    snap_preview: snap_preview::SnapPreview,
    title_host: HWND,
    title_target: Option<runtime_v2::TitleTarget>,
    title_generation: usize,
    title_font: HFONT,
    reorder_before: Option<String>,
    reorder_at_end: bool,
    grid: desktop_grid::GridMetrics,
    menu_warm_path: Option<String>,
    search_brush: HBRUSH,
    title_brush: HBRUSH,
    tile_title_brush: HBRUSH,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn pos(l: LPARAM) -> (i32, i32) {
    (
        (l.0 as u16 as i16) as i32,
        ((l.0 >> 16) as u16 as i16) as i32,
    )
}
unsafe fn text(h: HWND) -> String {
    let n = GetWindowTextLengthW(h).max(0) as usize;
    let mut v = vec![0; n + 1];
    GetWindowTextW(h, &mut v);
    String::from_utf16_lossy(&v[..n])
}
unsafe fn message(owner: HWND, t: &str) {
    let s = wide(t);
    MessageBoxW(
        owner,
        PCWSTR(s.as_ptr()),
        w!("NestDesktop"),
        MB_OK | MB_ICONINFORMATION,
    );
}
unsafe fn invalidate(h: HWND) {
    if !h.0.is_null() {
        let _ = InvalidateRect(h, None, false);
    }
}
fn defer_if_busy(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> bool {
    if msg == WM_PAINT && unsafe { GetWindowLongPtrW(h, GWLP_USERDATA) } > 0 {
        // DefWindowProc would consume this update without drawing our surface.
        // Validate now; replay one coalesced request after the App borrow ends.
        unsafe {
            let _ = ValidateRect(h, None);
        }
        if !paint_retry::is_rendering(h.0 as isize) {
            DEFERRED.with(|queue| {
                let mut queue = queue.borrow_mut();
                if !queue
                    .iter()
                    .any(|entry| entry.0 == h && entry.1 == WM_PAINT)
                {
                    queue.push((h, msg, wp, lp));
                }
            });
        }
        return true;
    }
    let taskbar = TASKBAR_MESSAGE.with(Cell::get);
    if matches!(
        msg,
        WM_CATALOGUE | WM_DO | WM_REFRESH | WM_REQUEST | WM_TITLE_FINISH | WM_DELAY_FINISH
    ) || (taskbar != 0 && msg == taskbar)
    {
        DEFERRED.with(|q| q.borrow_mut().push((h, msg, wp, lp)));
        true
    } else if matches!(msg, WM_DISPLAYCHANGE | WM_SETTINGCHANGE) {
        DEFERRED.with(|q| q.borrow_mut().push((h, WM_REFRESH, WPARAM(0), LPARAM(0))));
        true
    } else {
        false
    }
}
unsafe fn drain_deferred() {
    loop {
        let pending = DEFERRED.with(|q| std::mem::take(&mut *q.borrow_mut()));
        if pending.is_empty() {
            break;
        }
        for (h, msg, wp, lp) in pending {
            if IsWindow(h).as_bool() {
                wndproc(h, msg, wp, lp);
            } else if msg == WM_CATALOGUE {
                drop(Box::from_raw(lp.0 as *mut Vec<(String, String)>));
            } else if msg == WM_DO {
                drop(Box::from_raw(lp.0 as *mut Action));
            }
        }
    }
}
fn folder_path(items: &[Item], id: &str, path: &mut Vec<String>) -> bool {
    for item in items {
        if item.is_folder() {
            path.push(item.id.clone());
            if item.id == id || folder_path(&item.children, id, path) {
                return true;
            }
            path.pop();
        }
    }
    false
}
fn shell_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}
fn same_shell_path(left: &str, right: &str) -> bool {
    shell_path(left)
        .replace('/', "\\")
        .eq_ignore_ascii_case(&shell_path(right).replace('/', "\\"))
}
fn replace_targets(config: &mut Config, old: &str, new: &str) {
    fn walk(v: &mut [Item], old: &str, new: &str) {
        for it in v {
            if same_shell_path(&it.target, old) {
                it.target = shell_path(new)
            }
            walk(&mut it.children, old, new);
        }
    }
    for g in &mut config.groups {
        walk(&mut g.items, old, new);
    }
}
fn quiet_restore(data: &Path) -> std::result::Result<(), String> {
    restore_profile(data, false)
}
fn restore_profile(data: &Path, forget: bool) -> std::result::Result<(), String> {
    // A failed shared conversion must not strand ordinary collected shortcuts.
    let shared = restore_shared_profile(data);
    let personal = restore_personal_profile(data, forget);
    match (shared, personal) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(shared), Err(personal)) => Err(format!("{shared}\n{personal}")),
    }
}
fn restore_shared_profile(data: &Path) -> std::result::Result<(), String> {
    let layout = data.join("layout.json");
    if !shared_conversion::journal_path(&layout).exists() {
        return Ok(());
    }
    let public = platform::public_desktop_directory()
        .ok_or("The shared Desktop folder is unavailable for shortcut recovery.")?;
    let report = shared_conversion::recover(&layout, &public, &platform::desktop_directory())?;
    let mut positions = Vec::new();
    for converted in report.converted {
        if !converted.original.exists() {
            desktop_positions::notify_removed(std::slice::from_ref(&converted.original));
        }
        desktop_positions::notify_created(std::slice::from_ref(&converted.personal));
        if let Some(position) = converted.position {
            positions.push((converted.personal, position));
        }
    }
    let mut warnings = report.warnings;
    warnings.extend(desktop_refresh::finish_positions_blocking(positions));
    if !warnings.is_empty() {
        let _ = std::fs::write(
            data.join("shared-restore-warnings.txt"),
            warnings.join("\n"),
        );
    }
    if report.pending == 0 {
        Ok(())
    } else {
        warnings.push(format!("{} shared-shortcut conversion records still need recovery. Their files and records were retained.", report.pending));
        Err(warnings.join("\n"))
    }
}
fn restore_personal_profile(data: &Path, forget: bool) -> std::result::Result<(), String> {
    let storage = data.join("shortcuts");
    let entries = shortcuts::load_ledger(&storage)?;
    if entries.is_empty() && !forget {
        return Ok(());
    }
    let layout = data.join("layout.json");
    match Config::load(&layout) {
        Ok(mut config) => {
            let outcome = desktop_refresh::restore_all(&mut config, &layout, forget);
            let position_warnings = desktop_refresh::finish_positions_blocking(outcome.positions);
            let mut report = outcome.result?;
            report.warnings.extend(position_warnings);
            if !report.warnings.is_empty() {
                let _ = std::fs::write(
                    data.join("restore-warnings.txt"),
                    report.warnings.join("\n"),
                );
            }
            if report.failures == 0 {
                Ok(())
            } else {
                Err(report.warnings.join("\n"))
            }
        }
        Err(layout_error) => {
            // Corrupt layout data must never strand otherwise recoverable
            // shortcuts. Keep every ledger entry so repaired layout targets
            // can be remapped during a later successful recovery.
            let mut failures = vec![format!(
                "The layout could not be read and was preserved. Desktop shortcuts were restored where possible. {layout_error}"
            )];
            let mut positions = Vec::new();
            for entry in entries {
                let moved = Path::new(&entry.stored).is_file();
                match shortcuts::restore_shortcut(&entry) {
                    Err(error) => failures.push(error),
                    Ok(()) if moved => {
                        let original = PathBuf::from(&entry.original);
                        desktop_positions::notify_created(std::slice::from_ref(&original));
                        if let Some(position) = entry.position {
                            positions.push((original, position));
                        }
                    }
                    _ => {}
                }
            }
            failures.extend(desktop_refresh::finish_positions_blocking(positions));
            Err(failures.join("\n"))
        }
    }
}
#[cfg(test)]
mod recovery_tests {
    use super::*;

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let root = std::env::temp_dir().join(model::new_id("profile-recovery-test"));
            std::fs::create_dir_all(root.join("Desktop")).unwrap();
            std::fs::create_dir_all(root.join("Profile")).unwrap();
            Self(root)
        }
        fn data(&self) -> PathBuf {
            self.0.join("Profile")
        }
        fn desktop(&self) -> PathBuf {
            self.0.join("Desktop")
        }
        fn layout(&self) -> PathBuf {
            self.data().join("layout.json")
        }
        fn collect(&self) -> (Config, PathBuf) {
            let path = self.desktop().join("Example.lnk");
            std::fs::write(&path, b"synthetic shortcut").unwrap();
            let mut config = Config::default();
            config.save(&self.layout()).unwrap();
            desktop_transfer::collect_into(
                &mut config,
                &self.layout(),
                &self.desktop(),
                &path,
                0,
                None,
            )
            .unwrap();
            (config, path)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn unreadable_layout_does_not_strand_shortcuts_or_discard_recovery_records() {
        let s = Sandbox::new();
        let (_, original) = s.collect();
        let storage = s.data().join("shortcuts");
        let before = shortcuts::load_ledger(&storage).unwrap();
        std::fs::write(s.layout(), b"unreadable layout").unwrap();
        assert!(quiet_restore(&s.data()).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"synthetic shortcut");
        assert_eq!(shortcuts::load_ledger(&storage).unwrap(), before);
        assert_eq!(std::fs::read(s.layout()).unwrap(), b"unreadable layout");
    }

    #[test]
    #[ignore = "Native Shell integration; only synthetic directories and shortcut bytes"]
    fn native_shared_conversion_then_collection_and_restore() {
        struct Sta;
        impl Drop for Sta {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        }
        let _sta = Sta;
        let s = Sandbox::new();
        let public = s.0.join("SyntheticPublicDesktop");
        std::fs::create_dir_all(&public).unwrap();
        let source = public.join("Shared app.lnk");
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            let target = wide(&std::env::current_exe().unwrap().to_string_lossy());
            link.SetPath(PCWSTR(target.as_ptr())).unwrap();
            let persist = link.cast::<IPersistFile>().unwrap();
            let name = wide(&source.to_string_lossy());
            persist.Save(PCWSTR(name.as_ptr()), true).unwrap();
        }
        let payload = std::fs::read(&source).unwrap();
        let mut config = Config::default();
        config.groups[0]
            .items
            .push(Item::shortcut("Shared app", source.to_string_lossy()));
        let item_id = config.groups[0].items[0].id.clone();
        config.save(&s.layout()).unwrap();
        let position = shortcuts::DesktopPosition { x: 232, y: 94 };
        let prepared = shared_conversion::prepare(
            &s.layout(),
            &public,
            &s.desktop(),
            &[(source.clone(), Some(position))],
        )
        .unwrap();
        let report = prepared
            .execute(|source, stage| unsafe {
                shared_shell::move_shortcut(HWND::default(), source, &public, stage)
            })
            .unwrap();
        assert_eq!(report.pending, 0, "{:?}", report.warnings);
        assert_eq!(report.converted.len(), 1, "{:?}", report.warnings);
        let personal = report.converted[0].personal.clone();
        assert!(!source.exists());
        assert_eq!(std::fs::read(&personal).unwrap(), payload);
        config = Config::load(&s.layout()).unwrap();
        assert_eq!(config.groups[0].items[0].id, item_id);
        assert!(same_shell_path(
            &config.groups[0].items[0].target,
            &personal.to_string_lossy()
        ));
        assert_eq!(
            desktop_transfer::collect_many_into_with_positions(
                &mut config,
                &s.layout(),
                &s.desktop(),
                &[(personal.clone(), Some(position))],
                0,
                None,
                |_| {}
            )
            .unwrap(),
            1
        );
        assert!(!personal.exists());
        let active = shortcuts::load_ledger(&s.data().join("shortcuts")).unwrap();
        assert_eq!(active[0].position, Some(position));
        let restored = desktop_transfer::restore_all(&mut config, &s.layout(), false).unwrap();
        assert_eq!(restored.restored, 1);
        assert_eq!(std::fs::read(&personal).unwrap(), payload);
        assert!(!source.exists());
        assert_eq!(config.groups[0].items.len(), 1);
        assert_eq!(config.groups[0].items[0].id, item_id);
    }

    #[test]
    fn corrupt_shared_journal_does_not_prevent_personal_shortcut_restoration() {
        let s = Sandbox::new();
        let (_, original) = s.collect();
        let journal = shared_conversion::journal_path(&s.layout());
        std::fs::write(&journal, b"invalid shared conversion journal").unwrap();
        assert!(quiet_restore(&s.data()).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"synthetic shortcut");
        assert!(shortcuts::load_ledger(&s.data().join("shortcuts"))
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read(journal).unwrap(),
            b"invalid shared conversion journal"
        );
        let restored = Config::load(&s.layout()).unwrap();
        assert!(same_shell_path(
            &restored.groups[0].items[0].target,
            &original.to_string_lossy()
        ));
    }

    #[test]
    fn corrupt_ledger_is_preserved_without_following_any_recovery_paths() {
        let s = Sandbox::new();
        let (_, original) = s.collect();
        let ledger = s.data().join("shortcuts/collection.json");
        std::fs::write(&ledger, b"unreadable ledger").unwrap();
        assert!(quiet_restore(&s.data()).is_err());
        assert!(!original.exists());
        assert_eq!(std::fs::read(ledger).unwrap(), b"unreadable ledger");
    }

    #[test]
    fn explicit_profile_restore_forgets_ownership_even_with_an_empty_active_ledger() {
        let s = Sandbox::new();
        let (_, original) = s.collect();
        quiet_restore(&s.data()).unwrap();
        assert!(original.exists());
        assert_eq!(
            desktop_transfer::load_bindings(&s.layout()).unwrap().len(),
            1
        );
        assert!(shortcuts::load_ledger(&s.data().join("shortcuts"))
            .unwrap()
            .is_empty());
        restore_profile(&s.data(), true).unwrap();
        assert!(desktop_transfer::load_bindings(&s.layout())
            .unwrap()
            .is_empty());
        let mut config = Config::load(&s.layout()).unwrap();
        assert_eq!(
            desktop_transfer::reconcile_startup(&mut config, &s.layout(), &s.desktop())
                .unwrap()
                .collected,
            0
        );
        assert!(original.exists());
    }
}
fn point_in_rect(point: POINT, rect: RECT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}
unsafe fn rounded(h: HWND, w: i32, hh: i32, r: i32) {
    let _ = (w, hh, r);
    surface::round_window(h);
}
unsafe fn make_window(
    kind: isize,
    title: &str,
    w: i32,
    h: i32,
    parent: HWND,
    style: WINDOW_STYLE,
    ex: WINDOW_EX_STYLE,
    s: f32,
) -> HWND {
    let t = wide(title);
    let a = platform::primary_work_area();
    let pw = (w as f32 * s) as i32;
    let ph = (h as f32 * s) as i32;
    let window = CreateWindowExW(
        ex,
        w!("NestDesktopWindow"),
        PCWSTR(t.as_ptr()),
        style,
        a.left + (a.right - a.left - pw) / 2,
        a.top + (a.bottom - a.top - ph) / 2,
        pw,
        ph,
        parent,
        None,
        GetModuleHandleW(None).unwrap_or_default(),
        Some(kind as *const _),
    )
    .unwrap_or_default();
    if kind < TILE {
        brand::apply_window(window);
    }
    window
}
unsafe fn edit(
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    s: f32,
    font: HFONT,
    ph: &str,
) -> HWND {
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EDIT"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        (x as f32 * s) as i32,
        (y as f32 * s) as i32,
        (w as f32 * s) as i32,
        (h as f32 * s) as i32,
        parent,
        HMENU(id as *mut _),
        GetModuleHandleW(None).unwrap_or_default(),
        None,
    )
    .unwrap_or_default();
    SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    let hint = wide(ph);
    SendMessageW(
        hwnd,
        EM_SETCUEBANNER,
        WPARAM(0),
        LPARAM(hint.as_ptr() as isize),
    );
    hwnd
}
impl App {
    fn desktop_view(&mut self) -> Option<&desktop_positions::DesktopSession> {
        if self.desktop_session.is_none() {
            self.desktop_session = desktop_positions::DesktopSession::connect().ok();
        }
        self.desktop_session.as_ref()
    }
    unsafe fn apply_desktop_positions(
        &mut self,
        entries: desktop_refresh::Positions,
    ) -> Vec<String> {
        self.queue_desktop_positions(entries, true)
    }
    unsafe fn queue_desktop_positions(
        &mut self,
        entries: desktop_refresh::Positions,
        exact: bool,
    ) -> Vec<String> {
        if entries.is_empty() {
            return Vec::new();
        }
        for (path, position) in entries {
            self.pending_desktop_positions.retain(|(existing, _, _)| {
                !same_shell_path(&existing.to_string_lossy(), &path.to_string_lossy())
            });
            self.pending_desktop_positions.push((path, position, exact));
        }
        self.position_retry_attempts = 0;
        self.try_desktop_positions()
    }
    unsafe fn try_desktop_positions(&mut self) -> Vec<String> {
        let _ = KillTimer(self.controller, POSITION_RETRY_TIMER);
        let pending = std::mem::take(&mut self.pending_desktop_positions);
        if pending.is_empty() {
            return Vec::new();
        }
        let mut warnings = Vec::new();
        for exact in [true, false] {
            let entries: Vec<_> = pending
                .iter()
                .filter(|(_, _, value)| *value == exact)
                .map(|(path, position, _)| (path.clone(), *position))
                .collect();
            if entries.is_empty() {
                continue;
            }
            let result = self.desktop_view().map(|view| {
                if exact {
                    view.restore_positions(&entries)
                } else {
                    view.restore_drop_positions(&entries)
                }
            });
            match result {
                Some(Ok(report)) => {
                    self.pending_desktop_positions.extend(
                        report
                            .pending
                            .into_iter()
                            .map(|(path, position)| (path, position, exact)),
                    );
                    warnings.extend(report.warnings);
                }
                failure => {
                    self.pending_desktop_positions.extend(
                        entries
                            .into_iter()
                            .map(|(path, position)| (path, position, exact)),
                    );
                    self.desktop_session = None;
                    warnings.push(match failure {
                        Some(Err(error)) => error,
                        _ => "The Desktop view is not available for icon positioning.".into(),
                    });
                }
            }
        }
        if !self.pending_desktop_positions.is_empty() && self.position_retry_attempts < 3 {
            if SetTimer(self.controller, POSITION_RETRY_TIMER, 50, None) != 0 {
                return Vec::new();
            }
            warnings.push("The Desktop position retry could not be scheduled.".into());
        }
        if !self.pending_desktop_positions.is_empty() {
            warnings.push(format!(
                "Explorer has not restored {} saved icon positions yet.",
                self.pending_desktop_positions.len()
            ));
            self.pending_desktop_positions.clear();
            if let Some(root) = self.path.parent() {
                let _ = std::fs::write(
                    root.join("desktop-position-warnings.txt"),
                    warnings.join("\n"),
                );
            }
        }
        warnings
    }
    unsafe fn restore_collected(
        &mut self,
        forget: bool,
    ) -> std::result::Result<desktop_transfer::Report, String> {
        let outcome = desktop_refresh::restore_all(&mut self.config, &self.path, forget);
        let warnings = self.apply_desktop_positions(outcome.positions);
        let mut result = outcome.result;
        if let Ok(report) = result.as_mut() {
            report.warnings.extend(warnings);
        }
        result
    }
    unsafe fn begin_quit(&mut self) {
        self.v4_close_colors(false);
        self.v2_finish_title(true);
        self.v3_finish_hover_delay(true, false);
        if let Some(drag) = self.drag.take() {
            self.v2_snap_group(drag.group);
        }
        self.quitting = true;
        self.snap_preview.hide();
        self.v2_close_popup();
        for h in self.tiles.iter().copied().chain([
            self.manager,
            self.picker,
            self.prompt,
            self.title_host,
        ]) {
            let _ = ShowWindow(h, SW_HIDE);
        }
        for id in [7, 8, 9, 10, 11, PAINT_RETRY_TIMER] {
            let _ = KillTimer(self.controller, id);
        }
        self.paint_retries.clear();
        self.paint_retry_timer_active = false;
        self.save();
        match self.restore_collected(false) {
            Err(error) => {
                if let Some(root) = self.path.parent() {
                    let _ = std::fs::write(root.join("restore-error.txt"), error);
                }
            }
            Ok(report) if !report.warnings.is_empty() => {
                if let Some(root) = self.path.parent() {
                    let _ = std::fs::write(
                        root.join("restore-warnings.txt"),
                        report.warnings.join("\n"),
                    );
                }
            }
            _ => {}
        }
        self.remove_tray();
        if self.pending_desktop_positions.is_empty() {
            PostQuitMessage(0);
        }
    }
    unsafe fn dialog_owner(&self) -> HWND {
        if IsWindowVisible(self.popup).as_bool() {
            self.popup
        } else if IsWindowVisible(self.manager).as_bool() {
            self.manager
        } else {
            self.controller
        }
    }
    unsafe fn collect_desktop(&mut self) {
        let paths = platform::choose_desktop_shortcuts(self.popup);
        self.drop_paths(paths);
    }
    unsafe fn ensure_recovery(&mut self) -> std::result::Result<(), String> {
        if let Some(helper) = self.watchdog.as_mut() {
            if helper.is_running()? {
                return Ok(());
            }
        }
        self.watchdog = None;
        let data = self
            .path
            .parent()
            .ok_or("The layout folder is unavailable.")?;
        match recovery::spawn_watchdog(data) {
            Ok(helper) => {
                self.watchdog = Some(helper);
                Ok(())
            }
            Err(error) => {
                // If a previously-ready helper exited, return existing owned
                // shortcuts before allowing any further collection attempt.
                let restored = self.restore_collected(false);
                let detail = match restored {
                    Ok(report) if report.remaining == 0 => String::new(),
                    Ok(report) => format!("\n{}", report.warnings.join("\n")),
                    Err(restore_error) => format!("\n{restore_error}"),
                };
                Err(format!("Shortcut recovery could not start. No new shortcuts were moved. {error}{detail}"))
            }
        }
    }
    unsafe fn prepare_shared_drop(
        &mut self,
        paths: Vec<String>,
    ) -> Option<(Vec<String>, HashMap<String, shortcuts::DesktopPosition>)> {
        let mut positions = HashMap::new();
        let Some(public) =
            platform::public_desktop_directory().and_then(|path| std::fs::canonicalize(path).ok())
        else {
            return Some((paths, positions));
        };
        let mut shared = Vec::new();
        for path in &paths {
            let source = Path::new(path);
            let shortcut = matches!(
                source
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("lnk" | "url")
            );
            if shortcut
                && source
                    .parent()
                    .and_then(|value| std::fs::canonicalize(value).ok())
                    .as_ref()
                    == Some(&public)
                && !shared
                    .iter()
                    .any(|existing: &PathBuf| same_shell_path(&existing.to_string_lossy(), path))
            {
                shared.push(source.to_path_buf());
            }
        }
        if shared.is_empty() {
            return Some((paths, positions));
        }
        let selected = shared_ui::choose(self.dialog_owner(), &shared)?;
        if selected.is_empty() {
            return Some((paths, positions));
        }
        if let Err(error) = self
            .ensure_recovery()
            .and_then(|_| self.config.save(&self.path))
        {
            message(self.dialog_owner(), &error);
            return None;
        }
        let selection: Vec<_> = selected
            .iter()
            .map(|path| {
                let position = self
                    .desktop_view()
                    .and_then(|view| view.capture_public(path, &public).ok().flatten());
                (path.clone(), position)
            })
            .collect();
        let desktop = platform::desktop_directory();
        let prepared = match shared_conversion::prepare(&self.path, &public, &desktop, &selection) {
            Ok(prepared) => prepared,
            Err(error) => {
                message(self.dialog_owner(), &error);
                return None;
            }
        };
        let owner = self.dialog_owner();
        let mut errors = Vec::new();
        let report = match prepared
            .execute(|source, stage| shared_shell::move_shortcut(owner, source, &public, stage))
        {
            Ok(report) => report,
            Err(error) => {
                errors.push(error);
                match shared_conversion::recover(&self.path, &public, &desktop) {
                    Ok(report) => report,
                    Err(error) => {
                        errors.push(error);
                        message(owner, &errors.join("\n\n"));
                        return None;
                    }
                }
            }
        };
        errors.extend(report.warnings);
        let mut next = Vec::new();
        for path in paths {
            if let Some(converted) = report
                .converted
                .iter()
                .find(|entry| same_shell_path(&entry.original.to_string_lossy(), &path))
            {
                if !converted.original.exists() {
                    desktop_positions::notify_removed(std::slice::from_ref(&converted.original));
                }
                desktop_positions::notify_created(std::slice::from_ref(&converted.personal));
                let personal = shell_path(&converted.personal.to_string_lossy());
                if let Some(position) = converted.position {
                    positions.insert(personal.clone(), position);
                }
                next.push(personal);
            } else if !selected
                .iter()
                .any(|selected| same_shell_path(&selected.to_string_lossy(), &path))
            {
                // Unchecked rows explicitly requested a reference.
                next.push(path);
            }
        }
        match Config::load(&self.path) {
            Ok(config) => self.config = config,
            Err(error) => {
                errors.push(error);
                message(owner, &errors.join("\n\n"));
                return None;
            }
        }
        if !errors.is_empty() {
            message(
                owner,
                &errors.into_iter().take(8).collect::<Vec<_>>().join("\n\n"),
            );
        }
        Some((next, positions))
    }
    unsafe fn drop_paths(&mut self, paths: Vec<String>) {
        if paths.is_empty() || self.quitting {
            return;
        }
        self.v2_finish_title(true);
        let prepare = if self.pending_creation.is_some() {
            Err("Finish naming the new folder before adding files.".to_string())
        } else {
            Ok(())
        };
        if let Err(error) = prepare {
            self.status = error.clone();
            message(self.dialog_owner(), &error);
            self.refresh();
            return;
        }
        let Some((paths, converted_positions)) = self.prepare_shared_drop(paths) else {
            self.refresh();
            return;
        };
        let desktop = platform::desktop_directory();
        let personal = std::fs::canonicalize(&desktop).ok();
        let public =
            platform::public_desktop_directory().and_then(|path| std::fs::canonicalize(path).ok());
        let parent = self.trail.last().cloned();
        let mut collected = 0;
        let mut references = 0;
        let mut shared = false;
        let mut errors = Vec::new();
        // Read every source position before any removal can make Explorer reflow.
        let mut positions: HashMap<String, shortcuts::DesktopPosition> = paths
            .iter()
            .filter(|path| {
                let path = Path::new(path);
                let shortcut = matches!(
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("lnk" | "url")
                );
                shortcut
                    && personal.is_some()
                    && path.parent().and_then(|p| std::fs::canonicalize(p).ok()) == personal
            })
            .filter_map(|path| {
                self.desktop_view()
                    .and_then(|view| view.capture(Path::new(path)).ok().flatten())
                    .map(|position| (path.clone(), position))
            })
            .collect();
        for (path, position) in converted_positions {
            positions.insert(path, position);
        }
        let (owned, other): (Vec<_>, Vec<_>) = paths.into_iter().partition(|path| {
            let path = Path::new(path);
            let shortcut = matches!(
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("lnk" | "url")
            );
            shortcut
                && personal.is_some()
                && path.parent().and_then(|p| std::fs::canonicalize(p).ok()) == personal
        });
        if !owned.is_empty() {
            match self.ensure_recovery() {
                Err(error) => errors.push(error),
                Ok(()) => {
                    let selection: Vec<_> = owned
                        .iter()
                        .map(|path| (PathBuf::from(path), positions.get(path).copied()))
                        .collect();
                    let mut moved = Vec::new();
                    let result = desktop_transfer::collect_many_into_with_positions(
                        &mut self.config,
                        &self.path,
                        &desktop,
                        &selection,
                        self.group,
                        parent.as_deref(),
                        |entry| {
                            desktop_positions::notify_removed(&[PathBuf::from(&entry.original)]);
                            moved.push(entry.clone());
                        },
                    );
                    match result {
                        Ok(count) => collected = count,
                        Err(error) => {
                            // Only reposition files actually moved by this attempt
                            // and successfully returned. An occupied original may
                            // belong to another writer and must never be moved.
                            let returned: Vec<_> = moved
                                .iter()
                                .filter(|entry| {
                                    Path::new(&entry.original).is_file()
                                        && !Path::new(&entry.stored).exists()
                                })
                                .filter_map(|entry| {
                                    desktop_positions::notify_created(&[PathBuf::from(
                                        &entry.original,
                                    )]);
                                    entry
                                        .position
                                        .map(|position| (PathBuf::from(&entry.original), position))
                                })
                                .collect();
                            self.apply_desktop_positions(returned);
                            errors.push(error);
                        }
                    }
                }
            }
        }
        // Other files and public Desktop shortcuts remain ordinary references.
        // Save the whole reference selection once, rather than once per icon.
        let mut candidate = self.config.clone();
        let mut added = 0;
        for path in other {
            let source = Path::new(&path);
            let source_parent = source
                .parent()
                .and_then(|path| std::fs::canonicalize(path).ok());
            let shortcut = matches!(
                source
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("lnk" | "url")
            );
            shared |= shortcut && public.is_some() && source_parent == public;
            let destination = candidate.groups.get_mut(self.group).and_then(|group| {
                if let Some(parent) = parent.as_deref() {
                    group
                        .find_item_mut(parent)
                        .filter(|item| item.is_folder())
                        .map(|item| &mut item.children)
                } else {
                    Some(&mut group.items)
                }
            });
            let Some(items) = destination else {
                errors.push("The destination folder no longer exists.".into());
                break;
            };
            if items
                .iter()
                .any(|item| same_shell_path(&item.target, &path))
            {
                continue;
            }
            let name = source
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            items.push(Item::shortcut(name, shell_path(&path)));
            added += 1;
        }
        if added > 0 {
            match candidate.save(&self.path) {
                Ok(()) => {
                    self.config = candidate;
                    references = added;
                }
                Err(error) => errors.push(error),
            }
        }
        self.status =
            format!("{collected} shortcuts moved into the folder. {references} references added.");
        if shared {
            self.status
                .push_str(" Shared Desktop icons remain on the Windows Desktop.");
        }
        if !errors.is_empty() {
            self.status
                .push_str(&format!(" {} items could not be added.", errors.len()));
            message(
                self.dialog_owner(),
                &errors.into_iter().take(8).collect::<Vec<_>>().join("\n\n"),
            );
        }
        self.refresh();
    }
    unsafe fn restore_desktop(&mut self) {
        self.v2_finish_title(true);
        match self.restore_collected(true) {
            Ok(report) => {
                self.status = format!("{} Desktop shortcuts restored.", report.restored);
                if report.remaining != 0 {
                    self.status
                        .push_str(&format!(" {} kept in recovery storage.", report.remaining));
                }
                if !report.warnings.is_empty() {
                    message(self.dialog_owner(), &report.warnings.join("\n\n"));
                }
            }
            Err(error) => {
                self.status = error.clone();
                message(self.dialog_owner(), &error);
            }
        }
        self.refresh();
    }
    unsafe fn reveal_desktop(&mut self) {
        self.close_popup();
        for h in [self.manager, self.popup, self.picker, self.prompt] {
            if !h.0.is_null() {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
        if let Ok(shell) =
            CoCreateInstance::<_, IShellDispatch4>(&Shell, None, CLSCTX_INPROC_SERVER)
        {
            let _ = shell.MinimizeAll();
        }
        for h in self.tiles.clone() {
            self.show_ready_tile(h);
            invalidate(h);
        }
    }
    unsafe fn write_diagnostics(&self) {
        let mut tiles = vec![];
        for (i, h) in self.tiles.iter().enumerate() {
            let parent = GetParent(*h).unwrap_or_default();
            let mut cls = [0u16; 128];
            let n = GetClassNameW(parent, &mut cls).max(0) as usize;
            let mut r = RECT::default();
            let _ = GetWindowRect(*h, &mut r);
            let at = WindowFromPoint(POINT {
                x: (r.left + r.right) / 2,
                y: (r.top + r.bottom) / 2,
            });
            let mut topcls = [0u16; 128];
            let tn = GetClassNameW(at, &mut topcls).max(0) as usize;
            tiles.push(serde_json::json!({"group":self.config.groups.get(i).map(|g|&g.name),"visible":IsWindowVisible(*h).as_bool(),"surface_ready":self.canvases.contains_key(&(h.0 as isize)),"presentations":self.presentations.get(&(h.0 as isize)).copied().unwrap_or(0),"parent_class":String::from_utf16_lossy(&cls[..n]),"screen_rect":[r.left,r.top,r.right,r.bottom],"center_window_class":String::from_utf16_lossy(&topcls[..tn]),"center_is_tile":at==*h}));
        }
        if let Some(p) = self.path.parent() {
            let state = serde_json::json!({
                "tiles": tiles,
                "paint_retry_pending": self.paint_retries.pending().len(),
                "position_retry_pending": self.pending_desktop_positions.len(),
                "position_retry_attempts": self.position_retry_attempts,
                "quitting": self.quitting,
                "paint_retry_timer_active": self.paint_retry_timer_active,
                "tile_mouse_events": self.tile_mouse_events,
                "injected_paint_failures": self.injected_paint_failures,
                "layout_path": self.path,
                "manager_visible": IsWindowVisible(self.manager).as_bool(),
                "manager_scroll": self.manager_scroll,
                "manager_scrollbar_visible": IsWindowVisible(self.manager_scrollbar).as_bool(),
                "popup_visible": IsWindowVisible(self.popup).as_bool(),
                "popup_alpha_surface": surface::has_alpha_surface(self.popup),
                "popup_tint_opacity": surface::POPUP_TINT_OPACITY,
                "creation_pending": self.pending_creation.is_some(),
                "group_count": self.config.groups.len(),
                "popup_pinned": self.popup_pinned,
                "color_panel_visible": IsWindowVisible(self.color_window).as_bool(),
                "hover_opened": self.hover_opened,
                "hover_progress": self.hover_progress,
                "hover_delay_ms": self.config.settings.hover_delay_ms,
                "snap_preview_visible": IsWindowVisible(self.snap_preview.hwnd()).as_bool(),
                "title_edit_visible": IsWindowVisible(self.title_edit).as_bool(),
                "title_host_visible": IsWindowVisible(self.title_host).as_bool(),
                "grid": {"cell_w":self.grid.cell_w, "cell_h":self.grid.cell_h, "origin_x":self.grid.origin_x, "origin_y":self.grid.origin_y}
            });
            let _ = std::fs::write(
                p.join("diagnostics.json"),
                serde_json::to_vec_pretty(&state).unwrap_or_default(),
            );
        }
    }
    unsafe fn save(&mut self) {
        self.config.settings.start_with_windows = platform::startup_enabled();
        if let Err(e) =
            creation::without_pending(&self.config, self.pending_creation.as_ref()).save(&self.path)
        {
            self.status = e;
            invalidate(self.manager);
        }
    }
    unsafe fn commit_structure(&mut self, mut candidate: Config) -> bool {
        candidate.settings.start_with_windows = platform::startup_enabled();
        if let Err(error) = candidate.save(&self.path) {
            self.status = error.clone();
            message(self.dialog_owner(), &error);
            return false;
        }
        self.config = candidate;
        let outcome = desktop_refresh::restore_orphans(&self.config, &self.path);
        self.apply_desktop_positions(outcome.positions);
        match outcome.result {
            Ok(report) if !report.warnings.is_empty() => {
                self.status = report.warnings.join(" ");
                if let Some(root) = self.path.parent() {
                    let _ = std::fs::write(root.join("restore-warnings.txt"), &self.status);
                }
                if report.conflicts > 0 || report.failures > 0 {
                    message(self.dialog_owner(), &self.status);
                }
            }
            Err(error) => {
                self.status = error.clone();
                message(self.dialog_owner(), &error);
            }
            _ => {}
        }
        true
    }
    unsafe fn accept_change(&mut self, previous: Config) -> bool {
        if let Err(e) = self.config.validate() {
            self.config = previous;
            self.status = e.clone();
            message(self.manager, &e);
            self.refresh();
            false
        } else {
            true
        }
    }
    fn clamp_pages(&mut self) {
        let len = self.visible_items().len();
        self.page = self.page.min(len.saturating_sub(1) / 12);
        self.focused = self
            .focused
            .min(len.saturating_sub(self.page * 12).min(12).saturating_sub(1));
    }
    unsafe fn refresh(&mut self) {
        self.clamp_pages();
        self.sync_manager_scrollbar();
        self.v2_relayout_popup(false);
        for h in &self.tiles {
            invalidate(*h)
        }
        invalidate(self.manager);
        invalidate(self.popup);
    }
    unsafe fn clamp_positions(&mut self) {
        let r = platform::primary_work_area();
        let mx = ((r.right - r.left) as f32 / self.scale) as i32 - 128;
        let my = ((r.bottom - r.top) as f32 / self.scale) as i32 - 150;
        for g in &mut self.config.groups {
            g.x = g.x.clamp(8, mx.max(8));
            g.y = g.y.clamp(8, my.max(8));
        }
    }
    unsafe fn rebuild_tiles(&mut self) {
        let _ = KillTimer(self.controller, PAINT_RETRY_TIMER);
        self.paint_retry_timer_active = false;
        self.paint_retries.clear();
        self.cancel_group_drag();
        self.snap_preview.hide();
        self.v2_finish_title(true);
        for h in self.tiles.drain(..) {
            let key = h.0 as isize;
            self.canvases.remove(&key);
            self.presentations.remove(&key);
            self.hits.remove(&key);
            self.hover.remove(&key);
            if IsWindow(h).as_bool() {
                let _ = DestroyWindow(h);
            }
        }
        self.clamp_positions();
        if self.config.settings.snap_to_grid {
            for i in 0..self.config.groups.len() {
                self.v2_snap_group(i);
            }
        }
        let area = platform::primary_work_area();
        for (i, g) in self.config.groups.iter().enumerate() {
            let h = make_window(
                TILE + i as isize,
                &format!("{} — NestDesktop folder", g.name),
                layout::TILE_WIDTH as i32,
                layout::tile_height(self.config.settings.show_item_counts) as i32,
                self.controller,
                WS_POPUP,
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                self.scale,
            );
            if let Err(e) = platform::attach_to_desktop(h) {
                self.status = format!("Desktop connection unavailable: {e}");
                let _ = DestroyWindow(h);
                self.tiles.push(HWND::default());
                continue;
            }
            let (x, y) = platform::desktop_coordinates(
                area.left + (g.x as f32 * self.scale) as i32,
                area.top + (g.y as f32 * self.scale) as i32,
            );
            rounded(
                h,
                (layout::TILE_WIDTH * self.scale) as i32,
                (layout::tile_height(self.config.settings.show_item_counts) * self.scale) as i32,
                (30. * self.scale) as i32,
            );
            let _ = SetWindowPos(
                h,
                HWND_TOP,
                x,
                y,
                (layout::TILE_WIDTH * self.scale) as i32,
                (layout::tile_height(self.config.settings.show_item_counts) * self.scale) as i32,
                SWP_NOACTIVATE,
            );
            DragAcceptFiles(h, true);
            self.tiles.push(h);
        }
        // A layered HWND has no visible bitmap until its first successful ULW.
        // Register every tile first, then present while hidden, then expose it.
        for (index, h) in self.tiles.clone().into_iter().enumerate() {
            if !h.0.is_null() {
                if views::render(self, h, TILE + index as isize) {
                    self.show_ready_tile(h);
                } else {
                    self.queue_tile_paint(h);
                }
            }
        }
        if self.tiles.iter().any(|h| h.0.is_null()) && self.desktop_retries < 20 {
            SetTimer(self.controller, 7, 500, None);
        } else {
            let _ = KillTimer(self.controller, 7);
            if self.tiles.iter().all(|h| !h.0.is_null()) {
                self.desktop_retries = 0;
            }
        }
    }
    unsafe fn show_ready_tile(&mut self, h: HWND) {
        if self.quitting {
            return;
        }
        let Some(index) = self.tiles.iter().position(|tile| *tile == h) else {
            return;
        };
        if !IsWindow(h).as_bool() {
            return;
        }
        if !self.canvases.contains_key(&(h.0 as isize)) {
            self.queue_tile_paint(h);
            return;
        }
        if index == self.group && IsWindowVisible(self.popup).as_bool() {
            return;
        }
        let _ = SetWindowPos(
            h,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
    unsafe fn arm_paint_retry(&mut self) {
        if !self.paint_retry_timer_active && !self.paint_retries.is_empty() {
            self.paint_retry_timer_active =
                SetTimer(self.controller, PAINT_RETRY_TIMER, 100, None) != 0;
            if !self.paint_retry_timer_active {
                self.status = "A desktop folder could not schedule its redraw.".into();
            }
        }
    }
    unsafe fn queue_tile_paint(&mut self, h: HWND) {
        if self.quitting {
            return;
        }
        if !h.0.is_null() && IsWindow(h).as_bool() && self.tiles.contains(&h) {
            self.paint_retries.request(h.0 as isize);
            self.arm_paint_retry();
        }
    }
    unsafe fn tile_paint_completed(&mut self, h: HWND) {
        if self.paint_retries.succeeded(h.0 as isize) {
            self.show_ready_tile(h);
        }
        if self.paint_retries.is_empty() {
            let _ = KillTimer(self.controller, PAINT_RETRY_TIMER);
            self.paint_retry_timer_active = false;
        }
    }
    unsafe fn retry_tile_paints(&mut self) {
        let _ = KillTimer(self.controller, PAINT_RETRY_TIMER);
        self.paint_retry_timer_active = false;
        for key in self.paint_retries.pending() {
            let h = HWND(key as *mut _);
            let Some(index) = self.tiles.iter().position(|tile| *tile == h) else {
                self.paint_retries.remove(key);
                continue;
            };
            if !IsWindow(h).as_bool() {
                self.paint_retries.remove(key);
                continue;
            }
            if views::render(self, h, TILE + index as isize) {
                self.tile_paint_completed(h);
            } else if !self.paint_retries.failed(key) {
                self.status =
                    "A desktop folder could not be drawn. Try reopening NestDesktop.".into();
                invalidate(self.manager);
            }
        }
        self.arm_paint_retry();
    }
    unsafe fn show_manager(&mut self) {
        self.v2_finish_title(true);
        if self.manager.0.is_null() || !IsWindow(self.manager).as_bool() {
            self.manager = make_window(
                MANAGER,
                "NestDesktop — Desktop folders",
                1040,
                690,
                HWND::default(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
                WINDOW_EX_STYLE(0),
                self.scale,
            );
        }
        let _ = ShowWindow(self.manager, SW_SHOWNORMAL);
        // The first ShowWindow may obey the background launcher's SW_HIDE.
        // Showing through SetWindowPos also handles that first tray invocation.
        let _ = SetWindowPos(
            self.manager,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOOWNERZORDER,
        );
        self.v3_sync_hover_delay_field();
        self.sync_manager_scrollbar();
        let _ = SetForegroundWindow(self.manager);
        invalidate(self.manager);
    }
    fn current_title(&self) -> String {
        self.trail
            .last()
            .and_then(|id| self.config.groups.get(self.group)?.find_item(id))
            .map(|i| i.name.clone())
            .or_else(|| self.config.groups.get(self.group).map(|g| g.name.clone()))
            .unwrap_or_else(|| "Folder".into())
    }
    fn current_items(&self) -> Vec<Item> {
        let Some(g) = self.config.groups.get(self.group) else {
            return vec![];
        };
        let mut v = &g.items;
        for id in &self.trail {
            if let Some(it) = v.iter().find(|it| &it.id == id) {
                v = &it.children
            } else {
                return vec![];
            }
        }
        v.clone()
    }
    fn items_mut(&mut self) -> Option<&mut Vec<Item>> {
        let g = self.config.groups.get_mut(self.group)?;
        if let Some(id) = self.trail.last() {
            Some(&mut g.find_item_mut(id)?.children)
        } else {
            Some(&mut g.items)
        }
    }
    fn visible_items(&self) -> Vec<Item> {
        let items = self.current_items();
        let q = self.query.to_lowercase();
        if q.is_empty() {
            return items;
        }
        fn find(v: &[Item], q: &str, out: &mut Vec<Item>) {
            for i in v {
                if i.name.to_lowercase().contains(q) {
                    out.push(i.clone())
                }
                if i.is_folder() {
                    find(&i.children, q, out)
                }
            }
        }
        let mut out = vec![];
        find(&items, &q, &mut out);
        out
    }
    unsafe fn cancel_group_drag(&mut self) {
        self.snap_preview.hide();
        if let Some(drag) = self.drag.take() {
            if let Some(group) = self.config.groups.get_mut(drag.group) {
                (group.x, group.y) = drag.origin;
                if let Some(window) = self.tiles.get(drag.group) {
                    let work = platform::primary_work_area();
                    let (x, y) = platform::desktop_coordinates(
                        work.left + (group.x as f32 * self.scale).round() as i32,
                        work.top + (group.y as f32 * self.scale).round() as i32,
                    );
                    let _ =
                        SetWindowPos(*window, HWND_TOP, x, y, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE);
                }
            }
            let _ = ReleaseCapture();
        }
    }
    unsafe fn position_desktop_drop(&mut self, path: &Path, point: POINT) {
        desktop_positions::notify_created(&[path.to_path_buf()]);
        if let Ok(position) = desktop_positions::screen_position(point) {
            self.queue_desktop_positions(vec![(path.to_path_buf(), position)], false);
        }
    }
    unsafe fn release_item_to_desktop(
        &mut self,
        id: &str,
        point: POINT,
    ) -> std::result::Result<(), String> {
        self.v2_finish_title(true);
        if self.pending_creation.is_some() {
            return Err("Finish naming the new folder before moving files.".into());
        }
        let source = self
            .config
            .find_item(id)
            .cloned()
            .ok_or("The dragged item no longer exists.")?;
        let desktop = platform::desktop_directory();
        if source.is_folder() {
            let color = self
                .config
                .groups
                .iter()
                .find(|group| group.find_item(id).is_some())
                .map(|group| group.color.clone())
                .unwrap_or_else(|| "#A6E5D2".into());
            let work = platform::primary_work_area();
            let (x, y) = self.v2_snap_position(
                ((point.x - work.left) as f32 / self.scale - layout::TILE_WIDTH / 2.).round()
                    as i32,
                ((point.y - work.top) as f32 / self.scale
                    - layout::tile_height(self.config.settings.show_item_counts) / 2.)
                    .round() as i32,
            );
            let mut candidate = self.config.clone();
            let folder = candidate
                .remove_item(id)
                .ok_or("The dragged folder no longer exists.")?;
            candidate.groups.push(Group {
                id: folder.id,
                name: folder.name,
                color,
                x,
                y,
                items: folder.children,
            });
            candidate.save(&self.path)?;
            self.close_popup();
            self.config = candidate;
            self.group = self.config.groups.len() - 1;
            self.trail.clear();
            self.rebuild_tiles();
            self.status = "Folder moved to the Desktop.".into();
            return Ok(());
        }
        let data = self
            .path
            .parent()
            .ok_or("The layout folder is unavailable.")?;
        let managed = desktop_transfer::load_bindings(&self.path)?;
        let active = shortcuts::load_ledger(&data.join("shortcuts"))?;
        let original = managed
            .iter()
            .chain(active.iter())
            .find(|entry| {
                same_shell_path(&source.target, &entry.stored)
                    || same_shell_path(&source.target, &entry.original)
            })
            .map(|entry| PathBuf::from(shell_path(&entry.original)));
        if desktop_transfer::release_to_desktop(&mut self.config, &self.path, id, &desktop)? {
            if let Some(original) = original {
                self.position_desktop_drop(&original, point);
            }
            self.status = "Shortcut returned to the Desktop.".into();
            return Ok(());
        }
        let personal = std::fs::canonicalize(&desktop).ok();
        let public =
            platform::public_desktop_directory().and_then(|path| std::fs::canonicalize(path).ok());
        let source_path = PathBuf::from(&source.target);
        let existing = if source.arguments.is_empty() && source_path.exists() {
            let parent = source_path
                .parent()
                .and_then(|path| std::fs::canonicalize(path).ok());
            (parent.is_some() && (parent == personal || parent == public)).then_some(source_path)
        } else {
            None
        };
        let reference = if let Some(existing) = existing {
            existing
        } else {
            desktop_links::create_reference(&source, &desktop, &data.join("shortcut-staging"))?
        };
        let mut candidate = self.config.clone();
        candidate
            .remove_item(id)
            .ok_or("The dragged item no longer exists.")?;
        // A failed save keeps the group entry and the new Desktop reference.
        // Both remain usable; deleting the new file could race another writer.
        candidate.save(&self.path).map_err(|error| format!(
            "The Desktop shortcut is available, but the item was kept in its folder because the layout could not be saved. {error}"
        ))?;
        self.config = candidate;
        self.position_desktop_drop(&reference, point);
        self.status = "Shortcut moved to the Desktop.".into();
        Ok(())
    }
    unsafe fn finish_item_drag(&mut self, drag: ItemDrag) {
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        self.reorder_before = None;
        self.reorder_at_end = false;
        let under = WindowFromPoint(point);
        let mut destination = None;
        let mut bounds = RECT::default();
        if IsWindowVisible(self.popup).as_bool()
            && GetWindowRect(self.popup, &mut bounds).is_ok()
            && point_in_rect(point, bounds)
            && (under == self.popup || IsChild(self.popup, under).as_bool())
        {
            let mut local = point;
            let _ = ScreenToClient(self.popup, &mut local);
            let scale = GetDpiForWindow(self.popup).max(96) as f32 / 96.;
            let (x, y) = (local.x as f32 / scale, local.y as f32 / scale);
            if self.query.is_empty() {
                let items = self.current_items();
                let l = self.v2_popup_layout();
                let insertion = match reorder_ui::intent(&items, self.page, &l, x, y) {
                    reorder_ui::Intent::Before(id) => Some(Some(id)),
                    reorder_ui::Intent::End => Some(None),
                    reorder_ui::Intent::Outside => {
                        self.refresh();
                        return;
                    }
                    _ => None,
                };
                if let Some(before) = insertion {
                    let result = interaction::reorder_item(
                        &mut self.config,
                        drag.group,
                        self.trail.last().map(String::as_str),
                        &drag.id,
                        before.as_deref(),
                    );
                    match result {
                        Ok(()) => self.save(),
                        Err(error) => self.status = error,
                    }
                    self.refresh();
                    return;
                }
            }
            let target = self
                .hits
                .get(&(self.popup.0 as isize))
                .and_then(|hits| {
                    hits.iter().find(|hit| {
                        x >= hit.r.left && x <= hit.r.right && y >= hit.r.top && y <= hit.r.bottom
                    })
                })
                .and_then(|hit| match &hit.action {
                    Action::Launch(id) => Some(id.clone()),
                    _ => None,
                })
                .or_else(|| self.trail.last().cloned());
            destination = Some((self.group, target));
        } else {
            for (group, tile) in self.tiles.iter().enumerate() {
                if !IsWindowVisible(*tile).as_bool() {
                    continue;
                }
                let mut rect = RECT::default();
                if GetWindowRect(*tile, &mut rect).is_ok()
                    && point_in_rect(point, rect)
                    && (under == *tile || IsChild(*tile, under).as_bool())
                {
                    destination = Some((group, None));
                    break;
                }
            }
        }
        let Some((group, target)) = destination else {
            if desktop_links::point_is_desktop(point) {
                if let Err(error) = self.release_item_to_desktop(&drag.id, point) {
                    self.status = error.clone();
                    message(self.dialog_owner(), &error);
                }
            }
            self.refresh();
            return;
        };
        if drag.group == group && target.as_deref() == Some(drag.id.as_str()) {
            self.refresh();
            return;
        }
        let previous = self.config.clone();
        match interaction::drop_item(
            &mut self.config,
            drag.group,
            &drag.id,
            group,
            target.as_deref(),
        ) {
            Ok(created) => {
                if !self.accept_change(previous) {
                    return;
                }
                self.save();
                if let Some(id) = created {
                    self.open_group(group);
                    let mut path = Vec::new();
                    if folder_path(&self.config.groups[group].items, &id, &mut path) {
                        self.trail = path;
                    }
                    self.refresh();
                    self.show_prompt(PromptKind::RenameItem(id));
                } else {
                    self.status = "App moved to its folder.".into();
                    self.refresh();
                }
            }
            Err(error) => {
                self.status = error.clone();
                self.refresh();
                message(self.popup, &error);
            }
        }
    }
    unsafe fn open_group(&mut self, i: usize) {
        self.v2_open_group(i, false);
    }
    unsafe fn close_popup(&mut self) {
        self.v2_close_popup();
    }
    unsafe fn show_picker(&mut self) {
        if self.config.groups.is_empty() {
            self.begin_group_creation();
            return;
        }
        if self.picker.0.is_null() {
            self.picker = make_window(
                PICKER,
                "NestDesktop — Add applications",
                640,
                590,
                self.dialog_owner(),
                WS_POPUP | WS_BORDER,
                WS_EX_TOOLWINDOW | WS_EX_CONTROLPARENT,
                self.scale,
            );
            rounded(
                self.picker,
                (640. * self.scale) as i32,
                (590. * self.scale) as i32,
                (30. * self.scale) as i32,
            );
            self.picker_search = edit(
                self.picker,
                ID_SEARCH,
                38,
                100,
                560,
                30,
                self.scale,
                self.font,
                "Search applications and desktop shortcuts",
            );
            self.picker_list = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("LISTBOX"),
                w!(""),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_VSCROLL
                    | WS_TABSTOP
                    | WINDOW_STYLE(
                        LBS_NOTIFY as u32 | LBS_EXTENDEDSEL as u32 | LBS_NOINTEGRALHEIGHT as u32,
                    ),
                (32. * self.scale) as i32,
                (153. * self.scale) as i32,
                (576. * self.scale) as i32,
                (330. * self.scale) as i32,
                self.picker,
                HMENU(ID_LIST as *mut _),
                GetModuleHandleW(None).unwrap_or_default(),
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                self.picker_list,
                WM_SETFONT,
                WPARAM(self.font.0 as usize),
                LPARAM(1),
            );
            SendMessageW(
                self.picker_list,
                LB_SETITEMHEIGHT,
                WPARAM(0),
                LPARAM((28. * self.scale) as isize),
            );
        }
        let _ = SetWindowTextW(self.picker_search, w!(""));
        self.filter_picker();
        let _ = ShowWindow(self.picker, SW_SHOW);
        let _ = SetForegroundWindow(self.picker);
        let _ = SetFocus(self.picker_search);
        invalidate(self.picker);
    }
    unsafe fn filter_picker(&mut self) {
        let q = text(self.picker_search).to_lowercase();
        let labels = platform::catalogue_labels(&self.catalogue);
        SendMessageW(self.picker_list, WM_SETREDRAW, WPARAM(0), LPARAM(0));
        SendMessageW(self.picker_list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
        self.picker_map.clear();
        for (i, (n, _)) in self.catalogue.iter().enumerate() {
            if n.to_lowercase().contains(&q) {
                let s = wide(&labels[i]);
                SendMessageW(
                    self.picker_list,
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(s.as_ptr() as isize),
                );
                self.picker_map.push(i);
            }
        }
        SendMessageW(self.picker_list, WM_SETREDRAW, WPARAM(1), LPARAM(0));
        invalidate(self.picker_list);
        invalidate(self.picker);
    }
    unsafe fn add_paths(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        let _ = ShowWindow(self.picker, SW_HIDE);
        self.drop_paths(paths);
    }
    unsafe fn add_selected(&mut self) {
        let n = SendMessageW(self.picker_list, LB_GETSELCOUNT, WPARAM(0), LPARAM(0)).0;
        if n <= 0 {
            return;
        }
        let mut selected = vec![0i32; n as usize];
        SendMessageW(
            self.picker_list,
            LB_GETSELITEMS,
            WPARAM(n as usize),
            LPARAM(selected.as_mut_ptr() as isize),
        );
        let mut paths = vec![];
        for i in selected {
            if let Some(ci) = self.picker_map.get(i as usize) {
                paths.push(self.catalogue[*ci].1.clone());
            }
        }
        let _ = ShowWindow(self.picker, SW_HIDE);
        self.drop_paths(paths);
    }
    unsafe fn show_prompt(&mut self, kind: PromptKind) {
        if !self.prompt.0.is_null() {
            let key = self.prompt.0 as isize;
            self.canvases.remove(&key);
            self.hits.remove(&key);
            self.hover.remove(&key);
            let _ = DestroyWindow(self.prompt);
        }
        let (title, value) = match &kind {
            PromptKind::NewGroup => ("New desktop folder", String::new()),
            PromptKind::RenameGroup(i) => (
                "Rename folder",
                self.config
                    .groups
                    .get(*i)
                    .map(|g| g.name.clone())
                    .unwrap_or_default(),
            ),
            PromptKind::RenameItem(id) => (
                "Rename folder",
                self.config
                    .groups
                    .get(self.group)
                    .and_then(|g| g.find_item(id))
                    .map(|item| item.name.clone())
                    .unwrap_or_default(),
            ),
            PromptKind::NewFolder => ("New nested folder", String::new()),
        };
        self.prompt = make_window(
            PROMPT,
            title,
            440,
            238,
            self.dialog_owner(),
            WS_POPUP | WS_BORDER,
            WS_EX_TOOLWINDOW | WS_EX_CONTROLPARENT,
            self.scale,
        );
        rounded(
            self.prompt,
            (440. * self.scale) as i32,
            (238. * self.scale) as i32,
            (28. * self.scale) as i32,
        );
        let input = edit(
            self.prompt,
            ID_INPUT,
            36,
            91,
            368,
            32,
            self.scale,
            self.font,
            "Folder name",
        );
        SendMessageW(input, EM_SETLIMITTEXT, WPARAM(80), LPARAM(0));
        let v = wide(&value);
        let _ = SetWindowTextW(input, PCWSTR(v.as_ptr()));
        self.prompt_state = Some(Prompt { kind, input });
        let _ = ShowWindow(self.prompt, SW_SHOW);
        let _ = SetForegroundWindow(self.prompt);
        let _ = SetFocus(input);
        SendMessageW(input, EM_SETSEL, WPARAM(0), LPARAM(-1));
    }
    unsafe fn prompt_ok(&mut self) {
        let Some(p) = &self.prompt_state else { return };
        let name = text(p.input).trim().to_string();
        if name.is_empty() {
            return;
        }
        let kind = p.kind.clone();
        let previous = self.config.clone();
        let mut rebuild = false;
        let mut new_group = None;
        match kind {
            PromptKind::NewGroup => {
                let mut g = Group::new(name);
                let n = self.config.groups.len();
                g.x = 48 + (n % 5) as i32 * 136;
                g.y = 64 + (n / 5) as i32 * 166;
                self.config.groups.push(g);
                new_group = Some(n);
                rebuild = true;
            }
            PromptKind::RenameGroup(i) => {
                if let Some(g) = self.config.groups.get_mut(i) {
                    g.name = name
                }
                rebuild = true;
            }
            PromptKind::RenameItem(id) => {
                if let Some(item) = self
                    .config
                    .groups
                    .get_mut(self.group)
                    .and_then(|g| g.find_item_mut(&id))
                {
                    item.name = name;
                }
            }
            PromptKind::NewFolder => {
                if let Some(v) = self.items_mut() {
                    v.push(Item::folder(name));
                }
            }
        }
        if !self.accept_change(previous) {
            return;
        }
        if let Some(i) = new_group {
            self.close_popup();
            self.group = i;
            self.trail.clear();
        }
        if rebuild {
            self.rebuild_tiles();
        }
        let _ = ShowWindow(self.prompt, SW_HIDE);
        self.prompt_state = None;
        self.save();
        self.refresh();
    }
    unsafe fn action(&mut self, a: Action) {
        self.dispatch_action(a);
    }
    unsafe fn menu(&mut self, item: Option<String>) {
        if let Some(id) = item.as_ref() {
            if let Some(entry) = self
                .config
                .groups
                .get(self.group)
                .and_then(|g| g.find_item(id))
                .cloned()
            {
                if !entry.is_folder() {
                    let mut point = POINT::default();
                    let _ = GetCursorPos(&mut point);
                    let owner = self.dialog_owner();
                    match shell_menu::show(
                        owner,
                        &entry.target,
                        point,
                        &[(0x7001, "Move to folder…"), (0x7002, "Remove from folder")],
                    ) {
                        Ok(Some(0x7001)) => {
                            self.post(Action::MoveItem(id.clone()));
                            return;
                        }
                        Ok(Some(0x7002)) => {
                            self.post(Action::RemoveItem(id.clone()));
                            return;
                        }
                        Ok(_) => return,
                        Err(error) => self.status = format!("Windows menu unavailable: {error}"),
                    }
                }
            }
        }
        let m = CreatePopupMenu().unwrap_or_default();
        let mut actions = vec![];
        let mut add = |menu: HMENU, name: &str, a: Action| {
            let n = wide(name);
            let _ = AppendMenuW(menu, MF_STRING, 100 + actions.len(), PCWSTR(n.as_ptr()));
            actions.push(a);
        };
        if let Some(id) = item {
            add(m, "Open", Action::Launch(id.clone()));
            if self
                .config
                .groups
                .get(self.group)
                .and_then(|g| g.find_item(&id))
                .map(Item::is_folder)
                .unwrap_or(false)
            {
                add(m, "Rename folder…", Action::RenameItem(id.clone()));
            }
            add(m, "Move to folder…", Action::MoveItem(id.clone()));
            add(m, "Remove from folder", Action::RemoveItem(id));
        } else {
            add(m, "Add applications…", Action::AddApps);
            add(
                m,
                "Move desktop shortcuts into folder…",
                Action::CollectDesktop,
            );
            add(
                m,
                "Restore collected desktop shortcuts",
                Action::RestoreDesktop,
            );
            add(m, "New desktop folder…", Action::NewGroup);
            add(m, "New nested folder…", Action::NewSubfolder);
            add(m, "Rename desktop folder…", Action::RenameGroup);
            add(m, "Folder color…", Action::ChooseColor);
            add(m, "Folder settings…", Action::Manage);
            add(m, "Settings…", Action::Settings);
            add(m, "Remove desktop folder…", Action::RemoveGroup);
        }
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let _ = SetForegroundWindow(self.controller);
        let id = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            p.x,
            p.y,
            0,
            self.controller,
            None,
        )
        .0;
        let _ = DestroyMenu(m);
        if id >= 100 {
            if let Some(a) = actions.get(id as usize - 100) {
                self.post(a.clone())
            }
        }
    }
    unsafe fn post(&self, a: Action) {
        let ptr = Box::into_raw(Box::new(a));
        if PostMessageW(self.controller, WM_DO, WPARAM(0), LPARAM(ptr as isize)).is_err() {
            drop(Box::from_raw(ptr));
        }
    }
    unsafe fn move_menu(&mut self) {
        let m = CreatePopupMenu().unwrap_or_default();
        let mut dest = vec![];
        fn nested(
            m: HMENU,
            v: &[Item],
            gi: usize,
            prefix: &str,
            d: &mut Vec<(usize, Option<String>)>,
            skip: &str,
        ) {
            for it in v {
                if it.is_folder() && it.id != skip {
                    let label = format!("{prefix} / {}", it.name);
                    let t = wide(&label);
                    unsafe {
                        let _ = AppendMenuW(m, MF_STRING, 200 + d.len(), PCWSTR(t.as_ptr()));
                    }
                    d.push((gi, Some(it.id.clone())));
                    nested(m, &it.children, gi, &label, d, skip);
                }
            }
        }
        let skip = self.context_item.clone().unwrap_or_default();
        for (i, g) in self.config.groups.iter().enumerate() {
            let t = wide(&g.name);
            let _ = AppendMenuW(m, MF_STRING, 200 + dest.len(), PCWSTR(t.as_ptr()));
            dest.push((i, None));
            nested(m, &g.items, i, &g.name, &mut dest, &skip);
        }
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let _ = SetForegroundWindow(self.controller);
        let id = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            p.x,
            p.y,
            0,
            self.controller,
            None,
        )
        .0;
        let _ = DestroyMenu(m);
        if id >= 200 {
            if let Some((gi, folder)) = dest.get(id as usize - 200) {
                if let Some(id) = self.context_item.take() {
                    let previous = self.config.clone();
                    if let Some(item) = self
                        .config
                        .groups
                        .get_mut(self.group)
                        .and_then(|g| g.remove_item(&id))
                    {
                        if let Some(fid) = folder {
                            if let Some(f) = self.config.groups[*gi].find_item_mut(fid) {
                                f.children.push(item)
                            }
                        } else {
                            self.config.groups[*gi].items.push(item)
                        }
                        if self.accept_change(previous) {
                            self.save();
                            self.refresh();
                        }
                    }
                }
            }
        }
    }
    unsafe fn add_tray(&self) {
        let mut d = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.controller,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: brand::small_icon(GetDpiForWindow(self.controller)),
            ..Default::default()
        };
        let t = wide("NestDesktop • Desktop folders");
        d.szTip[..t.len()].copy_from_slice(&t);
        let _ = Shell_NotifyIconW(NIM_ADD, &d);
    }
    unsafe fn remove_tray(&self) {
        let d = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.controller,
            uID: 1,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &d);
    }
    unsafe fn uninstall(&mut self) {
        self.v2_finish_title(true);
        self.v3_finish_hover_delay(true, false);
        let setup = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Programs\\NestDesktop\\NestDesktop-Setup.exe"));
        if let Some(setup) = setup.filter(|path| path.is_file()) {
            if let Err(error) = platform::launch(&setup.to_string_lossy(), "--uninstall") {
                message(self.manager, &error);
            }
        } else {
            message(self.manager, "NestDesktop is running as a portable application. To remove it, quit NestDesktop and delete its program folder. Your saved layout is kept in the layout folder.");
        }
    }
    unsafe fn tray_menu(&mut self) {
        let m = CreatePopupMenu().unwrap_or_default();
        let _ = AppendMenuW(m, MF_STRING, 10, w!("Manage desktop folders"));
        let _ = AppendMenuW(m, MF_STRING, 11, w!("Settings"));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, MF_STRING, 12, w!("Quit NestDesktop"));
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let _ = SetForegroundWindow(self.controller);
        let n = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            p.x,
            p.y,
            0,
            self.controller,
            None,
        )
        .0;
        let _ = DestroyMenu(m);
        match n {
            10 => self.post(Action::Manage),
            11 => self.post(Action::Settings),
            12 => self.post(Action::Quit),
            _ => {}
        }
        let _ = PostMessageW(self.controller, WM_NULL, WPARAM(0), LPARAM(0));
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = shell_menu::handle_message(msg, wp, lp) {
        return result;
    }
    if msg == WM_NCCREATE {
        let c = &*(lp.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(h, GWLP_USERDATA, c.lpCreateParams as isize);
        return DefWindowProcW(h, msg, wp, lp);
    }
    let kind = GetWindowLongPtrW(h, GWLP_USERDATA);
    if kind == MANAGER && msg == WM_CTLCOLORSCROLLBAR {
        return manager_scroll::scrollbar_background(HDC(wp.0 as *mut _));
    }
    let handled = APP.with(|cell| {
        let Ok(mut b) = cell.try_borrow_mut() else {
            return if defer_if_busy(h, msg, wp, lp) {
                Some(LRESULT(0))
            } else {
                None
            };
        };
        let Some(a) = b.as_mut() else { return None };
        if a.taskbar_created != 0 && msg == a.taskbar_created {
            a.add_tray();
            let _ = PostMessageW(a.controller, WM_REFRESH, WPARAM(0), LPARAM(0));
            return Some(LRESULT(0));
        }
        match msg {
            WM_PAINT if kind > 0 => {
                views::paint(a, h, kind);
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_CLOSE => {
                if kind == 0 {
                    a.action(Action::Quit)
                } else if kind == COLOR_PANEL {
                    a.v4_close_colors(true);
                } else if kind == TITLE_HOST {
                    a.v2_finish_title(true);
                } else if kind == POPUP || kind == MANAGER {
                    if kind == MANAGER {
                        a.v3_finish_hover_delay(true, false);
                    }
                    a.close_popup();
                    let _ = ShowWindow(h, SW_HIDE);
                } else if kind < TILE {
                    let _ = ShowWindow(h, SW_HIDE);
                    if kind == PROMPT {
                        a.prompt_state = None;
                    }
                }
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                a.canvases.remove(&(h.0 as isize));
                a.presentations.remove(&(h.0 as isize));
                a.paint_retries.remove(h.0 as isize);
                if a.paint_retries.is_empty() {
                    let _ = KillTimer(a.controller, PAINT_RETRY_TIMER);
                    a.paint_retry_timer_active = false;
                }
                a.hits.remove(&(h.0 as isize));
                Some(LRESULT(0))
            }
            WM_SIZE => {
                if kind == MANAGER {
                    a.hits.remove(&(h.0 as isize));
                    a.hover.remove(&(h.0 as isize));
                    a.v3_sync_hover_delay_field();
                    a.sync_manager_scrollbar();
                }
                invalidate(h);
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                let _ = PostMessageW(a.controller, WM_REFRESH, WPARAM(0), LPARAM(0));
                Some(LRESULT(0))
            }
            WM_REFRESH if a.quitting => Some(LRESULT(0)),
            WM_TIMER if a.quitting && wp.0 != POSITION_RETRY_TIMER => Some(LRESULT(0)),
            WM_REFRESH => {
                a.v4_close_colors(false);
                a.desktop_session = None;
                a.desktop_retries = 0;
                a.scale = GetDpiForSystem().max(96) as f32 / 96.;
                a.grid = desktop_grid::metrics((a.scale * 96.) as u32);
                a.rebuild_tiles();
                a.v2_relayout_popup(false);
                a.v3_sync_hover_delay_field();
                a.sync_manager_scrollbar();
                invalidate(a.manager);
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == 7 => {
                a.desktop_retries = a.desktop_retries.saturating_add(1);
                a.rebuild_tiles();
                Some(LRESULT(0))
            }
            WM_TIMER if (8..=11).contains(&wp.0) => {
                a.v2_timer(wp.0);
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == PAINT_RETRY_TIMER => {
                a.retry_tile_paints();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == POSITION_RETRY_TIMER => {
                a.position_retry_attempts = a.position_retry_attempts.saturating_add(1);
                a.try_desktop_positions();
                if a.quitting && a.pending_desktop_positions.is_empty() {
                    PostQuitMessage(0);
                }
                Some(LRESULT(0))
            }
            WM_TITLE_FINISH => {
                if lp.0 as usize == a.title_generation {
                    a.v2_finish_title(wp.0 != 0);
                }
                Some(LRESULT(0))
            }
            WM_DELAY_FINISH => {
                if GetFocus() != a.hover_delay_edit {
                    a.v3_finish_hover_delay(true, false);
                }
                Some(LRESULT(0))
            }
            WM_CATALOGUE => {
                a.catalogue = *Box::from_raw(lp.0 as *mut Vec<(String, String)>);
                a.catalogue_ready = true;
                if !a.picker.0.is_null() {
                    a.filter_picker()
                }
                Some(LRESULT(0))
            }
            WM_REQUEST => {
                if a.quitting && wp.0 != 4 {
                    return Some(LRESULT(0));
                }
                match wp.0 {
                    1 => a.reveal_desktop(),
                    2 => a.begin_group_creation(),
                    3 => {
                        a.settings = true;
                        a.show_manager()
                    }
                    4 => a.write_diagnostics(),
                    5 => a.restore_desktop(),
                    6 => a.open_group(lp.0 as usize),
                    7 => a.v2_edit_group(lp.0 as usize),
                    8 => {
                        if let Some(root) = a.path.parent() {
                            let _ = transparency_check::verify(
                                &a.tiles,
                                &root.join("transparency-check.json"),
                            );
                        }
                    }
                    9 => a.action(Action::Manage),
                    10 => {
                        if let Some(root) = a.path.parent() {
                            let _ = popup_alpha_check::verify(
                                a.popup,
                                &root.join("popup-transparency-check.json"),
                            );
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_DO => {
                a.action(*Box::from_raw(lp.0 as *mut Action));
                Some(LRESULT(0))
            }
            WM_TRAY => {
                match lp.0 as u32 {
                    WM_LBUTTONUP => a.reveal_desktop(),
                    WM_RBUTTONUP => a.tray_menu(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_CTLCOLORLISTBOX | WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as *mut _);
                SetTextColor(dc, COLORREF(0x00fcf3f4));
                let control = HWND(lp.0 as *mut _);
                let (color, brush) = if control == a.search
                    || control == a.picker_search
                    || control == a.hover_delay_edit
                    || control == a.color_hex_edit
                {
                    (0x00402d25, a.search_brush)
                } else if control == a.title_edit {
                    if GetParent(control).unwrap_or_default() == a.popup {
                        (0x004a352c, a.title_brush)
                    } else {
                        (0x00574237, a.tile_title_brush)
                    }
                } else {
                    (0x0033241e, a.brush)
                };
                SetBkColor(dc, COLORREF(color));
                Some(LRESULT(brush.0 as isize))
            }
            WM_COMMAND => {
                let id = wp.0 & 0xffff;
                let code = (wp.0 >> 16) & 0xffff;
                if id == ID_COLOR_HEX && code == EN_CHANGE as usize {
                    a.v4_color_text_changed();
                } else if id == ID_SEARCH && code == EN_CHANGE as usize {
                    if kind == POPUP {
                        a.query = text(a.search);
                        a.page = 0;
                        a.focused = 0;
                        a.v2_relayout_popup(false);
                        invalidate(a.popup)
                    } else if kind == PICKER {
                        a.filter_picker()
                    }
                } else if id == ID_LIST && code == LBN_DBLCLK as usize {
                    a.add_selected()
                } else if id == ID_HOVER_DELAY && code == EN_KILLFOCUS as usize {
                    let _ = PostMessageW(a.controller, WM_DELAY_FINISH, WPARAM(0), LPARAM(0));
                } else if id == ID_TITLE && code == EN_KILLFOCUS as usize {
                    let _ = PostMessageW(
                        a.controller,
                        WM_TITLE_FINISH,
                        WPARAM(1),
                        LPARAM(a.title_generation as isize),
                    );
                }
                Some(LRESULT(0))
            }
            WM_DROPFILES => {
                let drop = HDROP(wp.0 as *mut _);
                let n = DragQueryFileW(drop, u32::MAX, None);
                let mut paths = vec![];
                for i in 0..n {
                    let len = DragQueryFileW(drop, i, None);
                    let mut p = vec![0u16; len as usize + 1];
                    DragQueryFileW(drop, i, Some(&mut p));
                    paths.push(String::from_utf16_lossy(&p[..len as usize]));
                }
                DragFinish(drop);
                if kind >= TILE {
                    a.close_popup();
                    a.group = (kind - TILE) as usize;
                    a.trail.clear()
                }
                a.drop_paths(paths);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                if kind >= TILE {
                    a.tile_mouse_events = a.tile_mouse_events.saturating_add(1);
                }
                if kind == COLOR_PANEL && a.color_drag.is_some() {
                    let (x, y) = pos(lp);
                    let scale = GetDpiForWindow(h).max(96) as f32 / 96.;
                    a.v4_drag_color(x as f32 / scale, y as f32 / scale);
                    return Some(LRESULT(0));
                }
                if let Some(d) = a.item_drag.as_mut() {
                    let mut point = POINT::default();
                    let _ = GetCursorPos(&mut point);
                    if (point.x - d.start.x).abs() + (point.y - d.start.y).abs() > 6 {
                        d.moved = true;
                    }
                }
                if let Some(d) = a.drag.as_mut() {
                    if kind >= TILE {
                        let mut p = POINT::default();
                        let _ = GetCursorPos(&mut p);
                        let dx = p.x - d.start.x;
                        let dy = p.y - d.start.y;
                        if dx.abs() + dy.abs() > 5 {
                            d.moved = true
                        }
                        if d.moved {
                            let r = platform::primary_work_area();
                            let mx = ((r.right - r.left) as f32 / a.scale) as i32
                                - layout::TILE_WIDTH as i32;
                            let my = ((r.bottom - r.top) as f32 / a.scale) as i32
                                - layout::tile_height(a.config.settings.show_item_counts) as i32;
                            let nx =
                                (d.origin.0 + (dx as f32 / a.scale) as i32).clamp(0, mx.max(0));
                            let ny =
                                (d.origin.1 + (dy as f32 / a.scale) as i32).clamp(0, my.max(0));
                            if let Some(g) = a.config.groups.get_mut(d.group) {
                                g.x = nx;
                                g.y = ny;
                            }
                            let (x, y) = platform::desktop_coordinates(
                                r.left + (nx as f32 * a.scale) as i32,
                                r.top + (ny as f32 * a.scale) as i32,
                            );
                            let _ =
                                SetWindowPos(h, HWND_TOP, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
                        }
                    }
                }
                if kind >= TILE {
                    if let Some((x, y, color)) = a
                        .drag
                        .as_ref()
                        .filter(|d| d.moved)
                        .and_then(|d| a.config.groups.get(d.group))
                        .map(|g| (g.x, g.y, g.color.clone()))
                    {
                        if a.config.settings.snap_to_grid {
                            let (x, y) = a.v2_snap_position(x, y);
                            let work = platform::primary_work_area();
                            let left = work.left + (x as f32 * a.scale).round() as i32;
                            let top = work.top + (y as f32 * a.scale).round() as i32;
                            let bounds = RECT {
                                left,
                                top,
                                right: left + (layout::TILE_WIDTH * a.scale).round() as i32,
                                bottom: top
                                    + (layout::tile_height(a.config.settings.show_item_counts)
                                        * a.scale)
                                        .round() as i32,
                            };
                            let _ = a.snap_preview.show(bounds, a.scale, &color);
                        } else {
                            a.snap_preview.hide();
                        }
                    }
                }
                let (px, py) = pos(lp);
                let s = GetDpiForWindow(h).max(96) as f32 / 96.;
                let (x, y) = (px as f32 / s, py as f32 / s);
                let hi = a
                    .hits
                    .get(&(h.0 as isize))
                    .and_then(|v| {
                        v.iter().position(|hit| {
                            x >= hit.r.left
                                && x <= hit.r.right
                                && y >= hit.r.top
                                && y <= hit.r.bottom
                        })
                    })
                    .unwrap_or(usize::MAX);
                let hover_changed = a.hover.get(&(h.0 as isize)) != Some(&hi);
                if hover_changed {
                    a.hover.insert(h.0 as isize, hi);
                    invalidate(h);
                }
                if kind >= TILE {
                    a.v2_hover_tile((kind - TILE) as usize, layout::tile_preview_contains(x, y));
                } else if kind == POPUP {
                    if a.item_drag.as_ref().map(|d| d.moved).unwrap_or(false) && a.query.is_empty()
                    {
                        let intent = reorder_ui::intent(
                            &a.current_items(),
                            a.page,
                            &a.v2_popup_layout(),
                            x,
                            y,
                        );
                        let (before, end) = match intent {
                            reorder_ui::Intent::Before(id) => (Some(id), false),
                            reorder_ui::Intent::End => (None, true),
                            _ => (None, false),
                        };
                        if before != a.reorder_before || end != a.reorder_at_end {
                            a.reorder_before = before;
                            a.reorder_at_end = end;
                            invalidate(h);
                        }
                    }
                    if hover_changed && a.item_drag.is_none() && a.animation.is_none() {
                        let path = a
                            .hits
                            .get(&(h.0 as isize))
                            .and_then(|v| v.get(hi))
                            .and_then(|hit| match &hit.action {
                                Action::Launch(id) => a
                                    .config
                                    .groups
                                    .get(a.group)?
                                    .find_item(id)
                                    .filter(|it| !it.is_folder())
                                    .map(|it| it.target.clone()),
                                _ => None,
                            });
                        a.menu_warm_path = path;
                        let _ = KillTimer(a.controller, 11);
                        if a.menu_warm_path.is_some() {
                            SetTimer(a.controller, 11, 180, None);
                        }
                    }
                }
                let mut t = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: h,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut t);
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                a.hover.remove(&(h.0 as isize));
                if kind >= TILE {
                    a.v2_leave_tile((kind - TILE) as usize);
                }
                if kind == POPUP {
                    a.menu_warm_path = None;
                    let _ = KillTimer(a.controller, 11);
                    a.v2_timer(9);
                }
                invalidate(h);
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN if kind == COLOR_PANEL => {
                let (x, y) = pos(lp);
                let scale = GetDpiForWindow(h).max(96) as f32 / 96.;
                let (x, y) = (x as f32 / scale, y as f32 / scale);
                let control = a.v4_color_control_at(x, y);
                a.color_pressed = control;
                if let Some(
                    c @ (palette_view::Control::Hue | palette_view::Control::SaturationValue),
                ) = control
                {
                    a.color_pressed = None;
                    a.color_drag = Some(c);
                    SetCapture(h);
                    a.v4_drag_color(x, y);
                } else if control.is_some() {
                    SetCapture(h);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP if kind == COLOR_PANEL => {
                let pressed = a.color_pressed.take();
                let dragged = a.color_drag.take().is_some();
                let (x, y) = pos(lp);
                let scale = GetDpiForWindow(h).max(96) as f32 / 96.;
                let released = a.v4_color_control_at(x as f32 / scale, y as f32 / scale);
                let _ = ReleaseCapture();
                if !dragged && pressed == released {
                    if let Some(control) = released {
                        a.action(Action::Palette(control));
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN if kind >= TILE => {
                let i = (kind - TILE) as usize;
                a.v2_leave_tile(i);
                if let Some(g) = a.config.groups.get(i) {
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    a.drag = Some(Drag {
                        group: i,
                        start: p,
                        origin: (g.x, g.y),
                        moved: false,
                    });
                    SetCapture(h);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN if kind == POPUP => {
                let (px, py) = pos(lp);
                let scale = GetDpiForWindow(h).max(96) as f32 / 96.;
                let (x, y) = (px as f32 / scale, py as f32 / scale);
                let item = a
                    .hits
                    .get(&(h.0 as isize))
                    .and_then(|hits| {
                        hits.iter().find(|hit| {
                            x >= hit.r.left
                                && x <= hit.r.right
                                && y >= hit.r.top
                                && y <= hit.r.bottom
                        })
                    })
                    .and_then(|hit| match &hit.action {
                        Action::Launch(id) => Some(id.clone()),
                        _ => None,
                    });
                if let Some(id) = item {
                    let mut point = POINT::default();
                    let _ = GetCursorPos(&mut point);
                    a.item_drag = Some(ItemDrag {
                        id,
                        group: a.group,
                        start: point,
                        moved: false,
                    });
                    SetCapture(h);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN if kind == MANAGER => {
                let _ = SetFocus(h);
                if !a.popup_pinned {
                    a.close_popup();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                a.snap_preview.hide();
                if let Some(drag) = a.item_drag.take() {
                    let _ = ReleaseCapture();
                    if drag.moved {
                        a.finish_item_drag(drag);
                        return Some(LRESULT(0));
                    }
                }
                if kind >= TILE {
                    let _ = ReleaseCapture();
                    if let Some(d) = a.drag.take() {
                        if d.moved {
                            a.v2_snap_group(d.group);
                            if let Some(g) = a.config.groups.get(d.group) {
                                let r = platform::primary_work_area();
                                let (x, y) = platform::desktop_coordinates(
                                    r.left + (g.x as f32 * a.scale) as i32,
                                    r.top + (g.y as f32 * a.scale) as i32,
                                );
                                let _ = SetWindowPos(
                                    h,
                                    HWND_TOP,
                                    x,
                                    y,
                                    0,
                                    0,
                                    SWP_NOSIZE | SWP_NOACTIVATE,
                                );
                            }
                            a.save();
                            invalidate(a.manager);
                            return Some(LRESULT(0));
                        }
                    }
                }
                let (px, py) = pos(lp);
                let s = GetDpiForWindow(h).max(96) as f32 / 96.;
                let (x, y) = (px as f32 / s, py as f32 / s);
                let action = a
                    .hits
                    .get(&(h.0 as isize))
                    .and_then(|v| {
                        v.iter().find(|hit| {
                            x >= hit.r.left
                                && x <= hit.r.right
                                && y >= hit.r.top
                                && y <= hit.r.bottom
                        })
                    })
                    .map(|hit| hit.action.clone());
                if let Some(action) = action {
                    if kind == PICKER && matches!(action, Action::PromptCancel) {
                        let _ = ShowWindow(h, SW_HIDE);
                    } else {
                        a.action(action)
                    }
                }
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                a.color_pressed = None;
                a.color_drag = None;
                a.cancel_group_drag();
                a.item_drag = None;
                a.reorder_before = None;
                a.reorder_at_end = false;
                Some(LRESULT(0))
            }
            WM_CANCELMODE => {
                a.color_pressed = None;
                a.color_drag = None;
                a.cancel_group_drag();
                a.item_drag = None;
                a.reorder_before = None;
                a.reorder_at_end = false;
                let _ = ReleaseCapture();
                Some(LRESULT(0))
            }
            WM_SETCURSOR if wp.0 == h.0 as usize => {
                let editable = a
                    .hover
                    .get(&(h.0 as isize))
                    .and_then(|i| a.hits.get(&(h.0 as isize))?.get(*i))
                    .map(|hit| matches!(hit.action, Action::EditTitle))
                    .unwrap_or(false);
                if editable {
                    SetCursor(LoadCursorW(None, IDC_IBEAM).unwrap_or_default());
                    Some(LRESULT(1))
                } else {
                    None
                }
            }
            WM_RBUTTONUP => {
                if kind >= TILE {
                    a.close_popup();
                    a.group = (kind - TILE) as usize;
                    a.trail.clear();
                    a.menu(None)
                } else if kind == POPUP {
                    let (px, py) = pos(lp);
                    let s = GetDpiForWindow(h).max(96) as f32 / 96.;
                    let (x, y) = (px as f32 / s, py as f32 / s);
                    let item = a
                        .hits
                        .get(&(h.0 as isize))
                        .and_then(|v| {
                            v.iter().find(|hit| {
                                x >= hit.r.left
                                    && x <= hit.r.right
                                    && y >= hit.r.top
                                    && y <= hit.r.bottom
                            })
                        })
                        .and_then(|hit| match &hit.action {
                            Action::Launch(id) => Some(id.clone()),
                            _ => None,
                        });
                    if item.is_some() {
                        a.menu(item)
                    }
                }
                Some(LRESULT(0))
            }
            WM_VSCROLL if kind == MANAGER && lp.0 == a.manager_scrollbar.0 as isize => {
                a.scroll_manager((wp.0 & 0xffff) as u16);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL if kind == MANAGER && !a.settings => {
                a.manager_wheel(((wp.0 >> 16) as u16 as i16) as i32);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL if kind == POPUP => {
                let d = ((wp.0 >> 16) as u16 as i16) as i32;
                a.action(if d < 0 {
                    Action::PageNext
                } else {
                    Action::PageBack
                });
                Some(LRESULT(0))
            }
            WM_CHAR if kind == POPUP && wp.0 >= 32 && wp.0 != 127 => {
                let _ = SetFocus(a.search);
                let _ = PostMessageW(a.search, WM_CHAR, wp, lp);
                Some(LRESULT(0))
            }
            WM_KEYDOWN
                if kind == MANAGER
                    && !a.settings
                    && matches!(wp.0 as u16, 0x21 | 0x22 | 0x24 | 0x23 | 0x26 | 0x28) =>
            {
                let command = match wp.0 as u16 {
                    0x21 => SB_PAGEUP,
                    0x22 => SB_PAGEDOWN,
                    0x24 => SB_TOP,
                    0x23 => SB_BOTTOM,
                    0x26 => SB_LINEUP,
                    _ => SB_LINEDOWN,
                };
                a.scroll_manager(command.0 as u16);
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                match wp.0 as u16 {
                    0x1b => {
                        if kind == POPUP {
                            if a.trail.is_empty() {
                                a.close_popup()
                            } else {
                                a.action(Action::Back)
                            }
                        } else {
                            let _ = ShowWindow(h, SW_HIDE);
                        }
                    }
                    0x0d if kind == PROMPT => a.prompt_ok(),
                    0x0d if kind == POPUP => {
                        if let Some(it) = a.visible_items().get(a.page * 12 + a.focused) {
                            a.action(Action::Launch(it.id.clone()))
                        }
                    }
                    0x25 | 0x26 | 0x27 | 0x28 if kind == POPUP => {
                        let n = a.visible_items().len().saturating_sub(a.page * 12).min(12);
                        let d = match wp.0 {
                            0x25 => -1,
                            0x26 => -4,
                            0x27 => 1,
                            _ => 4,
                        };
                        a.focused =
                            (a.focused as i32 + d).clamp(0, n.saturating_sub(1) as i32) as usize;
                        invalidate(h)
                    }
                    0x22 if kind == POPUP => a.action(Action::PageNext),
                    0x21 if kind == POPUP => a.action(Action::PageBack),
                    0x46 if kind == POPUP => {
                        if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                            let _ = SetFocus(a.search);
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE if kind == POPUP => {
                if (wp.0 & 0xffff) == WA_INACTIVE as usize {
                    let next = HWND(lp.0 as *mut _);
                    if !a.popup_pinned
                        && a.title_target.is_none()
                        && next != a.color_window
                        && next != a.picker
                        && next != a.prompt
                        && next != a.controller
                        && next != a.manager
                    {
                        a.close_popup();
                    }
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE if kind == COLOR_PANEL => {
                if (wp.0 & 0xffff) == WA_INACTIVE as usize {
                    let next = HWND(lp.0 as *mut _);
                    a.v4_close_colors(false);
                    if !a.popup_pinned
                        && ![a.popup, a.manager, a.picker, a.prompt, a.controller].contains(&next)
                    {
                        a.close_popup();
                    }
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE if kind == TITLE_HOST => {
                if (wp.0 & 0xffff) == WA_INACTIVE as usize && a.title_target.is_some() {
                    let _ = PostMessageW(
                        a.controller,
                        WM_TITLE_FINISH,
                        WPARAM(1),
                        LPARAM(a.title_generation as isize),
                    );
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    });
    handled.unwrap_or_else(|| DefWindowProcW(h, msg, wp, lp))
}

/// A guarded UI-only fixture. It never calls the shared move or collection APIs.
unsafe fn shared_choice_fixture() -> Option<i32> {
    if !std::env::args().any(|arg| arg == "--shared-choice-fixture") {
        return None;
    }
    let data = std::env::var_os("NESTDESKTOP_DATA_DIR").map(PathBuf::from);
    let result = (|| -> std::result::Result<(), String> {
        let data = data
            .as_ref()
            .ok_or("An isolated choice fixture directory is required.")?;
        if !data.join(".shared-choice-fixture").is_file() {
            return Err("Missing shared-choice fixture marker.".into());
        }
        let root = std::fs::canonicalize(data.join("items")).map_err(|error| error.to_string())?;
        let bytes = std::fs::read(data.join("shared-choice-paths.json"))
            .map_err(|error| error.to_string())?;
        if bytes.len() > 64 * 1024 {
            return Err("Choice fixture is too large.".into());
        }
        let paths: Vec<PathBuf> =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if paths.is_empty()
            || paths.len() > 32
            || paths.iter().any(|path| {
                !path.is_file()
                    || std::fs::canonicalize(path)
                        .ok()
                        .and_then(|value| value.parent().map(Path::to_path_buf))
                        .as_ref()
                        != Some(&root)
            })
        {
            return Err(
                "Choice fixture items must be direct files in its isolated items directory.".into(),
            );
        }
        let selected = shared_ui::choose(HWND::default(), &paths);
        let state =
            serde_json::json!({"confirmed":selected.is_some(),"selected":selected,"items":paths});
        std::fs::write(
            data.join("shared-choice-result.json"),
            serde_json::to_vec_pretty(&state).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })();
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            if let Some(data) = data {
                let _ = std::fs::write(data.join("shared-choice-error.txt"), error);
            }
            2
        }
    })
}
fn main() {
    if let Some(code) = recovery::run_if_requested() {
        std::process::exit(code);
    }
    // Explicit synthetic-process fixture: no UI, singleton or real Desktop.
    if std::env::args().any(|arg| arg == "--recovery-fixture") {
        let result = (|| -> std::result::Result<(), String> {
            let data = std::env::var_os("NESTDESKTOP_DATA_DIR")
                .map(PathBuf::from)
                .ok_or("A synthetic recovery profile is required.")?;
            if !data.join(".recovery-fixture").is_file() {
                return Err("The synthetic recovery fixture marker is missing.".into());
            }
            let _guard = recovery::LifetimeGuard::acquire(&data)?;
            let _watchdog = recovery::spawn_watchdog(&data)?;
            std::fs::write(data.join("fixture-ready"), "ready").map_err(|e| e.to_string())?;
            loop {
                std::thread::park();
            }
        })();
        if result.is_err() {
            std::process::exit(2);
        }
        return;
    }
    unsafe {
        let restore_requested = std::env::args().any(|argument| argument == "--restore-shortcuts");
        let requested_folder = std::env::args().find_map(|argument| {
            argument
                .strip_prefix("--open-folder-index=")
                .and_then(|index| index.parse::<usize>().ok())
        });
        let requested_edit = std::env::args().find_map(|argument| {
            argument
                .strip_prefix("--edit-folder-index=")
                .and_then(|index| index.parse::<usize>().ok())
        });
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Some(code) = shared_choice_fixture() {
            render::release_resources();
            CoUninitialize();
            std::process::exit(code);
        }
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: brand::icon(32),
            lpszClassName: w!("NestDesktopWindow"),
            ..Default::default()
        };
        RegisterClassW(&class);
        if let Ok(h) = FindWindowW(w!("NestDesktopWindow"), w!("NestDesktopController")) {
            if !h.0.is_null() {
                if std::env::args().any(|s| s == "--quit") {
                    let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
                } else if restore_requested {
                    let _ = PostMessageW(h, WM_REQUEST, WPARAM(5), LPARAM(0));
                } else if let Some(index) = requested_folder {
                    let _ = PostMessageW(h, WM_REQUEST, WPARAM(6), LPARAM(index as isize));
                } else if let Some(index) = requested_edit {
                    let _ = PostMessageW(h, WM_REQUEST, WPARAM(7), LPARAM(index as isize));
                } else if !std::env::args().any(|s| s == "--background") {
                    let request = if std::env::args().any(|s| s == "--new-folder") {
                        2
                    } else if std::env::args().any(|s| s == "--settings") {
                        3
                    } else if std::env::args().any(|s| s == "--diagnostics") {
                        4
                    } else if std::env::args().any(|s| s == "--verify-transparency") {
                        8
                    } else if std::env::args().any(|s| s == "--verify-popup-transparency") {
                        10
                    } else if std::env::args().any(|s| s == "--reveal") {
                        1
                    } else if std::env::args().any(|s| s == "--manage") {
                        9
                    } else {
                        0
                    };
                    let _ = PostMessageW(h, WM_REQUEST, WPARAM(request), LPARAM(0));
                }
                return;
            }
        }
        if std::env::args().any(|s| s == "--quit") {
            return;
        }
        let controller = make_window(
            0,
            "NestDesktopController",
            0,
            0,
            HWND::default(),
            WS_POPUP,
            WS_EX_TOOLWINDOW,
            1.,
        );
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let data = std::env::var_os("NESTDESKTOP_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("NestDesktop"));
        let path = data.join("layout.json");
        let _profile_guard = match recovery::LifetimeGuard::acquire(&data) {
            Ok(guard) => guard,
            Err(error) => {
                message(HWND::default(), &error);
                return;
            }
        };
        if restore_requested {
            match restore_profile(&data, true) {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    let _ = std::fs::create_dir_all(&data);
                    let _ = std::fs::write(data.join("restore-error.txt"), error);
                    std::process::exit(2)
                }
            }
        }
        // A freshly crash-restored icon may still occupy a temporary Explorer
        // slot. Only previously visible originals can replace saved positions.
        let visible_before_recovery: Vec<_> = desktop_transfer::load_bindings(&path)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| {
                Path::new(&entry.original).is_file() && !Path::new(&entry.stored).exists()
            })
            .map(|entry| entry.original)
            .collect();
        let startup_restore = quiet_restore(&data);
        let mut config = match Config::load(&path) {
            Ok(c) => c,
            Err(e) => {
                message(HWND::default(),&format!("Your layout could not be read. It has been left unchanged.\n\n{e}\n\nRestore layout.json.bak in your NestDesktop data folder to recover it."));
                return;
            }
        };
        let mut startup_status = startup_restore.err().unwrap_or_default();
        let mut watchdog = None;
        let mut desktop_session = None;
        let next_status = match desktop_transfer::load_bindings(&path) {
            Ok(bindings) if !bindings.is_empty() => match recovery::spawn_watchdog(&data) {
                Ok(helper) => {
                    watchdog = Some(helper);
                    desktop_session = desktop_positions::DesktopSession::connect().ok();
                    let positions: Vec<_> = bindings
                        .iter()
                        .filter_map(|entry| {
                            let original = Path::new(&entry.original);
                            if !visible_before_recovery
                                .iter()
                                .any(|path| same_shell_path(path, &entry.original))
                                || Path::new(&entry.stored).exists()
                                || !original.is_file()
                            {
                                return None;
                            }
                            desktop_positions::notify_created(&[original.to_path_buf()]);
                            desktop_session
                                .as_ref()
                                .and_then(|session| session.capture(original).ok().flatten())
                                .map(|point| (entry.original.clone(), point))
                        })
                        .collect();
                    let previously_stored: Vec<_> = bindings
                        .iter()
                        .filter(|entry| Path::new(&entry.stored).is_file())
                        .cloned()
                        .collect();
                    let result = desktop_transfer::reconcile_startup_with_positions(
                        &mut config,
                        &path,
                        &platform::desktop_directory(),
                        &positions,
                    );
                    let removed: Vec<_> = bindings
                        .iter()
                        .filter(|entry| {
                            !Path::new(&entry.original).exists()
                                && Path::new(&entry.stored).is_file()
                        })
                        .map(|entry| PathBuf::from(&entry.original))
                        .collect();
                    desktop_positions::notify_removed(&removed);
                    let returned: Vec<_> = bindings
                        .iter()
                        .filter(|entry| {
                            Path::new(&entry.original).is_file()
                                && !Path::new(&entry.stored).exists()
                        })
                        .map(|entry| PathBuf::from(&entry.original))
                        .collect();
                    desktop_positions::notify_created(&returned);
                    let positions: Vec<_> = previously_stored
                        .iter()
                        .filter(|entry| {
                            Path::new(&entry.original).is_file()
                                && !Path::new(&entry.stored).exists()
                        })
                        .filter_map(|entry| {
                            entry
                                .position
                                .map(|point| (PathBuf::from(&entry.original), point))
                        })
                        .collect();
                    let placement_warnings = desktop_refresh::finish_positions_blocking(positions);
                    match result {
                        Ok(mut report) => {
                            report.warnings.extend(placement_warnings);
                            report.warnings.join(" ")
                        }
                        Err(error) => error,
                    }
                }
                Err(error) => error,
            },
            Err(error) => error,
            _ => String::new(),
        };
        if !next_status.is_empty() {
            if !startup_status.is_empty() {
                startup_status.push(' ');
            }
            startup_status.push_str(&next_status);
        }
        let scale = GetDpiForSystem().max(96) as f32 / 96.;
        let font = CreateFontW(
            (-15. * scale) as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe UI"),
        );
        let brush = CreateSolidBrush(COLORREF(0x0033241e));
        let app = App {
            pending_creation: None,
            watchdog,
            config,
            path,
            controller,
            manager: HWND::default(),
            popup: HWND::default(),
            color_window: HWND::default(),
            color_hex_edit: HWND::default(),
            color_session: None,
            color_drag: None,
            color_pressed: None,
            color_sync: false,
            picker: HWND::default(),
            prompt: HWND::default(),
            tiles: vec![],
            canvases: HashMap::new(),
            presentations: HashMap::new(),
            paint_retries: paint_retry::RetryQueue::default(),
            paint_retry_timer_active: false,
            tile_mouse_events: 0,
            fail_first_tile_paint: std::env::args().any(|arg| arg == "--render-fixture")
                && std::env::args().any(|arg| arg == "--fail-first-tile-paint")
                && std::env::var_os("NESTDESKTOP_DATA_DIR").is_some()
                && data.join(".render-fixture").is_file(),
            injected_paint_failures: 0,
            desktop_session,
            pending_desktop_positions: Vec::new(),
            position_retry_attempts: 0,
            quitting: false,
            hits: HashMap::new(),
            hover: HashMap::new(),
            group: 0,
            trail: vec![],
            search: HWND::default(),
            query: String::new(),
            page: 0,
            focused: 0,
            catalogue: vec![],
            catalogue_ready: false,
            picker_list: HWND::default(),
            picker_search: HWND::default(),
            picker_map: vec![],
            prompt_state: None,
            drag: None,
            item_drag: None,
            scale,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            settings: false,
            manager_scroll: 0,
            manager_scrollbar: HWND::default(),
            manager_wheel_delta: 0,
            font,
            brush,
            status: startup_status,
            context_item: None,
            desktop_retries: 0,
            popup_pinned: false,
            hovered_group: None,
            hover_progress: 0.,
            hover_started: None,
            hover_opened: false,
            open_animation_progress: 1.,
            animation: None,
            title_edit: HWND::default(),
            hover_delay_edit: HWND::default(),
            snap_preview: snap_preview::SnapPreview::default(),
            title_host: HWND::default(),
            title_target: None,
            title_generation: 0,
            title_font: HFONT::default(),
            reorder_before: None,
            reorder_at_end: false,
            grid: desktop_grid::metrics(GetDpiForSystem()),
            menu_warm_path: None,
            search_brush: CreateSolidBrush(COLORREF(0x00402d25)),
            title_brush: CreateSolidBrush(COLORREF(0x004a352c)),
            tile_title_brush: CreateSolidBrush(COLORREF(0x00574237)),
        };
        TASKBAR_MESSAGE.with(|message| message.set(app.taskbar_created));
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        APP.with(|cell| {
            if let Some(a) = cell.borrow_mut().as_mut() {
                if restore_requested {
                    a.restore_desktop();
                    PostQuitMessage(0);
                } else {
                    a.add_tray();
                    a.rebuild_tiles();
                    if std::env::args().any(|arg| arg == "--render-fixture")
                        && std::env::var_os("NESTDESKTOP_DATA_DIR").is_some()
                        && data.join(".render-fixture").is_file()
                    {
                        a.write_diagnostics();
                        let _ = std::fs::copy(
                            data.join("diagnostics.json"),
                            data.join("startup-paint.json"),
                        );
                    }
                    if let Some(index) = requested_folder {
                        a.open_group(index)
                    } else if std::env::args().any(|s| s == "--settings") {
                        a.settings = true;
                        a.show_manager()
                    } else if std::env::args().any(|s| s == "--new-folder") {
                        a.begin_group_creation()
                    } else if std::env::args().any(|s| s == "--reveal") {
                        a.reveal_desktop()
                    } else if std::env::args().any(|s| s == "--manage") {
                        a.action(Action::Manage)
                    }
                    a.save();
                }
            }
        });
        if !restore_requested {
            let cid = controller.0 as isize;
            std::thread::spawn(move || {
                let cat = platform::discover_shortcuts();
                let ptr = Box::into_raw(Box::new(cat));
                if PostMessageW(
                    HWND(cid as *mut _),
                    WM_CATALOGUE,
                    WPARAM(0),
                    LPARAM(ptr as isize),
                )
                .is_err()
                {
                    drop(Box::from_raw(ptr));
                }
            });
        }
        let mut msg = MSG::default();
        loop {
            drain_deferred();
            if GetMessageW(&mut msg, None, 0, 0).0 <= 0 {
                break;
            }
            let mut consumed = false;
            if msg.message == WM_KEYDOWN {
                let root = GetAncestor(msg.hwnd, GA_ROOT);
                APP.with(|cell| {
                    if let Ok(mut b) = cell.try_borrow_mut() {
                        if let Some(a) = b.as_mut() {
                            if msg.wParam.0 == 0x41
                                && GetKeyState(VK_CONTROL.0 as i32) < 0
                                && [
                                    a.color_hex_edit,
                                    a.hover_delay_edit,
                                    a.title_edit,
                                    a.search,
                                    a.picker_search,
                                ]
                                .contains(&msg.hwnd)
                            {
                                SendMessageW(msg.hwnd, EM_SETSEL, WPARAM(0), LPARAM(-1));
                                consumed = true;
                            } else if root == a.color_window && msg.wParam.0 == VK_ESCAPE.0 as usize
                            {
                                a.v4_close_colors(true);
                                consumed = true;
                            } else if root == a.color_window && msg.wParam.0 == VK_RETURN.0 as usize
                            {
                                a.v4_color_action(palette_view::Control::Apply);
                                consumed = true;
                            } else if msg.hwnd == a.color_hex_edit
                                && msg.wParam.0 == VK_TAB.0 as usize
                            {
                                a.v4_color_text_changed();
                                let _ = SetFocus(a.color_window);
                                consumed = true;
                            } else if msg.wParam.0 == VK_ESCAPE.0 as usize
                                && (a.drag.is_some() || a.item_drag.is_some())
                            {
                                a.cancel_group_drag();
                                a.item_drag = None;
                                let _ = ReleaseCapture();
                                consumed = true;
                            } else if msg.hwnd == a.hover_delay_edit
                                && [
                                    VK_RETURN.0 as usize,
                                    VK_ESCAPE.0 as usize,
                                    VK_TAB.0 as usize,
                                ]
                                .contains(&msg.wParam.0)
                            {
                                if a.v3_finish_hover_delay(
                                    msg.wParam.0 != VK_ESCAPE.0 as usize,
                                    true,
                                ) {
                                    let _ = SetFocus(a.manager);
                                }
                                consumed = true;
                            } else if msg.hwnd == a.title_edit
                                && IsWindowVisible(a.title_edit).as_bool()
                                && [
                                    VK_RETURN.0 as usize,
                                    VK_ESCAPE.0 as usize,
                                    VK_TAB.0 as usize,
                                ]
                                .contains(&msg.wParam.0)
                            {
                                a.v2_finish_title(msg.wParam.0 != VK_ESCAPE.0 as usize);
                                consumed = true;
                            } else if msg.wParam.0 == VK_ESCAPE.0 as usize
                                && [a.popup, a.manager, a.picker, a.prompt, a.title_host]
                                    .contains(&root)
                            {
                                if root == a.popup && !a.trail.is_empty() {
                                    a.action(Action::Back)
                                } else if root == a.popup {
                                    a.close_popup();
                                } else {
                                    let _ = ShowWindow(root, SW_HIDE);
                                }
                                consumed = true;
                            } else if root == a.prompt && msg.wParam.0 == VK_RETURN.0 as usize {
                                a.prompt_ok();
                                consumed = true;
                            } else if root == a.picker && msg.wParam.0 == VK_RETURN.0 as usize {
                                a.add_selected();
                                consumed = true;
                            } else if root == a.popup
                                && msg.hwnd == a.search
                                && msg.wParam.0 == VK_DOWN.0 as usize
                            {
                                let _ = SetFocus(a.popup);
                                consumed = true;
                            }
                        }
                    }
                });
            }
            if !consumed {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        APP.with(|cell| {
            if let Some(mut a) = cell.borrow_mut().take() {
                for id in [7, 8, 9, 10, 11, PAINT_RETRY_TIMER, POSITION_RETRY_TIMER] {
                    let _ = KillTimer(a.controller, id);
                }
                shell_menu::clear_cache();
                a.remove_tray();
                // Render targets and their COM resources must precede HWND and
                // apartment teardown, including eager first-frame surfaces.
                a.canvases.clear();
                for h in a.tiles.drain(..) {
                    let _ = DestroyWindow(h);
                }
                let _ = DeleteObject(a.font);
                let _ = DeleteObject(a.brush);
                let _ = DeleteObject(a.search_brush);
                let _ = DeleteObject(a.title_brush);
                let _ = DeleteObject(a.tile_title_brush);
                if !a.title_font.0.is_null() {
                    let _ = DeleteObject(a.title_font);
                }
            }
        });
        render::release_resources();
        CoUninitialize();
    }
}
