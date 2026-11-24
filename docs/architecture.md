# Architecture

NestDesktop is a per-user Windows application with a Rust process and native
desktop windows. The installer is a separate, short-lived .NET Framework
program; it is not part of the resident application.

Normal application startup quietly attaches only the primary-desktop folder
tiles, without opening a manager, changing focus or minimizing other windows.
The manager is an optional settings view and is never the main application
surface. `--background` has the same quiet behaviour at sign-in.
Only an explicit `--reveal` action uses `IShellDispatch::MinimizeAll` to show
the desktop. Setup passes this option when the user chooses to show desktop
folders after installation.
`--new-folder` creates a folder through the existing instance when it is
already running. `--settings` opens the optional manager/settings view.

## Main components

- **Desktop tiles:** small primary-desktop windows with cached app icon
  mosaics, labels and input handling.
- **Expanded group:** an app grid and nested folder navigation, opened in
  response to the user's interaction.
- **Windows platform layer:** app launching, icon extraction, file selection,
  sign-in startup and attachment to the Explorer desktop.
- **Model:** a versioned local layout describing groups and launch targets.
  Launch targets are references to programs or shortcuts, not copies of
  installed applications.
- **Persistence:** bounded JSON data with a recovery-oriented save path.

## Rendering and idle work

The renderer shares its Direct2D factory, DirectWrite resources and Windows
Imaging Component resources. These small static HWND surfaces use software
Direct2D rendering to avoid allocating a GPU device for each window. Text and
icons are drawn natively on demand. Desktop tile pixels use premultiplied alpha
and cached WIC/DIB surfaces for `UpdateLayeredWindow`. Normal top-level views
keep native child controls and use DWM's outer clipping; their backgrounds do
not add a second, conflicting corner radius.

The executable embeds an `asInvoker` compatibility and PerMonitorV2 DPI manifest.
Windows 8/8.1 and Windows 10/11 compatibility declarations are required for the
layered-child desktop surfaces, even when the offscreen alpha pixels are valid.
`build.rs` writes a minimal COFF resource object with the manifest; no resource
compiler, Windows SDK or runtime build tools are needed by the released app.

Version 0.2.1 has short, bounded interaction animations: a configurable hover
dwell, initially 450 ms, and about 140 ms expansion centered over the preview.
Only the colored preview starts the dwell. Its rounded hit test excludes the
name, tile margins and uncolored corner sectors. The dwell timer rechecks the
actual cursor position and window beneath it. The preview highlight is a radial
center-out fill clipped to the rounded preview shape. The preview is hidden
while expanded. Timers run only during dwell, animation or an active
hover-opened view's pointer tracking; they stop when idle or pinned. There is
no continuous render loop or blur.

The process waits for window messages while idle. Preserve that design: do not
scan the desktop on a timer, continuously repaint stationary tiles, or retain
an unbounded icon cache. Measure private memory, working set and GPU memory
separately; executable size does not establish runtime memory usage.

## Compact views and customization

Desktop item counts are a serde-defaulted setting, off by default. Compact
previews are 104 × 114 logical pixels; enabling desktop counts adds twenty
logical pixels of height. Expanded-folder and manager-card totals remain visible.
Expanded views use an adaptive 240–392 logical-pixel height, omit folder-type
subtitles and keep padding between the native search edit and its search icon.
Expanded folder titles expose an in-place native edit on click with a hover
highlight. Closed-tile labels open their folder; they are not direct rename
targets. The context-menu rename command remains available. Clicking the folder
icon in an expanded desktop-folder header opens a native color popover. The
button and glyph use shared rectangles with the same center.

Explorer icon spacing is read through `IShellWindows`, `IShellBrowser` and
`IFolderView`, with system icon-spacing metrics as a fallback. The read uses
supported COM marshaling, not pointer-bearing cross-process ListView messages.
Reads are event driven, not polled. Tile dimensions determine the grid-cell
span. This alignment does not rearrange Explorer's own icons. A click-through
layered outline previews the drag destination using the same snapped-position
calculation that commits the drop. Its pixels are cached by size and color;
moving it changes only its HWND position, and it is hidden after the drag.

Colors include twelve stronger presets and a native HSV/Hex editor. Up to
sixteen custom presets can be saved, updated or removed. The panel keeps an
in-memory folder-color draft until Apply; saved presets commit independently.
Closing or Escape discards only the unapplied folder-color draft. Existing schema-1 layouts
remain readable because the added settings have defaults.

