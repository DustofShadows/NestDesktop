//! Lightweight, paint-on-demand native views. No retained animation loop.
use crate::layout;
use crate::model::Item;
use crate::render::{self, rect, Canvas, BG, LAVENDER, MUTED, PANEL, TEXT};
use crate::{
    Action, App, Hit, PromptKind, COLOR_PANEL, MANAGER, PICKER, POPUP, PROMPT, TILE, TITLE_HOST,
};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Direct2D::Common::D2D_RECT_F,
        Gdi::{BeginPaint, EndPaint, PAINTSTRUCT},
    },
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{GetClientRect, GetParent, IsWindowVisible},
    },
};

const EDGE: u32 = 0x30394d;
const SOFT: u32 = 0x252d40;
const DIM: u32 = 0x78859d;

fn hex(value: &str) -> u32 {
    u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(LAVENDER)
}

fn shortened(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let result: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{}…", result.trim_end())
    } else {
        result
    }
}

fn hit(hits: &mut Vec<Hit>, r: D2D_RECT_F, action: Action) -> usize {
    let index = hits.len();
    hits.push(Hit { r, action });
    index
}

unsafe fn button(
    c: &Canvas,
    hits: &mut Vec<Hit>,
    hover: Option<usize>,
    r: D2D_RECT_F,
    label: &str,
    action: Action,
    primary: bool,
) {
    let index = hit(hits, r, action);
    let over = hover == Some(index);
    c.fill(
        r,
        if primary {
            if over {
                0xdbd2ff
            } else {
                LAVENDER
            }
        } else if over {
            0x343e55
        } else {
            SOFT
        },
        11.,
    );
    if !primary {
        c.outline(r, if over { 0x64718d } else { EDGE }, 11., 1.);
    }
    c.text(
        label,
        r,
        13.,
        if primary { 0x23203b } else { TEXT },
        true,
        true,
    );
}

unsafe fn icon_button(
    c: &Canvas,
    hits: &mut Vec<Hit>,
    hover: Option<usize>,
    r: D2D_RECT_F,
    label: &str,
    action: Action,
) {
    let index = hit(hits, r, action);
    if hover == Some(index) {
        c.fill(r, SOFT, 10.);
    }
    c.text(label, r, 22., MUTED, false, true);
}

unsafe fn folder_icon(c: &Canvas, r: D2D_RECT_F, accent: u32) {
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    c.fill(
        rect(
            r.left + width * 0.08,
            r.top + height * 0.12,
            width * 0.43,
            height * 0.30,
        ),
        accent,
        height * 0.08,
    );
    c.fill(
        rect(
            r.left + width * 0.08,
            r.top + height * 0.27,
            width * 0.84,
            height * 0.61,
        ),
        accent,
        height * 0.10,
    );
    c.fill(
        rect(
            r.left + width * 0.20,
            r.top + height * 0.43,
            width * 0.59,
            height * 0.07,
        ),
        render::tint(accent),
        2.,
    );
}

unsafe fn mosaic(c: &mut Canvas, items: &[Item], r: D2D_RECT_F, accent: u32) {
    mosaic_progress(c, items, r, accent, 0.);
}

unsafe fn mosaic_progress(
    c: &mut Canvas,
    items: &[Item],
    r: D2D_RECT_F,
    accent: u32,
    progress: f32,
) {
    c.fill_alpha(r, accent, 16., 0.72);
    if progress > 0. {
        c.radial_fill(r, 0x4a5871, 16., progress);
    }
    let side = (r.right - r.left) * 0.29;
    let gap = (r.right - r.left) * 0.10;
    let start_x = r.left + ((r.right - r.left) - side * 2. - gap) / 2.;
    let start_y = r.top + ((r.bottom - r.top) - side * 2. - gap) / 2.;
    for index in 0..4 {
        let cell = rect(
            start_x + (index % 2) as f32 * (side + gap),
            start_y + (index / 2) as f32 * (side + gap),
            side,
            side,
        );
        match items.get(index) {
            Some(item) if item.is_folder() => folder_icon(c, cell, accent),
            Some(item) => {
                if !c.icon(&item.target, cell) {
                    c.fill(cell, accent, side * 0.25);
                    c.text(
                        &item.name.chars().next().unwrap_or('A').to_string(),
                        cell,
                        side * 0.65,
                        BG,
                        true,
                        true,
                    );
                }
            }
            None => c.fill(
                cell,
                if items.is_empty() { 0x45516b } else { 0x3b455d },
                side * 0.27,
            ),
        }
    }
}

