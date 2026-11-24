//! Compact, event-driven color popover. All rectangles are logical pixels.
use crate::{
    palette::{Color, COLOR_PRESETS, MAX_CUSTOM_COLORS},
    render::{rect, Canvas, BG, MUTED, TEXT},
};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

pub const WIDTH: f32 = 368.;
pub const HEIGHT: f32 = 568.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Preset(usize),
    Custom(usize),
    AddPreset,
    ReplacePreset(usize),
    RemovePreset,
    ResetColor,
    Apply,
    Close,
    SaturationValue,
    Hue,
}
pub struct PaletteHit {
    pub rect: D2D_RECT_F,
    pub control: Control,
}
pub struct ViewState {
    pub selected_hex: String,
    pub saved: Vec<String>,
    pub selected_custom: Option<usize>,
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
    pub error: String,
}

pub fn sv_rect(height: f32) -> D2D_RECT_F {
    rect(24., 294., 320., (height - 456.).clamp(32., 112.))
}
pub fn hue_rect(height: f32) -> D2D_RECT_F {
    rect(24., sv_rect(height).bottom + 14., 320., 14.)
}
pub fn hex_rect(height: f32) -> D2D_RECT_F {
    rect(36., hue_rect(height).bottom + 34., 96., 22.)
}

fn add(hits: &mut Vec<PaletteHit>, rect: D2D_RECT_F, control: Control) -> usize {
    let i = hits.len();
    hits.push(PaletteHit { rect, control });
    i
}

unsafe fn button(
    c: &Canvas,
    hits: &mut Vec<PaletteHit>,
    hover: Option<usize>,
    r: D2D_RECT_F,
    label: &str,
    control: Control,
    primary: bool,
) {
    let i = add(hits, r, control);
    c.fill(
        r,
        if primary {
            0xb8a3ff
        } else if hover == Some(i) {
            0x38445b
        } else {
            0x293348
        },
        9.,
    );
    c.text(
        label,
        r,
        12.,
        if primary { 0x1b2031 } else { TEXT },
        true,
        true,
    );
}

unsafe fn swatch(
    c: &Canvas,
    hits: &mut Vec<PaletteHit>,
    hover: Option<usize>,
    r: D2D_RECT_F,
    color: &str,
    selected: bool,
    control: Control,
) {
    let i = add(hits, r, control);
    draw_swatch(c, r, color, selected, hover == Some(i));
}

unsafe fn draw_swatch(c: &Canvas, r: D2D_RECT_F, color: &str, selected: bool, hovered: bool) {
    let rgb = Color::parse(color).unwrap_or(Color::from_packed(0x9b6dff));
    let side = r.right - r.left;
    if selected || hovered {
        c.outline(
            rect(r.left - 4., r.top - 4., side + 8., side + 8.),
            if selected { TEXT } else { 0x71809c },
            (side + 8.) / 2.,
            if selected { 2. } else { 1. },
        );
    }
    c.fill(r, rgb.packed(), side / 2.);
    if selected {
        let luminance = 0.2126 * rgb.r as f32 + 0.7152 * rgb.g as f32 + 0.0722 * rgb.b as f32;
        c.text(
            "✓",
            r,
            side * 0.53,
            if luminance > 155. { BG } else { 0xffffff },
            true,
            true,
        );
    }
}

fn custom_rect(index: usize) -> D2D_RECT_F {
    rect(
        27. + (index % 8) as f32 * 40.,
        220. + (index / 8) as f32 * 34.,
        26.,
        26.,
    )
}

fn replacement_rect(swatch: D2D_RECT_F) -> D2D_RECT_F {
    rect(swatch.right - 7., swatch.top - 5., 14., 14.)
}

/// Keep replacement regions in the same order while hovering. Their priority
/// over the overlapping swatch corner preserves normal centre-click selection.
fn custom_controls(saved_count: usize) -> Vec<PaletteHit> {
    let mut hits = Vec::new();
    for index in 0..MAX_CUSTOM_COLORS {
        let r = custom_rect(index);
        if index < saved_count {
            if saved_count == MAX_CUSTOM_COLORS {
                add(
                    &mut hits,
                    replacement_rect(r),
                    Control::ReplacePreset(index),
                );
            }
            add(&mut hits, r, Control::Custom(index));
        } else if index == saved_count {
            add(&mut hits, r, Control::AddPreset);
        }
    }
    hits
}

