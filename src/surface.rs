//! Native surfaces for antialiased window edges.
//!
//! Desktop tiles have per-pixel premultiplied alpha. Binary GDI window regions
//! are deliberately not used: they discard edge coverage and expose dark
//! pixels around the round card. Ordinary top-level windows keep native child
//! controls and ask DWM to round the frame instead.

use std::ffi::c_void;
use windows::{
    core::{w, Error, Result, HRESULT},
    Win32::{
        Foundation::{BOOL, COLORREF, HANDLE, HWND, SIZE},
        Graphics::{Dwm::*, Gdi::*, Imaging::IWICBitmap},
        UI::{Controls::MARGINS, WindowsAndMessaging::*},
    },
};

/// Configure a tile's alpha surface or a top-level window's native DWM frame.
/// Call before its first paint. Per-pixel tiles must never use
/// SetLayeredWindowAttributes; ordinary HWND-rendered popups can use it.
pub unsafe fn round_window(hwnd: HWND) -> bool {
    // A previous region disables DWM rounding and would clip the alpha fringe.
    SetWindowRgn(hwnd, HRGN::default(), false);
    let kind = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if kind >= crate::TILE {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
        return true;
    }

    let dark = BOOL(1);
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        &dark as *const _ as *const c_void,
        std::mem::size_of::<BOOL>() as u32,
    );
    let rgb = crate::render::BG;
    let border = COLORREF(((rgb & 255) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 255));
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_BORDER_COLOR,
        &border as *const _ as *const c_void,
        std::mem::size_of::<COLORREF>() as u32,
    );
    let preference = DWMWCP_ROUND;
    // Windows 10 does not support this attribute. A normal square native frame
    // is a safer fallback than a jagged GDI clip or a layered EDIT/LISTBOX host.
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &preference as *const _ as *const c_void,
        std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
    )
    .is_ok()
}

pub unsafe fn is_layered(hwnd: HWND) -> bool {
    GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_LAYERED.0 as isize != 0
}

/// A constant-opacity popup still paints through its ordinary HWND target so
/// native EDIT controls work normally. Only tiles use UpdateLayeredWindow.
pub unsafe fn uses_per_pixel_alpha(hwnd: HWND) -> bool {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) >= crate::TILE && is_layered(hwnd)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassMode {
    CompositedAlpha,
    UniformOpacity,
}

// Windows 11 24H2+: opt the redirected HWND bitmap into premultiplied alpha.
// windows 0.58 predates this documented attribute. Without it newer DWM can
// accept a backdrop while still treating every client pixel as opaque.
// https://learn.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
const DWMWA_REDIRECTIONBITMAP_ALPHA: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(39);
pub const POPUP_TINT_OPACITY: f32 = 0.82;
pub const POPUP_HEADER_OPACITY: f32 = 0.18;
const FALLBACK_OPACITY: u8 = 220;

/// Compose the popup directly over its actual underlay. System Acrylic can
/// silently become solid, so it must not sit behind our translucent pixels.
/// Native text/icons remain opaque; older systems use constant window opacity.
pub unsafe fn apply_glass(hwnd: HWND) -> Result<GlassMode> {
    if GetWindowLongPtrW(hwnd, GWLP_USERDATA) != crate::POPUP {
        return Err(Error::from_hresult(HRESULT(0x80070057u32 as i32)));
    }
    let backdrop = DWMSBT_NONE;
    let alpha = BOOL(1);
    let alpha_enabled = DwmSetWindowAttribute(
        hwnd,
        DWMWA_REDIRECTIONBITMAP_ALPHA,
        &alpha as *const _ as *const c_void,
        std::mem::size_of::<BOOL>() as u32,
    )
    .is_ok();
    if alpha_enabled
        && DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const _ as *const c_void,
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
        .is_ok()
        && DwmExtendFrameIntoClientArea(
            hwnd,
            &MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            },
        )
        .is_ok()
        && SetPropW(
            hwnd,
            w!("NestDesktop.AlphaSurface"),
            HANDLE(1usize as *mut _),
        )
        .is_ok()
    {
        return Ok(GlassMode::CompositedAlpha);
    }

    // Do not leave a partially configured backdrop which would make Canvas
    // choose a transparent render target for the constant-opacity fallback.
    let none = DWMSBT_NONE;
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_SYSTEMBACKDROP_TYPE,
        &none as *const _ as *const c_void,
        std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
    );
    let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
    let alpha = BOOL(0);
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_REDIRECTIONBITMAP_ALPHA,
        &alpha as *const _ as *const c_void,
        std::mem::size_of::<BOOL>() as u32,
    );
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
    SetLayeredWindowAttributes(hwnd, COLORREF(0), FALLBACK_OPACITY, LWA_ALPHA)?;
    Ok(GlassMode::UniformOpacity)
}