unsafe fn brand(c: &Canvas, x: f32, y: f32) {
    crate::brand::draw_mark(c, x, y, 34.);
}

unsafe fn sidebar(c: &Canvas, a: &App, hits: &mut Vec<Hit>, hover: Option<usize>, height: f32) {
    c.fill(rect(0., 0., 202., height), 0x1a2030, 0.);
    c.fill(rect(201., 0., 1., height), 0x293146, 0.);
    brand(c, 22., 27.);
    c.text(
        "NestDesktop",
        rect(67., 26., 126., 35.),
        17.,
        TEXT,
        true,
        false,
    );
    for (index, (label, action, selected)) in [
        ("Folders", Action::Home, !a.settings),
        ("Settings", Action::Settings, a.settings),
    ]
    .into_iter()
    .enumerate()
    {
        let r = rect(14., 130. + index as f32 * 51., 174., 42.);
        let hit_index = hit(hits, r, action);
        if selected || hover == Some(hit_index) {
            c.fill(r, if selected { 0x34304e } else { SOFT }, 11.);
        }
        if selected {
            c.fill(rect(14., r.top + 12., 3., 18.), LAVENDER, 1.5);
        }
        c.text(
            if index == 0 { "▦" } else { "⚙" },
            rect(26., r.top, 25., 42.),
            19.,
            if selected { LAVENDER } else { MUTED },
            false,
            true,
        );
        c.text(
            label,
            rect(61., r.top, 120., 42.),
            13.,
            if selected { TEXT } else { MUTED },
            selected,
            false,
        );
    }
    c.text(
        "Version 0.2.11 · Preview",
        rect(23., height - 89., 171., 21.),
        11.,
        MUTED,
        false,
        false,
    );
    let r = rect(14., height - 51., 174., 34.);
    let index = hit(hits, r, Action::Quit);
    if hover == Some(index) {
        c.fill(r, SOFT, 9.);
    }
    c.text(
        "Quit NestDesktop",
        rect(25., r.top, 155., 34.),
        12.,
        MUTED,
        false,
        false,
    );
}

