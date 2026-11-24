# Using NestDesktop

Android-inspired app folders directly on your normal Windows desktop. Small
folder tiles show miniature app previews and open into app grids. Groups can
contain nested folders. There is no launcher window to keep open. The manager
is an optional settings view; normal use happens on the primary desktop.

This is an unsigned alpha for Windows 10/11 x64. Desktop integration needs
validation against additional Windows versions and display configurations.

Version 0.2.11 uses a dark appearance and compact desktop tiles. Desktop item
counts are off by default: tiles are 104 × 114 logical pixels, or 104 × 134
when desktop counts are enabled. Open folders and manager cards retain their
item totals. Windows display scaling is applied. Theme and arbitrary tile-size
selection are not exposed yet. Short interaction animations stop when idle.

## Install

1. Extract the entire release ZIP to a folder.
2. Open **NestDesktop-Setup.exe**.
3. Choose whether to start NestDesktop at sign-in, then select **Install**.

Normal launches and sign-in startup attach the folder tiles quietly in the
background. They do not open the manager, change focus or minimize your
windows. Each tile is drawn before it is shown, so it appears without needing
to be hovered first. View your normal desktop to use the tiles. If **Show desktop folders
after installation** is checked, setup explicitly reveals the desktop once.
Installation also adds **New app folder** to the desktop's right-click menu
for your user account.

Installation is per-user in `%LOCALAPPDATA%\Programs\NestDesktop` and requires
no administrator privileges. Other devices need the release package, not Rust
or other development tools. The setup program uses the .NET Framework that is
included with supported Windows 10/11 installations.

For a portable trial, open `nest-desktop.exe` directly from the extracted
package. Portable, installed and sign-in launches use the same per-user
configuration under `%LOCALAPPDATA%\NestDesktop`, independent of the executable
location. Quitting commits an active folder-name edit and valid settings before
closing. Sign-in startup uses the executable's location, so install before
enabling it if the extracted package may be moved or deleted.

Launching `nest-desktop.exe --reveal` explicitly shows the desktop. The
`--settings` option opens the optional settings window; normal launches do not.
**Manage desktop folders** in the tray menu opens the folder overview, or use
`--manage`. It always selects the overview and restores a hidden/minimized window.
The overview uses a vertical scrollbar when its groups exceed the visible area.
Use the mouse wheel, scrollbar thumb, or Page Up/Down and Home/End to navigate.
The sidebar, header and footer stay fixed.
The tray, taskbar, window title bar and Settings header use the same four-color
NestDesktop icon as the installer, with the app's lavender behind the four cells.

## First use

- Right-click an empty part of the desktop and choose **New app folder**.
  On Windows 11, this entry may be under **Show more options**.
- The new tile appears immediately with its name selected. Enter accepts the
  name; Escape cancels creation. **New folder** inside an open group works the
  same way, without opening another dialog.
- Select a folder tile to open it; select an app to launch it. The compact
  expanded view opens over the preview. Its background expands first; app icons
  appear at their final size and position once it settles.
- With **Open folders on hover** enabled, pause over the colored miniature
  preview. Its highlight fills evenly outward from the center and the folder
  opens after the chosen delay, initially **0.45 seconds**. The name and outer
  tile margins do not start hover opening. The preview is hidden while expanded.
  A view opened this way closes when the pointer leaves it. Select the pin next
  to the close button to keep it open; select the pin again to unpin it.
- Clicking a closed folder's name opens the folder. To rename it, open the folder
  and click its title, or use **Rename desktop folder** from the context menu.
  The expanded title highlights on hover to indicate editing.
- Click the folder icon in an expanded desktop folder's header to choose a
  preset or mix a custom colour in the colour panel.
- Drag an app onto another app to create a nested folder, then name it.
  Dragging into an existing folder moves the app there.
- To reorder apps, drop near an item's left or right edge; a position indicator
  shows where the app will go. Dropping at an item's center groups the apps.
- Right-click an app for its normal Windows Shell menu. On Windows 11 this
  uses the classic menu, including installed Shell extensions.
- Right-click a tile for **Add applications**, **New desktop folder**,
  **New nested folder**, rename, color and settings commands.
- Open the optional manager when you need layout settings, or use the tray
  menu to quit. A separate program window is not needed for everyday use.
- Drag a personal desktop shortcut into a group to move it there. Drag it back
  onto the desktop to return it, or onto another group to move it between groups.

## Appearance and positioning

Settings include **Show item counts on desktop**, **Snap to the desktop grid**,
and **Open folders on hover**. The counts option affects only collapsed desktop
previews; turning it off keeps them shorter. Open-folder totals and manager-card
totals remain visible.

The hover row contains a seconds field next to the On/Off control. Its default
is **0.45 seconds**; enter a value from **0.10 to 5.00 seconds**, then press Enter
or leave the field to save it. Both `0.45` and `0,45` are accepted. Escape
restores the saved value.

Snapping aligns groups to Explorer's current icon spacing when it is available,
with system icon spacing as a fallback. Larger groups occupy a multiple of the
underlying grid cells. Snapping does not move or reserve the spaces used by
ordinary Windows desktop icons. While dragging a group with snapping enabled,
a translucent outline previews its exact landing position before release.
The preview disappears when the drag ends or is cancelled.

Click the folder icon in the expanded header, or choose **Folder color** from
its context menu, to open the color panel. Choose from twelve stronger presets,
or use the color square, hue slider and Hex field to mix your own color.
**Apply** changes the folder. Closing the panel or pressing Escape cancels the
unapplied folder-color draft.

The desktop preview uses the selected color at 72% opacity, preserving its hue
instead of replacing it with a darkened shade. Open groups have a dark translucent
background: 82% opacity leaves the desktop slightly visible through it. Text and
icons stay opaque on supported Windows 11 builds. Older systems use an 86% opaque
window as a translucency fallback. No background render loop is needed.

