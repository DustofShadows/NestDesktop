use std::{cell::RefCell, collections::HashMap};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::{Direct2D::Common::*, Direct2D::*, DirectWrite::*, Dxgi::Common::*, Imaging::*},
        System::Com::*,
        UI::WindowsAndMessaging::DestroyIcon,
    },
};

pub const BG: u32 = 0x151925;
pub const PANEL: u32 = 0x1e2433;
pub const TEXT: u32 = 0xf4f3fc;
pub const MUTED: u32 = 0x9ea8bd;
pub const LAVENDER: u32 = 0xc9bcff;

pub fn color(hex: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 255) as f32 / 255.,
        g: ((hex >> 8) & 255) as f32 / 255.,
        b: (hex & 255) as f32 / 255.,
        a: 1.,
    }
}
pub fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
pub fn tint(hex: u32) -> u32 {
    // Secondary highlights blend with the same neutral panel behind them.
    // The preview itself uses fill_alpha so its selected color is preserved.
    let channel = |shift: u32| -> u32 {
        let accent = (hex >> shift) & 255;
        let background = (PANEL >> shift) & 255;
        (accent * 40 + background * 60 + 50) / 100
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

enum Backing {
    Window(ID2D1HwndRenderTarget),
    Layered {
        bitmap: IWICBitmap,
        surface: crate::surface::LayeredSurface,
    },
}

pub struct Canvas {
    target: ID2D1RenderTarget,
    backing: Backing,
    hwnd: HWND,
    size: (u32, u32),
    transparent: bool,
    write: IDWriteFactory,
    imaging: IWICImagingFactory,
    icons: HashMap<String, ID2D1Bitmap>,
}
type Resources = (ID2D1Factory, IDWriteFactory, IWICImagingFactory);
thread_local! {static RESOURCES:RefCell<Option<Resources>>=const{RefCell::new(None)};}

/// Drop the UI thread's factory cache while its COM apartment is still alive.
/// All Canvas values must have been released first.
pub unsafe fn release_resources() {
    RESOURCES.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn render_properties(transparent: bool) -> D2D1_RENDER_TARGET_PROPERTIES {
    D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: if transparent {
                D2D1_ALPHA_MODE_PREMULTIPLIED
            } else {
                D2D1_ALPHA_MODE_IGNORE
            },
        },
        dpiX: 96.,
        dpiY: 96.,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    }
}

/// Direct2D accepts straight-alpha brush colors and premultiplies the output.
/// Keeping the chosen RGB channels intact avoids shifting/darkening the hue.
unsafe fn fill_alpha_target(
    target: &ID2D1RenderTarget,
    r: D2D_RECT_F,
    hex: u32,
    radius: f32,
    opacity: f32,
) -> Result<()> {
    if !opacity.is_finite() || opacity <= 0. {
        return Ok(());
    }
    let brush_color = D2D1_COLOR_F {
        a: opacity.min(1.),
        ..color(hex)
    };
    let brush = target.CreateSolidColorBrush(&brush_color, None)?;
    if radius > 0. {
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            &brush,
        );
    } else {
        target.FillRectangle(&r, &brush);
    }
    Ok(())
}

/// Shared target-only path permits offscreen regression of the pixels that the
/// real desktop Canvas emits, without creating or revealing a native window.
unsafe fn radial_fill_target(
    target: &ID2D1RenderTarget,
    r: D2D_RECT_F,
    hex: u32,
    radius: f32,
    progress: f32,
) -> Result<()> {
    if !progress.is_finite() || progress <= 0. {
        return Ok(());
    }
    let brush = target.CreateSolidColorBrush(&color(hex), None)?;
    if progress >= 1. {
        target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            &brush,
        );
        return Ok(());
    }
    let factory = target.GetFactory()?;
    let mask = factory
        .CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
            rect: r,
            radiusX: radius,
            radiusY: radius,
        })
        .and_then(|g| g.cast::<ID2D1Geometry>())?;
    let mut layer = D2D1_LAYER_PARAMETERS {
        contentBounds: r,
        geometricMask: std::mem::ManuallyDrop::new(Some(mask)),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: windows::Foundation::Numerics::Matrix3x2 {
            M11: 1.,
            M12: 0.,
            M21: 0.,
            M22: 1.,
            M31: 0.,
            M32: 0.,
        },
        opacity: 1.,
        ..Default::default()
    };
    let half_w = (r.right - r.left) / 2.;
    let half_h = (r.bottom - r.top) / 2.;
    let extent = half_w.hypot(half_h) * progress;
    target.PushLayer(&layer, None);
    target.FillEllipse(
        &D2D1_ELLIPSE {
            point: D2D_POINT_2F {
                x: r.left + half_w,
                y: r.top + half_h,
            },
            radiusX: extent,
            radiusY: extent,
        },
        &brush,
    );
    target.PopLayer();
    std::mem::ManuallyDrop::drop(&mut layer.geometricMask);
    Ok(())
}