unsafe fn manager(
    c: &mut Canvas,
    a: &App,
    hits: &mut Vec<Hit>,
    hover: Option<usize>,
    width: f32,
    height: f32,
) {
    sidebar(c, a, hits, hover, height);
    let x = 238.;
    c.text(
        if a.settings { "Settings" } else { "Folders" },
        rect(x, 32., width - x - 250., 48.),
        31.,
        TEXT,
        true,
        false,
    );
    c.text(
        if a.settings {
            "Startup and layout files."
        } else {
            "Folders shown on the primary desktop."
        },
        rect(x, 100., width - x - 195., 25.),
        13.,
        MUTED,
        false,
        false,
    );
    button(
        c,
        hits,
        hover,
        rect(width - 228., 36., 196., 40.),
        "Show desktop folders",
        Action::RevealDesktop,
        true,
    );
    if a.settings {
        let cw = width - x - 32.;
        let toggles = [
            (
                "Start with Windows",
                "Keep folders available after sign-in.",
                a.config.settings.start_with_windows,
                Action::Startup,
            ),
            (
                "Show item counts on desktop",
                "Display totals below desktop folder names.",
                a.config.settings.show_item_counts,
                Action::ToggleCounts,
            ),
            (
                "Snap to the desktop grid",
                "Align folder tiles when you move them.",
                a.config.settings.snap_to_grid,
                Action::ToggleSnap,
            ),
            (
                "Open folders on hover",
                "Pause over the colored preview. Pin a folder to keep it open.",
                a.config.settings.hover_open,
                Action::ToggleHover,
            ),
        ];
        c.fill(rect(x, 144., cw, 244.), PANEL, 17.);
        c.outline(rect(x, 144., cw, 244.), EDGE, 17., 1.);
        for (i, (name, detail, enabled, action)) in toggles.into_iter().enumerate() {
            let y = 151. + i as f32 * 59.;
            if i > 0 {
                c.fill(rect(x + 18., y - 4., cw - 36., 1.), EDGE, 0.);
            }
            c.text(
                name,
                rect(x + 18., y + 1., cw - 255., 25.),
                15.,
                TEXT,
                true,
                false,
            );
            c.text(
                detail,
                rect(x + 18., y + 26., cw - 255., 22.),
                11.,
                MUTED,
                false,
                false,
            );
            if i == 3 {
                c.fill(rect(width - 240., y + 9., 104., 32.), SOFT, 9.);
                c.outline(rect(width - 240., y + 9., 104., 32.), EDGE, 9., 1.);
                c.text(
                    "s",
                    rect(width - 171., y + 9., 22., 32.),
                    13.,
                    MUTED,
                    false,
                    true,
                );
            }
            button(
                c,
                hits,
                hover,
                rect(width - 121., y + 9., 66., 32.),
                if enabled { "On" } else { "Off" },
                action,
                enabled,
            );
        }
        c.fill(rect(x, 406., cw, 136.), PANEL, 17.);
        c.outline(rect(x, 406., cw, 136.), EDGE, 17., 1.);
        c.text(
            "Layouts and backups",
            rect(x + 18., 418., cw - 36., 28.),
            17.,
            TEXT,
            true,
            false,
        );
        c.text(
            "Move folder names, colors, positions and shortcuts between PCs.",
            rect(x + 18., 452., cw - 36., 23.),
            11.,
            MUTED,
            false,
            false,
        );
        button(
            c,
            hits,
            hover,
            rect(x + 18., 489., 135., 33.),
            "Export layout",
            Action::Export,
            true,
        );
        button(
            c,
            hits,
            hover,
            rect(x + 164., 489., 135., 33.),
            "Import layout",
            Action::Import,
            false,
        );
        button(
            c,
            hits,
            hover,
            rect(x, 565., 156., 34.),
            "Open layout folder",
            Action::DataFolder,
            false,
        );
        button(
            c,
            hits,
            hover,
            rect(x + 170., 565., 222., 34.),
            "Restore desktop shortcuts",
            Action::RestoreDesktop,
            false,
        );
        let uninstall = rect(width - 210., 565., 178., 34.);
        let uninstall_hit = hit(hits, uninstall, Action::Uninstall);
        c.fill(
            uninstall,
            if hover == Some(uninstall_hit) {
                0x783349
            } else {
                0x5d2838
            },
            10.,
        );
        c.outline(uninstall, 0xb55a6e, 10., 1.);
        c.text(
            "Uninstall NestDesktop…",
            uninstall,
            12.,
            0xffe5ec,
            true,
            true,
        );
    } else {
        button(
            c,
            hits,
            hover,
            rect(width - 174., 95., 142., 36.),
            "+  New folder",
            Action::NewGroup,
            false,
        );
        let overview = crate::manager_scroll::Overview::new(width, height, a.config.groups.len());
        let card_width = overview.card_width;
        c.push_clip(overview.viewport);
        for index in overview.visible(a.manager_scroll, a.config.groups.len()) {
            let group = &a.config.groups[index];
            let r = overview.card(index, a.manager_scroll);
            let Some(card_hit) = crate::manager_scroll::clipped(r, overview.viewport) else {
                continue;
            };
            let gx = r.left;
            let gy = r.top;
            // Only the visible part of a card or its menu accepts input.
            let menu = rect(gx + card_width - 41., gy + 12., 29., 29.);
            let menu_index = crate::manager_scroll::clipped(menu, overview.viewport)
                .map(|r| hit(hits, r, Action::Menu(index)));
            let card_index = hit(hits, card_hit, Action::Open(index));
            c.fill(
                r,
                if hover == Some(card_index) {
                    0x293147
                } else {
                    PANEL
                },
                18.,
            );
            c.outline(
                r,
                if hover == Some(card_index) {
                    0x6b6b97
                } else {
                    EDGE
                },
                18.,
                1.,
            );
            let accent = hex(&group.color);
            mosaic(c, &group.items, rect(gx + 20., gy + 19., 65., 65.), accent);
            if menu_index.is_some() && hover == menu_index {
                c.fill(menu, SOFT, 8.);
            }
            c.text("⋯", menu, 22., MUTED, false, true);
            c.text(
                &shortened(&group.name, 24),
                rect(gx + 20., gy + 101., card_width - 48., 28.),
                18.,
                TEXT,
                true,
                false,
            );
            let folders = group.items.iter().filter(|item| item.is_folder()).count();
            let count = group.items.len();
            let summary = if count == 0 {
                "Empty".to_string()
            } else if folders > 0 {
                format!(
                    "{} apps · {} {}",
                    count - folders,
                    folders,
                    if folders == 1 { "folder" } else { "folders" }
                )
            } else {
                format!("{} {}", count, if count == 1 { "app" } else { "apps" })
            };
            {
                c.text(
                    &summary,
                    rect(gx + 20., gy + 135., card_width - 44., 20.),
                    11.,
                    MUTED,
                    false,
                    false,
                );
            }
            c.text(
                "›",
                rect(gx + card_width - 36., gy + 102., 20., 27.),
                24.,
                accent,
                false,
                true,
            );
        }
        if a.config.groups.is_empty() {
            c.fill(rect(x, 158., width - x - 32., 330.), PANEL, 20.);
            folder_icon(c, rect(x + 58., 207., 67., 67.), LAVENDER);
            c.text(
                "No desktop folders",
                rect(x + 27., 297., width - x - 86., 39.),
                23.,
                TEXT,
                true,
                true,
            );
            c.text(
                "Create a folder, then add or drop shortcuts into it.",
                rect(x + 27., 346., width - x - 86., 28.),
                13.,
                MUTED,
                false,
                true,
            );
            let create = rect(x + 183., 401., 196., 41.);
            if let Some(visible) = crate::manager_scroll::clipped(create, overview.viewport) {
                let before = hits.len();
                button(
                    c,
                    hits,
                    hover,
                    create,
                    "Create desktop folder",
                    Action::NewGroup,
                    true,
                );
                hits[before].r = visible;
            }
        }
        c.pop_clip();
        c.fill(rect(x, height - 107., width - x - 32., 1.), EDGE, 0.);
        c.text(
            "Drag a desktop folder to move it.",
            rect(x, height - 89., width - x - 32., 24.),
            12.,
            MUTED,
            false,
            false,
        );
        c.text(
            "Open a folder to rename it. Right-click for colors and shortcuts.",
            rect(x, height - 61., width - x - 32., 22.),
            11.,
            DIM,
            false,
            false,
        );
    }
    if !a.status.is_empty() {
        c.text(
            &a.status,
            rect(x, height - 32., width - x - 24., 22.),
            11.,
            LAVENDER,
            false,
            false,
        );
    }
}

