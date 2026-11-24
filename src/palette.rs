//! Color conversions and bounded custom presets for the native color panel.

pub const MAX_CUSTOM_COLORS: usize = 16;

pub const COLOR_PRESETS: [(&str, &str); 12] = [
    ("Violet", "#9B6DFF"),
    ("Indigo", "#667DFF"),
    ("Blue", "#3EADFF"),
    ("Cyan", "#22C9DD"),
    ("Teal", "#21C9A5"),
    ("Green", "#63C86A"),
    ("Lime", "#B9D34B"),
    ("Amber", "#F3B844"),
    ("Orange", "#FF9554"),
    ("Coral", "#F56C79"),
    ("Pink", "#E86DB0"),
    ("Slate", "#8DA9C4"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    /// Hue in degrees, wrapped into [0, 360).
    pub h: f32,
    /// Saturation and value in [0, 1].
    pub s: f32,
    pub v: f32,
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0., 1.)
    } else {
        0.
    }
}

fn fraction(position: f32, extent: f32) -> f32 {
    if !position.is_finite() || !extent.is_finite() || extent <= 0. {
        0.
    } else {
        (position / extent).clamp(0., 1.)
    }
}

impl Color {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        let digits = value.strip_prefix('#').unwrap_or(value);
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Enter a six-digit color such as #9B6DFF.".into());
        }
        let packed = u32::from_str_radix(digits, 16)
            .map_err(|_| "Enter a six-digit color such as #9B6DFF.".to_string())?;
        Ok(Self::from_packed(packed))
    }

    pub const fn from_packed(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 255) as u8,
            g: ((rgb >> 8) & 255) as u8,
            b: (rgb & 255) as u8,
        }
    }

    pub const fn packed(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    pub fn hsv(self) -> Hsv {
        let r = self.r as f32 / 255.;
        let g = self.g as f32 / 255.;
        let b = self.b as f32 / 255.;
        let high = r.max(g).max(b);
        let low = r.min(g).min(b);
        let difference = high - low;
        let hue = if difference <= f32::EPSILON {
            0.
        } else if high == r {
            60. * ((g - b) / difference).rem_euclid(6.)
        } else if high == g {
            60. * ((b - r) / difference + 2.)
        } else {
            60. * ((r - g) / difference + 4.)
        };
        Hsv::new(hue, if high == 0. { 0. } else { difference / high }, high)
    }

    pub fn from_hsv(hsv: Hsv) -> Self {
        hsv.color()
    }
}

impl Hsv {
    pub fn new(h: f32, s: f32, v: f32) -> Self {
        Self {
            h: if h.is_finite() {
                h.rem_euclid(360.)
            } else {
                0.
            },
            s: unit(s),
            v: unit(v),
        }
    }

    pub fn color(self) -> Color {
        let Self { h, s, v } = Self::new(self.h, self.s, self.v);
        let chroma = v * s;
        let secondary = chroma * (1. - ((h / 60.).rem_euclid(2.) - 1.).abs());
        let (r, g, b) = match (h / 60.) as u32 {
            0 => (chroma, secondary, 0.),
            1 => (secondary, chroma, 0.),
            2 => (0., chroma, secondary),
            3 => (0., secondary, chroma),
            4 => (secondary, 0., chroma),
            _ => (chroma, 0., secondary),
        };
        let offset = v - chroma;
        let channel = |value: f32| ((value + offset) * 255.).round().clamp(0., 255.) as u8;
        Color {
            r: channel(r),
            g: channel(g),
            b: channel(b),
        }
    }

    pub fn with_hue(self, hue: f32) -> Self {
        Self::new(hue, self.s, self.v)
    }

    pub fn with_sv(self, saturation: f32, value: f32) -> Self {
        Self::new(self.h, saturation, value)
    }

    /// Pointer coordinates relative to the saturation/value square.
    /// Left/right select saturation; top/bottom select bright/dark.
    pub fn from_sv_point(self, x: f32, y: f32, width: f32, height: f32) -> Self {
        self.with_sv(fraction(x, width), 1. - fraction(y, height))
    }
}

/// Pointer x relative to a horizontal hue strip. Keep the rightmost marker at
/// the right rather than wrapping it to the left at an exact 360 degrees.
pub fn hue_from_point(x: f32, width: f32) -> f32 {
    (fraction(x, width) * 360.).min(359.99997)
}

/// Normalize and deduplicate the saved row while retaining one genuine white
/// preset from the old native picker's initially repeated white swatches.
/// This is read-only; callers choose when to persist the canonical row.
pub fn canonical_custom_colors(colors: &[String]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for value in colors {
        let value = Color::parse(value)?.hex();
        if !result.contains(&value) {
            result.push(value);
        }
    }
    if result.len() > MAX_CUSTOM_COLORS {
        return Err("You can save up to 16 custom colors.".into());
    }
    Ok(result)
}

/// On success, the row is canonical and the returned index selects the color.
/// An existing color is selected rather than stored twice.
pub fn add_custom(colors: &mut Vec<String>, value: &str) -> Result<usize, String> {
    save_custom(colors, value, None)
}