## Interaction and native menus

App dragging distinguishes insertion edges from center targets. Edges reorder
siblings without changing identity or launch arguments. A center drop moves
into a folder or creates a nested folder around two apps. Model changes are
transactional and validate limits before replacing the saved layout.

Hover opening is optional and defaults to 450 ms. A native seconds edit beside
the toggle accepts 0.10–5.00 seconds with a dot or comma decimal separator;
settings persist whole milliseconds. Existing explicit saved values are retained
unless changed. A hover-opened view closes on pointer exit unless pinned; a click-opened view closes through the usual
outside-click, Escape or close action. Pin state is shown beside the close
button. Opening stays centered on the source preview and within the primary
work area.

Native item menus cache up to eight same-apartment Shell handler objects for
sixty seconds, keyed by path, owner and file modification data. Only an app the
user pauses on is primed after about 180 ms. A fresh `HMENU` and
`QueryContextMenu` call preserve dynamic commands on every right-click. Edits
and view closure can release the cache. The first load of a third-party Shell
extension can still take time; it is not a guaranteed zero-latency operation.

## Shortcut collection

Adding an app normally stores a launch reference. An explicit collection action
can move selected `.lnk` and `.url` shortcuts directly from the current user's
Desktop into `%LOCALAPPDATA%\NestDesktop\shortcuts`. It does not recursively
collect arbitrary files, folders or Public Desktop items. Existing Windows
folders are not converted into app-folder tiles.

`shortcuts/collection.json` records original and stored paths. Restoration
preserves original-name conflicts and retains the corresponding ledger entries
and stored shortcuts for recovery. The uninstall helper invokes
`nest-desktop.exe --restore-shortcuts --background` and waits for its result
before removing program files. A nonzero status or timeout stops removal. It
never deletes the profile containing saved layouts and collected shortcuts.

## Desktop integration

Explorer desktop parenting uses implementation details such as `WorkerW` and
`SHELLDLL_DefView`; these are not a stable public widget-host contract. Keep
this code isolated in the platform layer. Changes to Windows or Explorer can
require compatibility updates.

The first compatibility checks are Show desktop, Explorer restarting, a
primary-display change, mixed DPI, sleep/resume and virtual desktops. A
successful run on one PC does not establish support for every configuration.
The app should surface a useful recovery message when desktop attachment is
unavailable, instead of appearing behind the wallpaper or over other apps.

## Installation and user data