unsafe fn popup(
    c: &mut Canvas,
    a: &App,
    hits: &mut Vec<Hit>,
    hover: Option<usize>,
    width: f32,
    height: f32,
) {
    let items = a.visible_items();
    let l = a.v2_popup_layout();
    if a.open_animation_progress < 1. {
        // Expand only the shell. The icons appear at their final size and
        // position once the window settles, without a changing preview mosaic.
        c.fill_alpha(
            rect(0., 0., width, height),
            BG,
            0.,
            crate::surface::POPUP_TINT_OPACITY,
        );
        return;
    }
    let accent = a
        .config
        .groups
        .get(a.group)
        .map(|g| hex(&g.color))
        .unwrap_or(LAVENDER);
    // DWM supplies the native outer curve. An additional larger header curve
    // would expose dark BG sectors between the two radii on an opaque HWND.
    c.fill_alpha(
        rect(0., 0., width, height),
        BG,
        0.,
        crate::surface::POPUP_TINT_OPACITY,
    );
    c.fill_alpha(
        rect(0., 0., width, 52.),
        0x1b2232,
        0.,
        crate::surface::POPUP_HEADER_OPACITY,
    );
    if a.trail.is_empty() {
        let color_rect = layout::popup_folder_button_rect();
        let color_hit = hit(hits, color_rect, Action::ChooseColor);
        if hover == Some(color_hit) {
            c.fill(color_rect, render::tint(accent), 9.);
        }
        folder_icon(c, layout::popup_folder_icon_rect(), accent);
    } else {
        icon_button(
            c,
            hits,
            hover,
            layout::popup_folder_button_rect(),
            "‹",
            Action::Back,
        );
    }
    let title_index = hit(hits, l.header_title, Action::EditTitle);
    let title_editing = IsWindowVisible(a.title_edit).as_bool()
        && GetParent(a.title_edit).unwrap_or_default() == a.popup;
    if hover == Some(title_index) || title_editing {
        c.fill(l.header_title, 0x2c354a, 9.);
    }
    if !title_editing {
        c.text(
            &shortened(&a.current_title(), 25),
            rect(
                l.header_title.left + 7.,
                l.header_title.top,
                l.header_title.right - l.header_title.left - 14.,
                38.,
            ),
            22.,
            TEXT,
            true,
            false,
        );
    }
    icon_button(
        c,
        hits,
        hover,
        rect(width - 119., 10., 32., 34.),
        "⋯",
        Action::Menu(a.group),
    );
    let pin_rect = rect(width - 83., 10., 32., 34.);
    let pin_index = hit(hits, pin_rect, Action::TogglePin);
    if a.popup_pinned {
        c.fill(pin_rect, render::tint(accent), 9.);
    } else if hover == Some(pin_index) {
        c.fill(pin_rect, SOFT, 9.);
    }
    c.text(
        "📌",
        pin_rect,
        17.,
        if a.popup_pinned { accent } else { MUTED },
        false,
        true,
    );
    icon_button(
        c,
        hits,
        hover,
        rect(width - 47., 10., 32., 34.),
        "×",
        Action::Close,
    );
    let search = layout::search_container(&l);
    c.fill(search, SOFT, 12.);
    c.outline(search, EDGE, 12., 1.);
    c.text("⌕", rect(27., 63., 23., 27.), 23., MUTED, false, true);
    let page_count = items.len().div_ceil(layout::PAGE_ITEMS).max(1);
    let page = a.page.min(page_count - 1);
    let visible: Vec<_> = items
        .iter()
        .skip(page * layout::PAGE_ITEMS)
        .take(layout::PAGE_ITEMS)
        .collect();
    for (offset, item) in visible.iter().enumerate() {
        let r = layout::item_rect(&l, offset);
        let action_index = hit(hits, r, Action::Launch(item.id.clone()));
        if hover == Some(action_index) {
            c.fill(r, SOFT, 12.);
        }
        if a.focused == offset {
            c.outline(r, 0x5c6184, 12., 1.);
        }
        let icon = rect(r.left + (l.cell_w - 38.) / 2. - 1.5, r.top + 5., 38., 38.);
        if item.is_folder() {
            mosaic(c, &item.children, icon, accent);
        } else if !c.icon(&item.target, icon) {
            c.fill(icon, render::tint(accent), 11.);
            c.text(
                &item.name.chars().next().unwrap_or('A').to_string(),
                icon,
                23.,
                accent,
                true,
                true,
            );
        }
        let item_editing = matches!(&a.title_target, Some(crate::runtime_v2::TitleTarget::Item(_, id)) if id == &item.id)
            && IsWindowVisible(a.title_host).as_bool();
        if !item_editing {
            c.text(
                &shortened(&item.name, 16),
                rect(r.left + 3., r.top + 46., l.cell_w - 9., 20.),
                11.5,
                TEXT,
                false,
                true,
            );
        }
        if a.reorder_before.as_deref() == Some(item.id.as_str()) {
            c.fill(layout::insertion_rect(&l, offset, false), accent, 1.5);
        }
    }
    if a.reorder_at_end && !visible.is_empty() {
        c.fill(
            layout::insertion_rect(&l, visible.len() - 1, true),
            accent,
            1.5,
        );
    }
    if items.is_empty() {
        if a.query.trim().is_empty() {
            folder_icon(c, rect(width / 2. - 20., 131., 40., 40.), accent);
            c.text(
                "Drop shortcuts here",
                rect(30., 178., width - 60., 29.),
                18.,
                TEXT,
                true,
                true,
            );
            c.text(
                "Add applications below, or drag shortcuts into this folder.",
                rect(25., 214., width - 50., 26.),
                11.,
                MUTED,
                false,
                true,
            );
        } else {
            c.text(
                "No matching apps",
                rect(30., l.grid_y + 10., width - 60., 29.),
                18.,
                TEXT,
                true,
                true,
            );
            c.text(
                "Try another name, or clear your search.",
                rect(25., l.grid_y + 42., width - 50., 23.),
                11.,
                MUTED,
                false,
                true,
            );
        }
    }
    c.fill(rect(16., height - 54., width - 32., 1.), EDGE, 0.);
    button(
        c,
        hits,
        hover,
        rect(16., height - 43., 111., 32.),
        "+  Add apps",
        Action::AddApps,
        true,
    );
    button(
        c,
        hits,
        hover,
        rect(137., height - 43., 108., 32.),
        "New folder",
        Action::NewSubfolder,
        false,
    );
    if page_count > 1 {
        c.text(
            &format!("{} items", items.len()),
            rect(width - 230., height - 43., 82., 32.),
            11.,
            MUTED,
            false,
            true,
        );
        if page > 0 {
            icon_button(
                c,
                hits,
                hover,
                rect(width - 126., height - 44., 30., 34.),
                "‹",
                Action::PageBack,
            );
        }
        c.text(
            &format!("{} / {}", page + 1, page_count),
            rect(width - 93., height - 43., 43., 32.),
            11.,
            MUTED,
            false,
            true,
        );
        if page + 1 < page_count {
            icon_button(
                c,
                hits,
                hover,
                rect(width - 47., height - 44., 30., 34.),
                "›",
                Action::PageNext,
            );
        }
    } else {
        c.text(
            &format!(
                "{} {}",
                items.len(),
                if items.len() == 1 { "item" } else { "items" }
            ),
            rect(width - 107., height - 43., 83., 32.),
            11.,
            MUTED,
            false,
            true,
        );
    }
}

