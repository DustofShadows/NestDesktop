//! A click-through desktop outline showing where the dragged group will land.
//! Pixels are cached for a size/color; moving the outline only moves its HWND.
//! There is no timer or ordinary application window behind this preview.

use crate::{platform, surface::LayeredSurface};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Gdi::{BeginPaint, EndPaint, PAINTSTRUCT},
            Imaging::{CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory},
        },
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Appearance {
    width: u32,
    height: u32,
    scale_milli: u32,
    accent: u32,
}

#[derive(Default)]
pub(crate) struct SnapPreview {
    window: HWND,
    surface: Option<LayeredSurface>,
    appearance: Option<Appearance>,
    bounds: Option<[i32; 4]>,
    visible: bool,
}

unsafe extern "system" fn wndproc(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match message {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            // UpdateLayeredWindow retains the pixels. Validate only; no redraw
            // or borrowing the application's state is needed on exposure.
            let mut paint = PAINTSTRUCT::default();
            let _ = BeginPaint(window, &mut paint);
            let _ = EndPaint(window, &paint);
            LRESULT(0)
        }
        _ => DefWindowProcW(window, message, wp, lp),
    }
}

fn accent(value: &str) -> u32 {
    u32::from_str_radix(value.trim_start_matches('#'), 16)
        .ok()
        .filter(|value| *value <= 0xff_ffff)
        .unwrap_or(0xc9bcff)
}

// Signed distance to an inset rounded rectangle, in physical pixels.
fn rounded_distance(x: f32, y: f32, width: f32, height: f32, inset: f32, radius: f32) -> f32 {
    let half_width = (width / 2. - inset).max(0.);
    let half_height = (height / 2. - inset).max(0.);
    let radius = radius.min(half_width).min(half_height).max(0.);
    let qx = (x - width / 2.).abs() - (half_width - radius);
    let qy = (y - height / 2.).abs() - (half_height - radius);
    qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius
}

fn pixels(appearance: Appearance) -> Vec<u8> {
    let scale = appearance.scale_milli as f32 / 1000.;
    let width = appearance.width;
    let height = appearance.height;
    let mut output = vec![0; width as usize * height as usize * 4];
    // Pale accent remains recognizable over both light and dark wallpapers.
    let red = (((appearance.accent >> 16) & 255) + 255) / 2;
    let green = (((appearance.accent >> 8) & 255) + 255) / 2;
    let blue = ((appearance.accent & 255) + 255) / 2;
    for y in 0..height {
        for x in 0..width {
            let outer = (0.5
                - rounded_distance(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    width as f32,
                    height as f32,
                    scale,
                    21. * scale,
                ))
            .clamp(0., 1.);
            let inner = (0.5
                - rounded_distance(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    width as f32,
                    height as f32,
                    3. * scale,
                    19. * scale,
                ))
            .clamp(0., 1.);
            let alpha = ((outer - inner).max(0.) * 208. + inner * 16.).round() as u32;
            let offset = (y as usize * width as usize + x as usize) * 4;
            output[offset] = ((blue * alpha + 127) / 255) as u8;
            output[offset + 1] = ((green * alpha + 127) / 255) as u8;
            output[offset + 2] = ((red * alpha + 127) / 255) as u8;
            output[offset + 3] = alpha as u8;
        }
    }
    output
}

