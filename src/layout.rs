//! Shared logical geometry for drawing, hit testing and native edit controls.
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

pub const TILE_WIDTH: f32 = 104.;
pub const PAGE_ITEMS: usize = 12;
pub const GRID_X: f32 = 16.;
pub const TILE_PREVIEW_RADIUS: f32 = 16.;

pub fn popup_folder_button_rect() -> D2D_RECT_F {
    bounds(16., 9., 36., 36.)
}
pub fn popup_folder_icon_rect() -> D2D_RECT_F {
    bounds(21., 14., 26., 26.)
}

/// The colored miniature grid is the only hover-open target on a closed tile.
pub fn tile_preview_rect() -> D2D_RECT_F {
    bounds(14., 9., TILE_WIDTH - 28., TILE_WIDTH - 28.)
}

pub fn tile_preview_contains(x: f32, y: f32) -> bool {
    let r = tile_preview_rect();
    if x < r.left || x >= r.right || y < r.top || y >= r.bottom {
        return false;
    }
    let nearest_x = x.clamp(r.left + TILE_PREVIEW_RADIUS, r.right - TILE_PREVIEW_RADIUS);
    let nearest_y = y.clamp(r.top + TILE_PREVIEW_RADIUS, r.bottom - TILE_PREVIEW_RADIUS);
    (x - nearest_x).powi(2) + (y - nearest_y).powi(2) <= TILE_PREVIEW_RADIUS.powi(2)
}

pub fn tile_height(show_counts: bool) -> f32 {
    if show_counts {
        134.
    } else {
        114.
    }
}

pub fn tile_title_rect(_show_counts: bool) -> D2D_RECT_F {
    bounds(5., 86., TILE_WIDTH - 10., 22.)
}

#[derive(Clone, Copy, Debug)]
pub struct PopupLayout {
    pub width: f32,
    pub height: f32,
    /// The native EDIT's inner rectangle, padded inside search_container().
    pub search_x: f32,
    pub search_y: f32,
    pub search_w: f32,
    pub search_h: f32,
    pub grid_y: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub cols: usize,
    pub rows: usize,
    pub header_title: D2D_RECT_F,
}

pub fn popup_layout(item_count: usize) -> PopupLayout {
    let rows = if item_count == 0 {
        2
    } else {
        item_count.min(PAGE_ITEMS).div_ceil(4).clamp(1, 3)
    };
    PopupLayout {
        width: 488.,
        height: 110. + rows as f32 * 76. + 54.,
        search_x: 62.,
        search_y: 66.,
        search_w: 402.,
        search_h: 22.,
        grid_y: 110.,
        cell_w: 114.,
        cell_h: 76.,
        cols: 4,
        rows,
        header_title: bounds(56., 8., 300., 38.),
    }
}

pub fn search_container(layout: &PopupLayout) -> D2D_RECT_F {
    bounds(16., 58., layout.width - 32., 38.)
}

pub fn item_rect(layout: &PopupLayout, offset: usize) -> D2D_RECT_F {
    bounds(
        GRID_X + (offset % layout.cols) as f32 * layout.cell_w,
        layout.grid_y + (offset / layout.cols) as f32 * layout.cell_h,
        layout.cell_w - 3.,
        layout.cell_h - 5.,
    )
}

pub fn insertion_rect(layout: &PopupLayout, offset: usize, after: bool) -> D2D_RECT_F {
    let r = item_rect(layout, offset.min(PAGE_ITEMS - 1));
    bounds(
        if after { r.right + 1. } else { r.left - 3. },
        r.top + 4.,
        3.,
        60.,
    )
}

pub fn bounds(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_color_button_and_glyph_share_their_center() {
        let b = popup_folder_button_rect();
        let i = popup_folder_icon_rect();
        assert_eq!(
            (b.left + b.right, b.top + b.bottom),
            (i.left + i.right, i.top + i.bottom)
        );
        assert!(i.left > b.left && i.right < b.right && i.top > b.top && i.bottom < b.bottom);
    }

    #[test]
    fn hover_target_excludes_tile_margins_title_and_uncolored_rounded_corners() {
        assert!(tile_preview_contains(52., 47.));
        assert!(tile_preview_contains(14., 47.));
        assert!(tile_preview_contains(30., 9.));
        assert!(!tile_preview_contains(13., 47.));
        assert!(!tile_preview_contains(90., 47.));
        assert!(!tile_preview_contains(52., 90.));
        assert!(!tile_preview_contains(14., 9.));
        assert!(!tile_preview_contains(15., 10.));
        assert!(!tile_preview_contains(89., 84.));
    }

    #[test]
    fn count_toggle_reclaims_height_without_moving_the_editable_name() {
        assert!(tile_height(false) < tile_height(true));
        let r = tile_title_rect(false);
        assert!(r.bottom < tile_height(false));
        assert_eq!(r.top, tile_title_rect(true).top);
    }

    #[test]
    fn grid_and_search_fit_compact_and_full_popups_without_overlapping_footer() {
        for count in [0, 1, 4, 5, 8, 9, 12, 60] {
            let l = popup_layout(count);
            let search = search_container(&l);
            assert!(
                l.search_x > 54.,
                "leave room between the magnifier and typing"
            );
            assert!(l.search_x + l.search_w < search.right);
            assert!(l.search_y >= search.top && l.search_y + l.search_h <= search.bottom);
            let last = item_rect(&l, l.rows * l.cols - 1);
            assert!(last.right < l.width);
            assert!(last.bottom <= l.height - 54.);
            assert!(search.bottom < l.grid_y);
        }
        assert!(popup_layout(2).height < popup_layout(10).height);
        assert_eq!(popup_layout(13).height, popup_layout(12).height);
    }
}