unsafe fn picker(
    c: &Canvas,
    a: &App,
    hits: &mut Vec<Hit>,
    hover: Option<usize>,
    width: f32,
    height: f32,
) {
    c.text(
        "Add apps",
        rect(32., 22., width - 100., 38.),
        27.,
        TEXT,
        true,
        false,
    );
    c.text(
        &format!("Choose apps for {}", shortened(&a.current_title(), 42)),
        rect(33., 64., width - 66., 23.),
        13.,
        MUTED,
        false,
        false,
    );
    icon_button(
        c,
        hits,
        hover,
        rect(width - 49., 23., 30., 33.),
        "×",
        Action::ClosePicker,
    );
    c.fill(rect(32., 92., width - 64., 48.), SOFT, 12.);
    c.outline(rect(32., 92., width - 64., 48.), EDGE, 12., 1.);
    c.fill(rect(31., 152., width - 62., 332.), PANEL, 13.);
    c.outline(rect(31., 152., width - 62., 332.), EDGE, 13., 1.);
    if !a.catalogue_ready {
        c.text(
            "Finding your installed apps…",
            rect(45., 259., width - 90., 30.),
            15.,
            MUTED,
            false,
            true,
        );
    } else if a.picker_map.is_empty() {
        c.text(
            "No matching apps",
            rect(45., 259., width - 90., 30.),
            16.,
            TEXT,
            true,
            true,
        );
        c.text(
            "Try another search, or browse for a shortcut.",
            rect(45., 297., width - 90., 24.),
            12.,
            MUTED,
            false,
            true,
        );
    }
    c.text(
        "Select one or more apps. Use Ctrl or Shift to select several.",
        rect(33., 492., width - 66., 25.),
        11.,
        MUTED,
        false,
        false,
    );
    button(
        c,
        hits,
        hover,
        rect(32., height - 59., 145., 38.),
        "Browse for a file…",
        Action::Browse,
        false,
    );
    button(
        c,
        hits,
        hover,
        rect(width - 202., height - 59., 170., 38.),
        "Add selected apps",
        Action::PickAdd,
        true,
    );
}