/// Save the current color immediately, optionally replacing one visible slot.
/// Existing colors are selected without replacing or moving any saved color.
/// Explicit replacement indices are validated even when the color is a duplicate.
/// Errors leave the caller's row untouched.
pub fn save_custom(
    colors: &mut Vec<String>,
    value: &str,
    replace: Option<usize>,
) -> Result<usize, String> {
    let value = Color::parse(value)?.hex();
    let mut next = canonical_custom_colors(colors)?;
    if replace.is_some_and(|index| index >= next.len()) {
        return Err("This custom color is no longer available.".into());
    }
    let index = if let Some(index) = next.iter().position(|c| c == &value) {
        index
    } else if let Some(index) = replace {
        next[index] = value;
        index
    } else {
        if next.len() == MAX_CUSTOM_COLORS {
            return Err("You can save up to 16 custom colors.".into());
        }
        let index = next.len();
        next.push(value);
        index
    };
    *colors = next;
    Ok(index)
}

/// The index refers to the canonical visible row, not repeated legacy slots.
/// Invalid, missing or duplicate replacements leave the original row untouched.
pub fn update_custom(colors: &mut Vec<String>, index: usize, value: &str) -> Result<usize, String> {
    let value = Color::parse(value)?.hex();
    let mut next = canonical_custom_colors(colors)?;
    if index >= next.len() {
        return Err("This custom color is no longer available.".into());
    }
    if next
        .iter()
        .enumerate()
        .any(|(i, c)| i != index && c == &value)
    {
        return Err("This color is already in your custom presets.".into());
    }
    next[index] = value;
    *colors = next;
    Ok(index)
}