unsafe fn plus(c: &Canvas, r: D2D_RECT_F, arm: f32, thickness: f32, color: u32) {
    let center_x = (r.left + r.right) / 2.;
    let center_y = (r.top + r.bottom) / 2.;
    c.fill(
        rect(
            center_x - arm,
            center_y - thickness / 2.,
            arm * 2.,
            thickness,
        ),
        color,
        thickness / 2.,
    );
    c.fill(
        rect(
            center_x - thickness / 2.,
            center_y - arm,
            thickness,
            arm * 2.,
        ),
        color,
        thickness / 2.,
    );
}

pub unsafe fn draw(
    c: &mut Canvas,
    state: &ViewState,
    hover: Option<usize>,
    height: f32,
) -> Vec<PaletteHit> {
    let mut hits = Vec::new();
    c.fill(rect(0., 0., WIDTH, height), BG, 0.);
    let rgb = Color::parse(&state.selected_hex).unwrap_or(Color::from_packed(0x9b6dff));
    c.fill(rect(24., 20., 24., 24.), rgb.packed(), 12.);
    c.text(
        "Folder color",
        rect(60., 13., 242., 36.),
        20.,
        TEXT,
        true,
        false,
    );
    let close = rect(316., 14., 32., 32.);
    let ci = add(&mut hits, close, Control::Close);
    if hover == Some(ci) {
        c.fill(close, 0x293348, 9.);
    }
    c.text("×", close, 23., MUTED, false, true);
    c.text(
        "Presets",
        rect(24., 63., 220., 20.),
        12.,
        MUTED,
        true,
        false,
    );
    for (i, (_, color)) in COLOR_PRESETS.iter().enumerate() {
        swatch(
            c,
            &mut hits,
            hover,
            rect(
                26. + (i % 6) as f32 * 56.,
                92. + (i / 6) as f32 * 44.,
                34.,
                34.,
            ),
            color,
            state.selected_custom.is_none() && state.selected_hex.eq_ignore_ascii_case(color),
            Control::Preset(i),
        );
    }
    c.text(
        "Your colors",
        rect(24., 184., 170., 24.),
        12.,
        MUTED,
        true,
        false,
    );
    if state.selected_custom.is_some() {
        button(
            c,
            &mut hits,
            hover,
            rect(280., 182., 64., 26.),
            "Remove",
            Control::RemovePreset,
            false,
        );
    }
    hits.extend(custom_controls(state.saved.len()));
    let hovered_control = hover
        .and_then(|index| hits.get(index))
        .map(|hit| hit.control);
    for i in 0..MAX_CUSTOM_COLORS {
        let r = custom_rect(i);
        if let Some(color) = state.saved.get(i) {
            let hovered = matches!(hovered_control, Some(Control::Custom(index) | Control::ReplacePreset(index)) if index == i);
            let selected =
                state.selected_custom == Some(i) && state.selected_hex.eq_ignore_ascii_case(color);
            draw_swatch(c, r, color, selected, hovered);
            if state.saved.len() == MAX_CUSTOM_COLORS && hovered {
                let badge = replacement_rect(r);
                c.fill(badge, BG, 7.);
                c.outline(
                    badge,
                    if hovered_control == Some(Control::ReplacePreset(i)) {
                        TEXT
                    } else {
                        0x71809c
                    },
                    7.,
                    1.,
                );
                plus(c, badge, 3.5, 1.5, TEXT);
            }
        } else {
            c.outline(r, 0x333e53, 13., 1.);
            if i == state.saved.len() {
                if hovered_control == Some(Control::AddPreset) {
                    c.fill(r, 0x293348, 13.);
                }
                plus(c, r, 5., 2., MUTED);
            }
        }
    }
    let sv = sv_rect(height);
    add(&mut hits, sv, Control::SaturationValue);
    c.color_square(sv, state.hue);
    let x = sv.left + state.saturation * (sv.right - sv.left);
    let y = sv.top + (1. - state.value) * (sv.bottom - sv.top);
    c.outline(rect(x - 6., y - 6., 12., 12.), 0x000000, 6., 3.);
    c.outline(rect(x - 6., y - 6., 12., 12.), 0xffffff, 6., 1.5);
    let hue = hue_rect(height);
    add(&mut hits, hue, Control::Hue);
    c.hue_strip(hue);
    let hx = hue.left + state.hue.clamp(0., 1.) * (hue.right - hue.left);
    c.outline(
        rect(hx - 3., hue.top - 3., 6., hue.bottom - hue.top + 6.),
        TEXT,
        3.,
        2.,
    );
    let hex = hex_rect(height);
    c.text(
        "Hex",
        rect(24., hex.top - 25., 126., 18.),
        11.,
        MUTED,
        false,
        false,
    );
    c.fill(rect(24., hex.top - 6., 126., 34.), 0x252d40, 9.);
    c.outline(rect(24., hex.top - 6., 126., 34.), 0x364159, 9., 1.);
    c.fill(rect(166., hex.top - 4., 30., 30.), rgb.packed(), 8.);
    let context = match hovered_control {
        Some(Control::AddPreset) => "Save current color",
        Some(Control::ReplacePreset(_)) => "Replace this preset",
        Some(Control::Custom(_)) if state.saved.len() == MAX_CUSTOM_COLORS => "Use + to replace",
        _ if state
            .selected_custom
            .and_then(|index| state.saved.get(index))
            .is_some_and(|color| color.eq_ignore_ascii_case(&state.selected_hex)) =>
        {
            "Saved color"
        }
        _ => "Current color",
    };
    c.text(
        context,
        rect(207., hex.top - 3., 137., 28.),
        11.,
        MUTED,
        false,
        false,
    );
    c.text(
        &state.error,
        rect(24., hex.top + 33., 320., 18.),
        10.,
        0xffa2a7,
        false,
        false,
    );
    let by = height - 48.;
    button(
        c,
        &mut hits,
        hover,
        rect(24., by, 202., 32.),
        "Reset color",
        Control::ResetColor,
        false,
    );
    button(
        c,
        &mut hits,
        hover,
        rect(238., by, 106., 32.),
        "Apply",
        Control::Apply,
        true,
    );
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hit_at(hits: &[PaletteHit], x: f32, y: f32) -> Option<Control> {
        hits.iter()
            .find(|hit| {
                x >= hit.rect.left
                    && x <= hit.rect.right
                    && y >= hit.rect.top
                    && y <= hit.rect.bottom
            })
            .map(|hit| hit.control)
    }

    #[test]
    fn full_palette_centres_select_and_corner_badges_replace() {
        let hits = custom_controls(MAX_CUSTOM_COLORS);
        assert!(!hits.iter().any(|hit| hit.control == Control::AddPreset));
        for index in 0..MAX_CUSTOM_COLORS {
            let r = custom_rect(index);
            assert_eq!(
                hit_at(&hits, (r.left + r.right) / 2., (r.top + r.bottom) / 2.),
                Some(Control::Custom(index))
            );
            // This point overlaps the saved swatch and replacement badge;
            // replacing must win only in the dedicated corner region.
            assert_eq!(
                hit_at(&hits, r.right - 2., r.top + 2.),
                Some(Control::ReplacePreset(index))
            );
            let badge = replacement_rect(r);
            assert!(badge.right <= WIDTH - 24.);
            assert!(badge.top > 208. && badge.bottom < 294.);
        }
    }

    #[test]
    fn add_moves_to_the_first_empty_slot_until_palette_is_full() {
        for count in [0, 1, 7, 8, MAX_CUSTOM_COLORS - 1] {
            let hits = custom_controls(count);
            let r = custom_rect(count);
            assert_eq!(
                hit_at(&hits, (r.left + r.right) / 2., (r.top + r.bottom) / 2.),
                Some(Control::AddPreset)
            );
            assert!(!hits
                .iter()
                .any(|hit| matches!(hit.control, Control::ReplacePreset(_))));
            assert_eq!(
                hits.iter()
                    .filter(|hit| hit.control == Control::AddPreset)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn gradients_and_edit_fit_compact_and_normal_panels() {
        for height in [488., 520., HEIGHT] {
            let sv = sv_rect(height);
            let hue = hue_rect(height);
            let hex = hex_rect(height);
            assert!(sv.bottom < hue.top && hue.bottom < hex.top - 6.);
            assert!(hex.top + 33. + 18. <= height - 48.);
            assert!(hex.right < WIDTH - 24.);
        }
    }
}