unsafe fn prompt(
    c: &Canvas,
    a: &App,
    hits: &mut Vec<Hit>,
    hover: Option<usize>,
    width: f32,
    height: f32,
) {
    let rename = matches!(
        a.prompt_state.as_ref().map(|p| &p.kind),
        Some(PromptKind::RenameGroup(_) | PromptKind::RenameItem(_))
    );
    let title = match a.prompt_state.as_ref().map(|p| &p.kind) {
        Some(PromptKind::NewGroup) => "New desktop folder",
        Some(PromptKind::RenameGroup(_) | PromptKind::RenameItem(_)) => "Rename folder",
        _ => "New folder inside",
    };
    c.text(
        title,
        rect(30., 23., width - 60., 36.),
        23.,
        TEXT,
        true,
        false,
    );
    c.text(
        "Enter a folder name.",
        rect(31., 61., width - 62., 24.),
        12.,
        MUTED,
        false,
        false,
    );
    c.fill(rect(30., 86., width - 60., 46.), SOFT, 12.);
    c.outline(rect(30., 86., width - 60., 46.), 0x6f6c9c, 12., 1.);
    button(
        c,
        hits,
        hover,
        rect(width - 244., height - 52., 95., 36.),
        "Cancel",
        Action::PromptCancel,
        false,
    );
    button(
        c,
        hits,
        hover,
        rect(width - 137., height - 52., 107., 36.),
        if rename { "Save name" } else { "Create folder" },
        Action::PromptOk,
        true,
    );
}