/// Queried once when a render target is created, never during idle operation.
pub unsafe fn has_alpha_surface(hwnd: HWND) -> bool {
    if GetWindowLongPtrW(hwnd, GWLP_USERDATA) != crate::POPUP || is_layered(hwnd) {
        return false;
    }
    !GetPropW(hwnd, w!("NestDesktop.AlphaSurface")).is_invalid()
}

/// Native EDIT controls use GDI, whose alpha bytes are undefined on a glass
/// client area. Give the control its own fully opaque redirected surface;
/// Windows then composes its normal text/caret/selection above the backdrop.
/// Child layering is documented on Windows 8+ (the app is Windows 10-aware).
/// Call after creating or reparenting an EDIT, before showing it. Reparenting
/// to an ordinary window removes the redirection again.
pub unsafe fn apply_opaque_control(hwnd: HWND) -> Result<()> {
    if !IsWindow(hwnd).as_bool() || GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_CHILD.0 as isize == 0 {
        return Err(Error::from_hresult(HRESULT(0x80070057u32 as i32)));
    }
    let parent = GetParent(hwnd)?;
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    if has_alpha_surface(parent) {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
        // Constant opacity deliberately ignores GDI's unreliable per-pixel
        // alpha. No color key, UpdateLayeredWindow or paint hook is involved.
        if let Err(error) = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) {
            // A layered HWND is initially invisible until attributes succeed.
            // Retain the ordinary native control if redirection is unavailable.
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
            let _ = RedrawWindow(hwnd, None, HRGN::default(), RDW_ERASE | RDW_INVALIDATE);
            return Err(error);
        }
    } else if style & WS_EX_LAYERED.0 as isize != 0 {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_LAYERED.0 as isize));
    } else {
        return Ok(());
    }
    let _ = RedrawWindow(
        hwnd,
        None,
        HRGN::default(),
        RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN,
    );
    Ok(())
}

/// Cached top-down 32-bit DIB and memory DC. Allocation is per window/resize,
/// never per paint. Direct2D renders into WIC; CopyPixels transfers its PBGRA
/// pixels directly into this DIB for the Windows compositor.
pub struct LayeredSurface {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u8,
    width: u32,
    height: u32,
    stride: u32,
    length: usize,
}

impl LayeredSurface {
    pub unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let length = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(4))
            .filter(|_| width > 0 && height > 0 && width <= 16384 && height <= 16384)
            .ok_or_else(|| Error::from_hresult(HRESULT(0x80070057u32 as i32)))?;
        let stride = width * 4;
        let dc = CreateCompatibleDC(HDC::default());
        if dc.0.is_null() {
            return Err(Error::from_win32());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: length,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels: *mut c_void = std::ptr::null_mut();
        let bitmap =
            match CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, HANDLE::default(), 0) {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error);
                }
            };
        let previous = SelectObject(dc, bitmap);
        if previous.0.is_null() || pixels.is_null() {
            let error = Error::from_win32();
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(dc);
            return Err(error);
        }
        Ok(Self {
            hwnd,
            dc,
            bitmap,
            previous,
            pixels: pixels as *mut u8,
            width,
            height,
            stride,
            length: length as usize,
        })
    }

    pub unsafe fn present(&mut self, bitmap: &IWICBitmap) -> Result<()> {
        let pixels = std::slice::from_raw_parts_mut(self.pixels, self.length);
        bitmap.CopyPixels(std::ptr::null(), self.stride, pixels)?;
        let source = windows::Win32::Foundation::POINT::default();
        let size = SIZE {
            cx: self.width as i32,
            cy: self.height as i32,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // Keep the existing position: no screen/client conversion and no drag
        // movement can be accidentally applied twice to an Explorer child.
        UpdateLayeredWindow(
            self.hwnd,
            HDC::default(),
            None,
            Some(&size),
            self.dc,
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    }
}

impl Drop for LayeredSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.dc);
        }
    }
}