**Your colors** holds up to sixteen saved presets. Choose a colour in the editor,
then click the **+** circle to save it immediately. The next empty circle becomes
the add button. Clicking a saved colour selects it for the folder; **Apply**
confirms the folder change.

When all sixteen slots are filled, hover over a saved colour to reveal a small
**+** badge. Click that badge to replace its slot with the current editor colour;
the circle's centre still selects the saved colour normally. Saving a colour
that is already present selects its existing slot instead of duplicating it.
**Remove** deletes the selected saved swatch.

**Reset color** selects the same default blue used for a newly created desktop
group. Select **Apply** to confirm it, or close the panel to keep the existing
folder colour. Saved presets remain available after restarting and travel with
exported layouts. Repeated legacy white swatches are shown as one white preset.

The expanded header's folder glyph and hover highlight share a centered button.
The expanded view has no folder-type subtitle, and its search field has space
around the magnifying-glass icon.

Only hover-opened views close immediately on pointer exit. A click-opened view
uses the normal outside-click or Escape closing behaviour. Pinning keeps either
kind open until it is unpinned or explicitly closed.

These are app-folder tiles. Existing File Explorer folders are not converted
into desktop widgets.

## Collect and restore desktop shortcuts

Drag shortcuts into a group, or choose **Move desktop shortcuts into folder**
from a tile's right-click menu and select them. Only selected `.lnk` and `.url` files
directly on your user Desktop are moved into NestDesktop's shortcut storage.
Adding a personal Desktop shortcut through **Browse** or the app list uses the
same collection and recovery path as dragging it. Subfolders, documents and installed programs are not physically moved. Other
items can be added as references. Shared Public Desktop shortcuts use the
explicit choice described below.

When a selected shortcut is on the shared Public Desktop, a **Shared desktop
shortcuts** window shows each affected app's icon and name. Checked apps move to
your personal Desktop and then into the group; unchecked apps stay on the shared
Desktop and are added as references. The row labels and totals show both results
before you choose **Move selected** or **Add references**. **Cancel** closes this
choice without adding the selection.

Making a shared shortcut personal removes that shortcut from other Windows
accounts' desktops. Windows may ask for administrator approval; you control that
prompt. This is a one-time conversion: later quit/recovery returns the shortcut
to your personal Desktop. It does not restore the all-users shortcut. An app
installer/update can recreate a new shared shortcut later; that new file is not
automatically removed. Re-select it to make another explicit choice.

Conversion records are flushed before Windows moves a shared shortcut into
private staging. An interrupted conversion recovers forward to the personal
Desktop without overwriting other files. The whole data-folder backup includes
`shared-conversions.json` and `shared-staging` as well as collected shortcuts.
Cancelling a Windows file-operation prompt leaves uncompleted moves in place;
any completed conversions remain usable and recoverable.

Collected shortcuts return when NestDesktop quits. A small sleeping recovery
process also returns them after a crash; it waits for the organizer to exit
rather than scanning the desktop. Normal startup collects the same managed
shortcuts again if they are still used by your folders. No running application
can guarantee immediate recovery through a power loss or Windows shutdown;
startup also replays the durable recovery ledger.

Transfers notify Explorer as each file moves, without waiting for its periodic
folder refresh. A multi-selection uses one durable recovery reservation and one
layout commit. Saved Desktop icon coordinates travel with the recovery records,
so normal quit and crash recovery can return icons to their previous slots.
Windows **Auto arrange icons** still controls placement when enabled. Coordinates
cannot recover historical positions for shortcuts collected by older versions;
those are recorded when next collected from a visible desktop icon. If you move
an icon while NestDesktop is closed, its next collection uses its new position.

Dragging an item back to the desktop returns its collected shortcut. For an app
added from elsewhere it creates a desktop shortcut and leaves the source file
alone. Dragging a nested folder out creates a desktop group with its contents.

Choose **Restore collected desktop shortcuts** to return them to their original
locations. Existing names are never overwritten. Conflicting shortcuts remain
in local storage with their original paths recorded in
`%LOCALAPPDATA%\NestDesktop\shortcuts\collection.json`.
This explicit restore also turns off automatic collection for those shortcuts.
Ownership across restarts is stored in `managed-shortcuts.json` beside the layout.

Saved folders and their positions are stored in
`%LOCALAPPDATA%\NestDesktop\layout.json`. Back up the entire NestDesktop data
folder before trying a development build, including any collected shortcuts.
Layout export/import transfers the organization; target applications must be
installed on the destination PC, and unmatched shortcuts may need adding again.
A layout backup does not contain the applications or documents it points to.

## Uninstall

Use **NestDesktop Settings > Uninstall NestDesktop**, Windows' **Installed apps**,
or open the installed `NestDesktop-Setup.exe` with `--uninstall`. The setup window
asks you to select **Uninstall** before closing the organizer and removing it.
Uninstallation removes the application and its
shortcuts, desktop context-menu entry and startup registration. Before removing
the executable, it restores collected desktop shortcuts where names are
available. Saved folders, the collection ledger and any conflicting shortcuts
are retained in local app data. A restore error stops uninstall before program
files are deleted.

`installer/install.ps1` and `installer/uninstall.ps1` are optional command-line
alternatives for users who already allow local PowerShell scripts. They do not
alter PowerShell execution policy. The normal graphical installer does not
depend on PowerShell script execution.

For a read-only installation preview, run `installer/install.ps1 -WhatIf`
from an extracted release. It reports the resolved source and install paths
without changing files, shortcuts, desktop menus or startup registration.

See [Building and packaging](building.md) for development instructions and [Testing and limitations](testing.md) for compatibility notes.
