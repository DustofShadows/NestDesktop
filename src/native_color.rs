//! Native RGB picker with per-profile custom swatches.
use std::ffi::c_void;
use windows::Win32::Foundation::HWND;

#[repr(C)]
struct ChooseColor {
    size: u32,
    owner: *mut c_void,
    instance: *mut c_void,
    result: u32,
    custom: *mut u32,
    flags: u32,
    data: isize,
    hook: *const c_void,
    template: *const u16,
}

#[link(name = "comdlg32")]
extern "system" {
    fn ChooseColorW(dialog: *mut ChooseColor) -> i32;
}

fn rgb(value: &str) -> Option<u32> {
    let raw = value.strip_prefix('#')?;
    if raw.len() != 6 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(raw, 16).ok()?;
    Some(((value >> 16) & 255) | (value & 0xff00) | ((value & 255) << 16))
}

fn hex(colorref: u32) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        colorref & 255,
        (colorref >> 8) & 255,
        (colorref >> 16) & 255
    )
}

/// Choose any RGB color. Custom swatches are returned for profile persistence.
/// Cancelling leaves both the group and its saved palette unchanged.
pub unsafe fn choose(
    owner: HWND,
    initial: &str,
    custom: &[String],
) -> Option<(String, Vec<String>)> {
    let mut colors = [0x00ff_ffff; 16];
    for (slot, value) in colors.iter_mut().zip(custom) {
        *slot = rgb(value).unwrap_or(0x00ff_ffff);
    }
    let mut dialog = ChooseColor {
        size: std::mem::size_of::<ChooseColor>() as u32,
        owner: owner.0,
        instance: std::ptr::null_mut(),
        result: rgb(initial).unwrap_or(0x00ff_bcc9),
        custom: colors.as_mut_ptr(),
        flags: 0x0000_0001 | 0x0000_0002 | 0x0000_0100, // RGBINIT, FULLOPEN, ANYCOLOR
        data: 0,
        hook: std::ptr::null(),
        template: std::ptr::null(),
    };
    if ChooseColorW(&mut dialog) == 0 {
        return None;
    }
    Some((hex(dialog.result), colors.into_iter().map(hex).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgb_roundtrip_and_invalid_values() {
        for color in [
            "#000000", "#FFFFFF", "#12aBef", "#FF0000", "#00FF00", "#0000FF",
        ] {
            assert_eq!(hex(rgb(color).unwrap()), color.to_uppercase());
        }
        assert_eq!(rgb("#FF0000"), Some(0x0000ff));
        for bad in ["red", "123456", "#000", "#GG0000", "#12345600"] {
            assert!(rgb(bad).is_none());
        }
    }
}