impl Canvas {
    pub unsafe fn new(hwnd: HWND, w: u32, h: u32) -> Result<Self> {
        let (factory, write, imaging) = RESOURCES.with(|slot| -> Result<Resources> {
            let mut r = slot.borrow_mut();
            if r.is_none() {
                *r = Some((
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                    DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?,
                ));
            }
            Ok(r.as_ref().unwrap().clone())
        })?;
        // These small, static desktop surfaces do not need a separate GPU device.
        // Software Direct2D still provides antialiasing; redraws only follow events.
        let layered = crate::surface::uses_per_pixel_alpha(hwnd);
        let transparent = layered || crate::surface::has_alpha_surface(hwnd);
        let props = render_properties(transparent);
        let (target, backing) = if layered {
            let bitmap =
                imaging.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
            let target = factory.CreateWicBitmapRenderTarget(&bitmap, &props)?;
            let surface = crate::surface::LayeredSurface::new(hwnd, w, h)?;
            (target, Backing::Layered { bitmap, surface })
        } else {
            let window = factory.CreateHwndRenderTarget(
                &props,
                &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd,
                    pixelSize: D2D_SIZE_U {
                        width: w,
                        height: h,
                    },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                },
            )?;
            let target = window.cast::<ID2D1RenderTarget>()?;
            (target, Backing::Window(window))
        };
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32;
        target.SetDpi(dpi, dpi);
        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        if transparent {
            // ClearType assumes an opaque background. Grayscale antialiasing
            // preserves coverage without colored fringes on transparent edges.
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
        Ok(Self {
            target,
            backing,
            hwnd,
            size: (w, h),
            transparent,
            write,
            imaging,
            icons: HashMap::new(),
        })
    }
    pub unsafe fn resize(&mut self, w: u32, h: u32) {
        if self.size == (w, h) {
            return;
        }
        match &self.backing {
            Backing::Window(window) => {
                if window
                    .Resize(&D2D_SIZE_U {
                        width: w,
                        height: h,
                    })
                    .is_ok()
                {
                    self.size = (w, h);
                    let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd).max(96) as f32;
                    self.target.SetDpi(dpi, dpi);
                }
            }
            Backing::Layered { .. } => {
                // WIC surfaces have fixed dimensions. Reallocate only on an
                // actual resize, not on hover, paint or opening a folder.
                if let Ok(replacement) = Self::new(self.hwnd, w, h) {
                    *self = replacement;
                }
            }
        }
    }
    pub unsafe fn begin(&self, bg: u32) {
        self.target.BeginDraw();
        if self.transparent {
            self.target.Clear(Some(&D2D1_COLOR_F {
                r: 0.,
                g: 0.,
                b: 0.,
                a: 0.,
            }));
        } else {
            self.target.Clear(Some(&color(bg)));
        }
    }
    pub unsafe fn end(&mut self) -> Result<()> {
        self.target.EndDraw(None, None)?;
        match &mut self.backing {
            Backing::Layered { bitmap, surface } => surface.present(bitmap),
            Backing::Window(_) => Ok(()),
        }
    }
    pub unsafe fn fill(&self, r: D2D_RECT_F, hex: u32, radius: f32) {
        let _ = fill_alpha_target(&self.target, r, hex, radius, 1.);
    }
    pub unsafe fn push_clip(&self, r: D2D_RECT_F) {
        self.target
            .PushAxisAlignedClip(&r, D2D1_ANTIALIAS_MODE_ALIASED);
    }
    pub unsafe fn pop_clip(&self) {
        self.target.PopAxisAlignedClip();
    }
    pub unsafe fn fill_alpha(&self, r: D2D_RECT_F, hex: u32, radius: f32, opacity: f32) {
        let _ = fill_alpha_target(&self.target, r, hex, radius, opacity);
    }
    pub unsafe fn outline(&self, r: D2D_RECT_F, hex: u32, radius: f32, width: f32) {
        if let Ok(b) = self.target.CreateSolidColorBrush(&color(hex), None) {
            self.target.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: radius,
                    radiusY: radius,
                },
                &b,
                width,
                None,
            );
        }
    }
    /// Circular dwell fill, clipped to the colored preview's rounded boundary.
    pub unsafe fn radial_fill(&self, r: D2D_RECT_F, hex: u32, radius: f32, progress: f32) {
        let _ = radial_fill_target(&self.target, r, hex, radius, progress);
    }
    unsafe fn gradient(
        &self,
        r: D2D_RECT_F,
        stops: &[D2D1_GRADIENT_STOP],
        vertical: bool,
        radius: f32,
    ) {
        let Ok(collection) =
            self.target
                .CreateGradientStopCollection(stops, D2D1_GAMMA_1_0, D2D1_EXTEND_MODE_CLAMP)
        else {
            return;
        };
        let Ok(brush) = self.target.CreateLinearGradientBrush(
            &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F {
                    x: r.left,
                    y: r.top,
                },
                endPoint: D2D_POINT_2F {
                    x: if vertical { r.left } else { r.right },
                    y: if vertical { r.bottom } else { r.top },
                },
            },
            None,
            &collection,
        ) else {
            return;
        };
        self.target.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            &brush,
        );
    }
    pub unsafe fn color_square(&self, r: D2D_RECT_F, hue: f32) {
        let pure = crate::palette::Hsv::new(hue * 360., 1., 1.)
            .color()
            .packed();
        self.gradient(
            r,
            &[
                D2D1_GRADIENT_STOP {
                    position: 0.,
                    color: color(0xffffff),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.,
                    color: color(pure),
                },
            ],
            false,
            6.,
        );
        let mut clear = color(0);
        clear.a = 0.;
        self.gradient(
            r,
            &[
                D2D1_GRADIENT_STOP {
                    position: 0.,
                    color: clear,
                },
                D2D1_GRADIENT_STOP {
                    position: 1.,
                    color: color(0),
                },
            ],
            true,
            6.,
        );
    }
    pub unsafe fn hue_strip(&self, r: D2D_RECT_F) {
        let stops: Vec<_> = [
            0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xff0000,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, rgb)| D2D1_GRADIENT_STOP {
            position: i as f32 / 6.,
            color: color(rgb),
        })
        .collect();
        self.gradient(r, &stops, false, 6.);
    }
    pub unsafe fn text(
        &self,
        t: &str,
        r: D2D_RECT_F,
        size: f32,
        hex: u32,
        bold: bool,
        center: bool,
    ) {
        let weight = if bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        if let (Ok(f), Ok(b)) = (
            self.write.CreateTextFormat(
                w!("Segoe UI"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            ),
            self.target.CreateSolidColorBrush(&color(hex), None),
        ) {
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            if center {
                let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            }
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let s: Vec<u16> = t.encode_utf16().collect();
            self.target.DrawText(
                &s,
                &f,
                &r,
                &b,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
    pub unsafe fn icon(&mut self, path: &str, r: D2D_RECT_F) -> bool {
        if !self.icons.contains_key(path) {
            if self.icons.len() >= 256 {
                self.icons.clear();
            }
            if let Some(hicon) = crate::platform::icon_for_path(path, 48) {
                if let Ok(source) = self.imaging.CreateBitmapFromHICON(hicon) {
                    if let Ok(converter) = self.imaging.CreateFormatConverter() {
                        if converter
                            .Initialize(
                                &source,
                                &GUID_WICPixelFormat32bppPBGRA,
                                WICBitmapDitherTypeNone,
                                None,
                                0.,
                                WICBitmapPaletteTypeCustom,
                            )
                            .is_ok()
                        {
                            if let Ok(bitmap) =
                                self.target.CreateBitmapFromWicBitmap(&converter, None)
                            {
                                self.icons.insert(path.to_string(), bitmap);
                            }
                        }
                    }
                }
                let _ = DestroyIcon(hicon);
            }
        }
        if let Some(bitmap) = self.icons.get(path) {
            self.target.DrawBitmap(
                bitmap,
                Some(&r),
                1.,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_color_overlay_preserves_hue_and_transparent_rounded_edges() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).unwrap();
            struct Apartment;
            impl Drop for Apartment {
                fn drop(&mut self) {
                    unsafe { CoUninitialize() }
                }
            }
            let _apartment = Apartment;
            let result = (|| -> Result<()> {
                let factory: ID2D1Factory =
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
                let imaging: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                let render = |accent, background: Option<u32>| -> Result<Vec<u8>> {
                    let bitmap = imaging.CreateBitmap(
                        40,
                        40,
                        &GUID_WICPixelFormat32bppPBGRA,
                        WICBitmapCacheOnLoad,
                    )?;
                    let target =
                        factory.CreateWicBitmapRenderTarget(&bitmap, &render_properties(true))?;
                    target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    target.BeginDraw();
                    target.Clear(Some(&background.map(color).unwrap_or(D2D1_COLOR_F {
                        r: 0.,
                        g: 0.,
                        b: 0.,
                        a: 0.,
                    })));
                    fill_alpha_target(&target, rect(2., 2., 36., 36.), accent, 12., 0.72)?;
                    target.EndDraw(None, None)?;
                    let mut pixels = vec![0; 40 * 40 * 4];
                    bitmap.CopyPixels(std::ptr::null(), 40 * 4, &mut pixels)?;
                    Ok(pixels)
                };
                let center = (20 * 40 + 20) * 4;
                for accent in [0xff5277, 0x35ce97, 0x539cff] {
                    let pixels = render(accent, None)?;
                    let center = &pixels[center..center + 4];
                    assert!(center[3].abs_diff(184) <= 1);
                    for (channel, shift) in [(0, 0), (1, 8), (2, 16)] {
                        let selected = (accent >> shift) & 255;
                        let straight = center[channel] as f32 * 255. / center[3] as f32;
                        assert!(
                            (straight - selected as f32).abs() <= 2.,
                            "translucency must preserve the selected RGB hue"
                        );
                    }
                    for (x, y) in [(0, 0), (2, 2), (37, 2), (2, 37), (37, 37)] {
                        let offset = (y * 40 + x) * 4;
                        assert_eq!(&pixels[offset..offset + 4], &[0, 0, 0, 0]);
                    }
                    assert!(pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 180));
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .all(|p| p[..3].iter().all(|channel| *channel <= p[3])),
                        "rounded alpha must remain valid premultiplied BGRA"
                    );
                    let overlay = render(accent, Some(PANEL))?;
                    let center_offset = (20 * 40 + 20) * 4;
                    let center = &overlay[center_offset..center_offset + 4];
                    assert_eq!(center[3], 255);
                    for (channel, shift) in [(0, 0), (1, 8), (2, 16)] {
                        let selected = ((accent >> shift) & 255) as f32;
                        let background = ((PANEL >> shift) & 255) as f32;
                        let expected = selected * 0.72 + background * 0.28;
                        assert!(
                            (center[channel] as f32 - expected).abs() <= 2.,
                            "the preview must alpha-composite the selected color, not crush it with a dark tint"
                        );
                    }
                }
                Ok(())
            })();
            result.unwrap();
        }
    }

    /// Exercises the exact target-only path called by Canvas::radial_fill,
    /// including Direct2D's rounded geometric layer on a premultiplied WIC
    /// bitmap. No native window or desktop manipulation is involved.
    #[test]
    fn radial_hover_fill_grows_evenly_and_keeps_the_rounded_preview_clipped() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).unwrap();
            struct Apartment;
            impl Drop for Apartment {
                fn drop(&mut self) {
                    unsafe { CoUninitialize() }
                }
            }
            let _apartment = Apartment;
            let result = (|| -> Result<()> {
                let factory: ID2D1Factory =
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
                let imaging: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                let preview = rect(14., 9., 76., 76.);
                let render = |progress, background: Option<u32>| -> Result<Vec<u8>> {
                    let bitmap = imaging.CreateBitmap(
                        112,
                        114,
                        &GUID_WICPixelFormat32bppPBGRA,
                        WICBitmapCacheOnLoad,
                    )?;
                    let target =
                        factory.CreateWicBitmapRenderTarget(&bitmap, &render_properties(true))?;
                    target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    target.BeginDraw();
                    target.Clear(Some(&background.map(color).unwrap_or(D2D1_COLOR_F {
                        r: 0.,
                        g: 0.,
                        b: 0.,
                        a: 0.,
                    })));
                    radial_fill_target(&target, preview, 0x4a5871, 16., progress)?;
                    target.EndDraw(None, None)?;
                    let mut data = vec![0; 112 * 114 * 4];
                    bitmap.CopyPixels(std::ptr::null(), 112 * 4, &mut data)?;
                    Ok(data)
                };
                fn pixel(data: &[u8], x: usize, y: usize) -> &[u8] {
                    let offset = (y * 112 + x) * 4;
                    &data[offset..offset + 4]
                }
                let zero = render(0., None)?;
                let middle = render(0.5, None)?;
                // Exercise clipping while the circle actually crosses the
                // rounded corners, rather than remaining inside the preview.
                let near_full = render(0.9, None)?;
                let full = render(1., None)?;
                assert!(zero.iter().all(|channel| *channel == 0));
                assert_eq!(pixel(&middle, 52, 47), &[0x71, 0x58, 0x4a, 255]);
                assert_eq!(pixel(&middle, 15, 47), &[0, 0, 0, 0]);
                for distance in [10, 20, 27, 32] {
                    let horizontal = pixel(&middle, 52 + distance, 47)[3];
                    let vertical = pixel(&middle, 52, 47 + distance)[3];
                    assert!(
                        horizontal.abs_diff(vertical) <= 2,
                        "dwell should spread circularly from its center"
                    );
                }
                for (x, y) in [(0, 0), (111, 113), (14, 9), (89, 9), (14, 84), (89, 84)] {
                    for image in [&middle, &near_full, &full] {
                        assert_eq!(
                            pixel(image, x, y),
                            &[0, 0, 0, 0],
                            "hover fill must not paint the exterior rounded corners"
                        );
                    }
                }
                for y in 0..114 {
                    for x in 0..112 {
                        // Analytic distance to this 76px rounded preview. Keep
                        // a 1px margin so this checks interior/exterior, while
                        // fractional boundary coverage is checked separately.
                        let qx = (x as f32 + 0.5 - 52.).abs() - 22.;
                        let qy = (y as f32 + 0.5 - 47.).abs() - 22.;
                        let distance = qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - 16.;
                        if distance <= -1. {
                            assert_eq!(
                                pixel(&full, x, y),
                                &[0x71, 0x58, 0x4a, 255],
                                "full progress must reach every preview interior pixel"
                            );
                        } else if distance >= 1. {
                            for image in [&middle, &near_full, &full] {
                                assert_eq!(pixel(image, x, y), &[0, 0, 0, 0]);
                            }
                        }
                    }
                }
                for image in [&middle, &near_full, &full] {
                    assert!(image.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
                    assert!(
                        image
                            .chunks_exact(4)
                            .all(|p| p[..3].iter().all(|channel| *channel <= p[3])),
                        "fractional edges must keep premultiplied color channels"
                    );
                }
                let unchanged = render(0., Some(0x1c2333))?;
                assert!(unchanged.chunks_exact(4).all(|p| p == [51, 35, 28, 255]));
                let overlay = render(0.9, Some(0x1c2333))?;
                for (x, y) in [(0, 0), (14, 9), (89, 9), (14, 84), (89, 84)] {
                    assert_eq!(pixel(&overlay, x, y), pixel(&unchanged, x, y));
                }
                Ok(())
            })();
            result.unwrap();
        }
    }

    /// Exercises the actual software Direct2D/PBGRA format used by desktop
    /// tiles, without creating a window or changing the user's desktop.
    #[test]
    fn rounded_tile_has_transparent_corners_and_antialiased_alpha() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).unwrap();
            let result = (|| -> Result<()> {
                let factory: ID2D1Factory =
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
                let imaging: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                let bitmap = imaging.CreateBitmap(
                    112,
                    138,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapCacheOnLoad,
                )?;
                let target =
                    factory.CreateWicBitmapRenderTarget(&bitmap, &render_properties(true))?;
                target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                target.BeginDraw();
                target.Clear(Some(&D2D1_COLOR_F {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                    a: 0.,
                }));
                let brush = target.CreateSolidColorBrush(&color(0x1c2333), None)?;
                target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(1., 1., 110., 136.),
                        radiusX: 21.,
                        radiusY: 21.,
                    },
                    &brush,
                );
                target.EndDraw(None, None)?;
                let mut pixels = vec![0; 112 * 138 * 4];
                bitmap.CopyPixels(std::ptr::null(), 112 * 4, &mut pixels)?;
                for (x, y) in [(0, 0), (111, 0), (0, 137), (111, 137)] {
                    let offset = (y * 112 + x) * 4;
                    assert_eq!(
                        &pixels[offset..offset + 4],
                        &[0, 0, 0, 0],
                        "outer corners must be transparent, not opaque black"
                    );
                }
                let center = (69 * 112 + 56) * 4;
                assert_eq!(&pixels[center..center + 4], &[51, 35, 28, 255]);
                assert!(
                    pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
                    "rounded edges must retain fractional antialias coverage"
                );
                assert!(pixels.chunks_exact(4).any(|p|p[3]>32 && p[3]<224 && p[0]>0), "edge color must be premultiplied with alpha rather than an opaque black fringe");
                assert!(
                    pixels
                        .chunks_exact(4)
                        .all(|p| p[..3].iter().all(|c| *c <= p[3])),
                    "UpdateLayeredWindow requires premultiplied color channels"
                );
                Ok(())
            })();
            CoUninitialize();
            result.unwrap();
        }
    }
}