impl SnapPreview {
    /// Physical screen bounds must be exactly the rectangle used on drop.
    /// The caller should hide the preview when snapping is disabled.
    pub(crate) unsafe fn show(
        &mut self,
        screen_bounds: RECT,
        scale: f32,
        color: &str,
    ) -> Result<(), String> {
        let width = screen_bounds.right - screen_bounds.left;
        let height = screen_bounds.bottom - screen_bounds.top;
        if !(6..=2048).contains(&width)
            || !(6..=2048).contains(&height)
            || !scale.is_finite()
            || scale <= 0.
        {
            self.hide();
            return Err("The snap preview bounds are invalid.".into());
        }
        if !IsWindow(self.window).as_bool() {
            self.surface = None;
            self.appearance = None;
            self.bounds = None;
            self.visible = false;
            let instance = GetModuleHandleW(None).map_err(|error| error.to_string())?;
            // Re-registration after Explorer destroyed a child is harmless;
            // CreateWindowExW uses the class already registered by this process.
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("NestDesktopSnapPreview"),
                ..Default::default()
            });
            self.window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                w!("NestDesktopSnapPreview"),
                w!(""),
                WS_POPUP,
                0,
                0,
                width,
                height,
                HWND::default(),
                None,
                instance,
                None,
            )
            .map_err(|error| error.to_string())?;
            if let Err(error) = platform::attach_to_desktop(self.window) {
                let _ = DestroyWindow(self.window);
                self.window = HWND::default();
                return Err(error);
            }
        }
        let appearance = Appearance {
            width: width as u32,
            height: height as u32,
            scale_milli: (scale * 1000.).round().max(1.) as u32,
            accent: accent(color),
        };
        let bounds = [
            screen_bounds.left,
            screen_bounds.top,
            screen_bounds.right,
            screen_bounds.bottom,
        ];
        if self.bounds != Some(bounds) || !self.visible {
            let (x, y) = platform::desktop_coordinates(screen_bounds.left, screen_bounds.top);
            SetWindowPos(
                self.window,
                HWND_TOP,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
            .map_err(|error| error.to_string())?;
            self.bounds = Some(bounds);
            self.visible = true;
        } else {
            // The moving tile is raised as it follows the pointer. Keep the
            // click-through destination visible above it without repainting.
            SetWindowPos(
                self.window,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
            .map_err(|error| error.to_string())?;
        }
        if self.appearance != Some(appearance) {
            let data = pixels(appearance);
            let imaging: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| error.to_string())?;
            let bitmap = imaging
                .CreateBitmapFromMemory(
                    appearance.width,
                    appearance.height,
                    &GUID_WICPixelFormat32bppPBGRA,
                    appearance.width * 4,
                    &data,
                )
                .map_err(|error| error.to_string())?;
            let mut surface = LayeredSurface::new(self.window, appearance.width, appearance.height)
                .map_err(|error| error.to_string())?;
            surface
                .present(&bitmap)
                .map_err(|error| error.to_string())?;
            self.surface = Some(surface);
            self.appearance = Some(appearance);
        }
        Ok(())
    }

    pub(crate) unsafe fn hide(&mut self) {
        if IsWindow(self.window).as_bool() && self.visible {
            let _ = ShowWindow(self.window, SW_HIDE);
        }
        self.visible = false;
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.window
    }
}

impl Drop for SnapPreview {
    fn drop(&mut self) {
        unsafe {
            self.surface = None;
            if IsWindow(self.window).as_bool() {
                let _ = DestroyWindow(self.window);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_keeps_corners_clear_and_interior_translucent_at_multiple_scales() {
        for scale in [1., 1.5, 2.] {
            let width = (104. * scale) as u32;
            let height = (114. * scale) as u32;
            let data = pixels(Appearance {
                width,
                height,
                scale_milli: (scale * 1000.) as u32,
                accent: 0x2bb6a5,
            });
            let alpha = |x, y| data[((y * width + x) * 4 + 3) as usize];
            for (x, y) in [
                (0, 0),
                (width - 1, 0),
                (0, height - 1),
                (width - 1, height - 1),
            ] {
                assert_eq!(alpha(x, y), 0, "exterior must show the actual desktop");
            }
            assert_eq!(alpha(width / 2, height / 2), 16);
            assert!(data.chunks_exact(4).any(|pixel| pixel[3] >= 200));
            assert!(data
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 16 && pixel[3] < 200));
            assert!(data
                .chunks_exact(4)
                .all(|pixel| pixel[..3].iter().all(|c| *c <= pixel[3])));
        }
    }
}