/// Remove an explicitly selected canonical-row preset.
pub fn remove_custom(colors: &mut Vec<String>, index: usize) -> Result<(), String> {
    let mut next = canonical_custom_colors(colors)?;
    if index >= next.len() {
        return Err("This custom color is no longer available.".into());
    }
    next.remove(index);
    *colors = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_normalization_rejects_partial_alpha_and_unicode_values() {
        assert_eq!(Color::parse(" 9b6dff ").unwrap().hex(), "#9B6DFF");
        assert_eq!(Color::parse("#12ABef").unwrap().packed(), 0x12abef);
        for bad in [
            "",
            "#FFF",
            "#12345678",
            "#GG0000",
            "red",
            "#ééé",
            "##123456",
        ] {
            assert!(Color::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hsv_roundtrips_rgb_samples_presets_and_grayscale() {
        for r in [0, 1, 64, 128, 192, 255] {
            for g in [0, 1, 64, 128, 192, 255] {
                for b in [0, 1, 64, 128, 192, 255] {
                    let color = Color { r, g, b };
                    assert_eq!(color.hsv().color(), color, "{color:?}");
                }
            }
        }
        for (_, hex) in COLOR_PRESETS {
            let color = Color::parse(hex).unwrap();
            assert_eq!(Color::from_hsv(color.hsv()), color);
        }
        assert_eq!(Color::from_packed(0x808080).hsv().s, 0.);
        assert_eq!(Color::from_packed(0).hsv(), Hsv::new(0., 0., 0.));
    }

    #[test]
    fn hue_boundaries_and_pointer_coordinates_are_clamped() {
        for (h, rgb) in [
            (0., 0xff0000),
            (60., 0xffff00),
            (120., 0x00ff00),
            (180., 0x00ffff),
            (240., 0x0000ff),
            (300., 0xff00ff),
            (360., 0xff0000),
            (-60., 0xff00ff),
        ] {
            assert_eq!(Hsv::new(h, 1., 1.).color().packed(), rgb);
        }
        let hue = Hsv::new(210., 0.5, 0.5);
        assert_eq!(
            hue.from_sv_point(-30., -10., 100., 80.),
            Hsv::new(210., 0., 1.)
        );
        assert_eq!(
            hue.from_sv_point(130., 100., 100., 80.),
            Hsv::new(210., 1., 0.)
        );
        assert_eq!(
            hue.from_sv_point(25., 20., 100., 80.),
            Hsv::new(210., 0.25, 0.75)
        );
        assert_eq!(hue_from_point(-1., 100.), 0.);
        assert!(hue_from_point(1000., 100.) < 360.);
        assert_eq!(hue_from_point(20., 0.), 0.);
        assert_eq!(
            Hsv::new(f32::NAN, f32::INFINITY, f32::NAN).color().packed(),
            0
        );
    }

    #[test]
    fn legacy_custom_row_keeps_one_white_and_operations_use_visible_indices() {
        let mut colors = vec![
            "#ffffff".into(),
            "#FFFFFF".into(),
            "123456".into(),
            "#FFFFFF".into(),
        ];
        assert_eq!(
            canonical_custom_colors(&colors).unwrap(),
            vec!["#FFFFFF", "#123456"]
        );
        assert_eq!(add_custom(&mut colors, "#123456").unwrap(), 1);
        assert_eq!(colors, vec!["#FFFFFF", "#123456"]);
        assert_eq!(update_custom(&mut colors, 1, "aabbcc").unwrap(), 1);
        assert_eq!(colors, vec!["#FFFFFF", "#AABBCC"]);
        remove_custom(&mut colors, 0).unwrap();
        assert_eq!(colors, vec!["#AABBCC"]);
    }

    #[test]
    fn invalid_duplicate_or_missing_custom_changes_are_transactional() {
        let mut colors = vec!["#aa0000".into(), "#00aa00".into()];
        let original = colors.clone();
        assert!(update_custom(&mut colors, 0, "#00AA00").is_err());
        assert_eq!(colors, original);
        assert!(update_custom(&mut colors, 5, "#0000AA").is_err());
        assert_eq!(colors, original);
        assert!(add_custom(&mut colors, "invalid").is_err());
        assert_eq!(colors, original);
        assert!(remove_custom(&mut colors, 5).is_err());
        assert_eq!(colors, original);
        let mut invalid = vec!["#AA0000".into(), "bad".into()];
        let before = invalid.clone();
        assert!(add_custom(&mut invalid, "#BB0000").is_err());
        assert_eq!(invalid, before);
    }

    #[test]
    fn full_custom_row_can_select_or_update_without_exceeding_sixteen() {
        let mut colors: Vec<_> = (0..16).map(|i| format!("#{i:06X}")).collect();
        let original = colors.clone();
        assert!(add_custom(&mut colors, "#FFFFFF").is_err());
        assert_eq!(colors, original);
        assert_eq!(add_custom(&mut colors, "00000a").unwrap(), 10);
        assert_eq!(colors.len(), 16);
        update_custom(&mut colors, 10, "#FFFFFF").unwrap();
        assert_eq!(colors.len(), 16);
        remove_custom(&mut colors, 10).unwrap();
        assert_eq!(add_custom(&mut colors, "#ABCDEF").unwrap(), 15);
        assert_eq!(colors.len(), 16);
    }

    #[test]
    fn instant_save_appends_a_new_color_and_selects_existing_without_duplicates() {
        let mut colors = vec!["#ff0000".into(), "#00FF00".into()];
        assert_eq!(save_custom(&mut colors, "0000ff", None).unwrap(), 2);
        assert_eq!(colors, vec!["#FF0000", "#00FF00", "#0000FF"]);
        let before = colors.clone();
        assert_eq!(save_custom(&mut colors, "#00ff00", None).unwrap(), 1);
        assert_eq!(colors, before);
    }

    #[test]
    fn full_palette_replacement_preserves_other_slots_and_selects_duplicate_colors() {
        let mut colors: Vec<_> = (0..MAX_CUSTOM_COLORS)
            .map(|index| format!("#{index:06X}"))
            .collect();
        let original = colors.clone();
        assert_eq!(save_custom(&mut colors, "#00000A", Some(5)).unwrap(), 10);
        assert_eq!(colors, original);
        assert_eq!(save_custom(&mut colors, "000005", Some(5)).unwrap(), 5);
        assert_eq!(colors, original);
        assert_eq!(save_custom(&mut colors, "ff00ff", Some(5)).unwrap(), 5);
        assert_eq!(colors.len(), MAX_CUSTOM_COLORS);
        assert_eq!(colors[5], "#FF00FF");
        for (index, value) in original.iter().enumerate() {
            if index != 5 {
                assert_eq!(&colors[index], value);
            }
        }
        let replaced = colors.clone();
        assert!(save_custom(&mut colors, "#ABCDEF", None).is_err());
        assert_eq!(colors, replaced);
        assert_eq!(save_custom(&mut colors, "ff00ff", None).unwrap(), 5);
        assert_eq!(colors, replaced);
    }

    #[test]
    fn instant_save_rejects_invalid_targets_and_rows_without_partial_changes() {
        let mut colors = vec!["#ffffff".into(), "#FFFFFF".into(), "123456".into()];
        let original = colors.clone();
        assert!(save_custom(&mut colors, "#FFFFFF", Some(2)).is_err());
        assert_eq!(colors, original);
        assert!(save_custom(&mut colors, "not a color", Some(0)).is_err());
        assert_eq!(colors, original);
        assert_eq!(save_custom(&mut colors, "#ABCDEF", Some(1)).unwrap(), 1);
        assert_eq!(colors, vec!["#FFFFFF", "#ABCDEF"]);

        let mut invalid = vec!["#AA0000".into(), "invalid".into()];
        let original = invalid.clone();
        assert!(save_custom(&mut invalid, "#AA0000", None).is_err());
        assert_eq!(invalid, original);
        let mut oversized: Vec<_> = (0..=MAX_CUSTOM_COLORS)
            .map(|index| format!("#{index:06X}"))
            .collect();
        let original = oversized.clone();
        assert!(save_custom(&mut oversized, "#FFFFFF", Some(0)).is_err());
        assert_eq!(oversized, original);
    }
}
