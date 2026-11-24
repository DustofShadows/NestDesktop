//! The same four-color mark on native Windows surfaces and the settings header.
use crate::render::{rect, Canvas};
use std::{cell::RefCell, collections::HashMap};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            WindowsAndMessaging::*,
        },
    },
};

thread_local! {
    // LR_SHARED icons belong to the executable module and must not be destroyed.
    static ICONS: RefCell<HashMap<i32, HICON>> = RefCell::new(HashMap::new());
}

pub unsafe fn icon(size: i32) -> HICON {
    let size = size.clamp(16, 256);
    ICONS.with(|icons| {
        *icons.borrow_mut().entry(size).or_insert_with(|| {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            LoadImageW(
                HINSTANCE(instance.0),
                PCWSTR(1usize as *const u16),
                IMAGE_ICON,
                size,
                size,
                LR_SHARED,
            )
            .map(|handle| HICON(handle.0))
            .unwrap_or_default()
        })
    })
}

pub unsafe fn small_icon(dpi: u32) -> HICON {
    icon(GetSystemMetricsForDpi(SM_CXSMICON, dpi.max(96)))
}

pub unsafe fn apply_window(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let dpi = GetDpiForWindow(hwnd).max(96);
    let small = small_icon(dpi);
    let large = icon(GetSystemMetricsForDpi(SM_CXICON, dpi));
    SendMessageW(
        hwnd,
        WM_SETICON,
        WPARAM(ICON_SMALL as usize),
        LPARAM(small.0 as isize),
    );
    SendMessageW(
        hwnd,
        WM_SETICON,
        WPARAM(ICON_BIG as usize),
        LPARAM(large.0 as isize),
    );
}

/// Match SetupWindow.DrawMark's 58-unit geometry and exact four-color palette.
pub unsafe fn draw_mark(c: &Canvas, x: f32, y: f32, size: f32) {
    let scale = size / 58.;
    c.fill(
        rect(x + scale, y + scale, 55. * scale, 55. * scale),
        0xc9bcff,
        14. * scale,
    );
    for (index, color) in [0x6c5ad2, 0x8f7de7, 0x619fb2, 0xe9ad75]
        .into_iter()
        .enumerate()
    {
        c.fill(
            rect(
                x + (12. + (index % 2) as f32 * 18.) * scale,
                y + (12. + (index / 2) as f32 * 18.) * scale,
                14. * scale,
                14. * scale,
            ),
            color,
            4. * scale,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP};

    #[test]
    fn embedded_windows_icons_load_at_requested_sizes_and_share_handles() {
        unsafe {
            for size in [16, 20, 24, 32, 40, 48, 64, 128, 256] {
                let handle = icon(size);
                assert!(!handle.is_invalid(), "Embedded icon must load at {size}px");
                assert_eq!(handle, icon(size));
                let mut info = ICONINFO::default();
                GetIconInfo(handle, &mut info).unwrap();
                let mut bitmap = BITMAP::default();
                assert_ne!(
                    GetObjectW(
                        info.hbmColor,
                        std::mem::size_of::<BITMAP>() as i32,
                        Some(&mut bitmap as *mut _ as *mut _)
                    ),
                    0
                );
                assert_eq!((bitmap.bmWidth, bitmap.bmHeight), (size, size));
                let _ = DeleteObject(info.hbmColor);
                let _ = DeleteObject(info.hbmMask);
            }
        }
    }
}