unsafe fn tile(
    c: &mut Canvas,
    a: &App,
    hits: &mut Vec<Hit>,
    _hover: Option<usize>,
    width: f32,
    height: f32,
    group_index: usize,
) {
    let Some(group) = a.config.groups.get(group_index) else {
        return;
    };
    let name_rect = layout::tile_title_rect(a.config.settings.show_item_counts);
    hit(hits, rect(0., 0., width, height), Action::Open(group_index));
    let accent = hex(&group.color);
    c.fill(rect(1., 1., width - 2., height - 2.), 0x1c2333, 21.);
    let progress = if a.hovered_group == Some(group_index) {
        a.hover_progress.clamp(0., 1.)
    } else {
        0.
    };
    c.outline(rect(1.5, 1.5, width - 3., height - 3.), 0x394259, 20., 1.);
    mosaic_progress(
        c,
        &group.items,
        layout::tile_preview_rect(),
        accent,
        progress,
    );
    c.text(
        &shortened(&group.name, 15),
        name_rect,
        13.,
        TEXT,
        true,
        true,
    );
    if a.config.settings.show_item_counts {
        c.text(
            &format!(
                "{} {}",
                group.items.len(),
                if group.items.len() == 1 {
                    "item"
                } else {
                    "items"
                }
            ),
            rect(5., 111., width - 10., 16.),
            9.,
            MUTED,
            false,
            true,
        );
    }
}