The release package contains the native executable and a small setup program.
The setup program copies files to `%LOCALAPPDATA%\Programs\NestDesktop`, creates
a Start menu shortcut and registers a per-user uninstall entry. The optional
Run key is `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value
`NestDesktop`, with a quoted executable path followed by `--background`.

When the installed setup executable receives `--uninstall`, it copies itself
to a uniquely named temporary executable, launches that helper and exits. The
helper displays the uninstall UI before removing the installed setup file, so
Windows is not asked to delete the executable that is currently loaded. The
small temporary helper is retained for normal temporary-file cleanup; no
recursive temporary-directory deletion, elevation or reboot is used.

The desktop right-click command is registered only for the current user at
`HKCU\Software\Classes\DesktopBackground\Shell\NestDesktop`, labelled
`New app folder`. Its command is the quoted executable path followed by
`--new-folder`. Uninstall removes that exact application-owned key. No taskbar
customization or Explorer restart is required by the installer.

`%LOCALAPPDATA%\NestDesktop\layout.json` remains outside the installation
directory and is retained on uninstall. Normal portable, installed and startup
launches resolve this same per-user profile, independent of the executable
location. Test profiles may use an explicit data-directory override, but a
review launch must not rely on a temporary override if ordinary reopening is
expected to show that layout. Quitting commits an active inline name edit and
valid settings before closing. The application reads the current
Windows startup registration when presenting the start-at-sign-in preference.


## Version 0.2.2 surfaces

Opening animates only the window background for the bounded expansion. The
miniature mosaic is no longer enlarged or translated; the full icon grid paints
once the window reaches its final bounds. The search edit is revealed then too.

The color popover is a normal DWM-rounded native HWND owned by the expanded
folder (or settings/controller). It uses Direct2D gradients and a native Hex
edit, with no browser runtime, polling, idle rendering or additional timers.
The folder hover-close timer is suspended during color editing and restored
when the panel closes. Folder identity is retained by ID while applying a draft.

## Version 0.2.3 desktop ownership and surfaces

`desktop_transfer.rs` separates the active pre-move recovery ledger from the
managed-shortcut list. A successful incoming personal Desktop shortcut drop
writes ownership and the layout transactionally. Normal quit restores active
files but keeps ownership; startup replays recovery, then reacquires only
managed shortcuts that are still referenced. Explicit Restore forgets ownership.
No-overwrite name conflicts retain both files and their active recovery records.
Malformed layouts remain unchanged while valid ledger entries can still return
their files. Public Desktop, executables and user documents are never collected.

The UI thread holds a per-profile named mutex for its lifetime. Before a move,
`recovery.rs` launches the same executable as a companion and waits for a ready
handshake. The companion opens the exact parent process identity (PID plus
creation time), then sleeps on its process handle. After exit it acquires the
profile mutex only if a replacement organizer has not already taken over.
It runs restoration without initializing COM, desktop windows or timers.
Startup recovery also covers machine shutdown, where the companion may not run.

`desktop_links.rs` exports ordinary references as no-overwrite Windows shortcuts;
collected items return through their original recovery binding. An existing
personal/shared desktop reference is reused. Outgoing pointer drops are accepted
only on the Shell process's actual Progman/WorkerW desktop surface. Explorer
folder windows do not qualify. Optional positioning uses `IFolderView`, avoiding
cross-process pointer messages. Nested folders can become desktop groups while
keeping the same child identities.

New folders are drafts in `creation.rs`. The icon and native name editor appear
immediately, Enter/focus loss commits, and Escape removes the draft. Persistence
and export omit drafts until accepted. Floating editors keep desktop tiles on
their per-pixel surfaces, while open-folder editors remain owned by the popup.

Collapsed previews draw their exact chosen RGB at 72% alpha. Open groups request
the Windows 11 transient DWM backdrop and extend it into the client area before
creating their premultiplied-alpha Direct2D HWND target. Older systems fall back
to 94% constant window opacity. Desktop tiles still use per-pixel composition;
the fallback popup is never sent through UpdateLayeredWindow. Native search and
name controls remain ordinary EDIT windows. Rendering is event driven.

## Version 0.2.4 identity and direct translucency

The shared mark follows Setup's four-color rounded-square geometry. A generated
ICO contains nine independently antialiased sizes from 16 to 256 pixels. The
resource builder embeds RT_ICON/RT_GROUP_ICON alongside the original manifest.
Native window small/large icons and the notification-area icon use those shared
HICON resources, while the Settings sidebar draws the same geometry. Setup embeds
the same asset. No external icon file or additional runtime is required.

Merely accepting a system Acrylic backdrop did not establish visible transmission:
the controlled black/white underlay test measured zero change on this machine.
The popup now uses direct premultiplied alpha with DWMSBT_NONE, opting into
DWMWA_REDIRECTIONBITMAP_ALPHA on supported Windows builds. Its dark body is 82%
opaque, with an 18% header overlay. Icons, labels and child EDIT controls remain
opaque. Older systems fall back to 220/255 constant window opacity.

`popup_alpha_check.rs` is an explicit, bounded development check. It creates an
owned black/white underlay directly beneath an owned visible popup, synchronizes
DWM and compares six background pixels, then removes its probe. It adds no idle
timer, capture loop, desktop scanning or data movement. This verifies actual
background transmission rather than a requested DWM attribute alone.

## Version 0.2.6 first presentation

Desktop tile creation no longer relies on a future WM_PAINT to supply its first
layered bitmap. After all new handles have their final positions and dimensions,
`views::render` presents their complete surfaces directly, then exposes them.
The WM_PAINT wrapper still owns BeginPaint/EndPaint for native paint delivery.

Busy paint requests are validated and coalesced until the App borrow ends. A
per-window rendering guard prevents presentation from generating a repaint
feedback loop. Failed tile presentation enters a bounded queue with three
100-ms retry opportunities; the timer exists only while work remains. Successful
frames and pending work are exposed in diagnostics so native visibility flags
cannot be mistaken for a painted bitmap. Guarded synthetic fixtures test startup
without any pointer event and can inject a single failed first presentation.

Shutdown clears native render targets before destroying tile windows, then
clears the UI-thread factory cache before CoUninitialize. The repeated-startup
test exercises both initial presentation and graceful shutdown.