pub unsafe fn paint(a: &mut App, h: HWND, kind: isize) {
    let mut ps = PAINTSTRUCT::default();
    let _ = BeginPaint(h, &mut ps);
    let healthy = render(a, h, kind);
    let _ = EndPaint(h, &ps);
    if kind >= TILE {
        if healthy {
            a.tile_paint_completed(h);
        } else {
            a.queue_tile_paint(h);
        }
    }
}

/// Render/present a complete surface independently of native WM_PAINT timing.
/// Startup uses this while the tile is still hidden, after its final sizing.
pub unsafe fn render(a: &mut App, h: HWND, kind: isize) -> bool {
    let _painting = crate::paint_retry::Painting::begin(h.0 as isize);
    if kind == MANAGER {
        a.sync_manager_scrollbar();
    }
    if kind >= TILE && a.fail_first_tile_paint {
        a.fail_first_tile_paint = false;
        a.injected_paint_failures += 1;
        return false;
    }
    let mut client = RECT::default();
    if GetClientRect(h, &mut client).is_err() {
        return false;
    }
    let width = (client.right - client.left).max(1) as u32;
    let height = (client.bottom - client.top).max(1) as u32;
    let scale = (GetDpiForWindow(h).max(96) as f32) / 96.;
    let logical_width = width as f32 / scale;
    let logical_height = height as f32 / scale;
    let key = h.0 as isize;
    let canvas = a
        .canvases
        .remove(&key)
        .or_else(|| Canvas::new(h, width, height).ok());
    if let Some(mut c) = canvas {
        c.resize(width, height);
        c.begin(BG);
        let mut hits = Vec::new();
        let hover = a.hover.get(&key).copied();
        if kind >= TILE {
            tile(
                &mut c,
                a,
                &mut hits,
                hover,
                logical_width,
                logical_height,
                (kind - TILE) as usize,
            );
        } else {
            if matches!(kind, PICKER | PROMPT) {
                c.fill(rect(0., 0., logical_width, logical_height), BG, 0.);
            }
            match kind {
                MANAGER => manager(&mut c, a, &mut hits, hover, logical_width, logical_height),
                POPUP => popup(&mut c, a, &mut hits, hover, logical_width, logical_height),
                PICKER => picker(&c, a, &mut hits, hover, logical_width, logical_height),
                PROMPT => prompt(&c, a, &mut hits, hover, logical_width, logical_height),
                COLOR_PANEL => {
                    if let Some(session) = a.color_session.as_ref() {
                        for entry in
                            crate::palette_view::draw(&mut c, &session.view, hover, logical_height)
                        {
                            hits.push(Hit {
                                r: entry.rect,
                                action: Action::Palette(entry.control),
                            });
                        }
                    }
                }
                TITLE_HOST => c.fill(rect(0., 0., logical_width, logical_height), 0x374257, 0.),
                _ => {}
            }
        }
        let healthy = c.end().is_ok();
        a.hits.insert(key, hits);
        if healthy {
            a.canvases.insert(key, c);
            if kind >= TILE {
                let count = a.presentations.entry(key).or_default();
                *count = count.saturating_add(1);
            }
        }
        return healthy;
    }
    false
}
