//! Explicit compositor check using only an owned popup and an owned underlay.
//! No image is captured, no input is sent, and no timer or idle work is added.
use serde::Serialize;
use std::{fs, path::Path};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::*,
        Graphics::{Dwm::DwmFlush, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

const MIN_DELTA: f64 = 8.;

struct Probe {
    hwnd: HWND,
    class: Vec<u16>,
    instance: HINSTANCE,
}
impl Drop for Probe {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(PCWSTR(self.class.as_ptr()), self.instance);
        }
    }
}
struct ScreenDc(HDC);
impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseDC(HWND::default(), self.0);
        }
    }
}

unsafe extern "system" fn probe_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut bounds = RECT::default();
            let _ = GetClientRect(hwnd, &mut bounds);
            let brush = CreateSolidBrush(COLORREF(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32));
            FillRect(dc, &bounds, brush);
            let _ = DeleteObject(brush);
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

impl Probe {
    unsafe fn new(popup: HWND, bounds: RECT) -> Result<Self, String> {
        let class: Vec<_> = format!("NestDesktop.PopupAlphaProbe.{}", std::process::id())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let instance: HINSTANCE = GetModuleHandleW(None).map_err(|e| e.to_string())?.into();
        if RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(probe_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        }) == 0
        {
            return Err(format!(
                "Could not register the owned probe: {}",
                windows::core::Error::from_win32()
            ));
        }
        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class.as_ptr()),
            w!("NestDesktop — Transparency probe"),
            WS_POPUP,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            instance,
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                let _ = UnregisterClassW(PCWSTR(class.as_ptr()), instance);
                return Err(format!("Could not create the owned probe: {error}"));
            }
        };
        let probe = Self {
            hwnd,
            class,
            instance,
        };
        SetWindowPos(
            probe.hwnd,
            popup,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        )
        .map_err(|e| format!("Could not position the owned probe: {e}"))?;
        Ok(probe)
    }
    unsafe fn paint(&self, color: u32) -> Result<(), String> {
        SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, color as isize);
        if !RedrawWindow(
            self.hwnd,
            None,
            HRGN::default(),
            RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
        )
        .as_bool()
        {
            return Err("Could not paint the owned probe.".into());
        }
        // Bounded compositor synchronization; never used during ordinary use.
        for _ in 0..3 {
            DwmFlush().map_err(|e| format!("Could not synchronize the compositor: {e}"))?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct Sample {
    x: i32,
    y: i32,
    black_underlay_rgb: Option<[u8; 3]>,
    white_underlay_rgb: Option<[u8; 3]>,
    mean_channel_delta: Option<f64>,
}
#[derive(Serialize)]
struct Report {
    passed: bool,
    status: String,
    reason: Option<String>,
    popup_bounds: Option<[i32; 4]>,
    valid_samples: usize,
    mean_channel_delta: Option<f64>,
    required_mean_channel_delta: f64,
    limitations: &'static str,
    samples: Vec<Sample>,
}
fn rgb(pixel: u32) -> Option<[u8; 3]> {
    (pixel != CLR_INVALID).then_some([
        (pixel & 255) as u8,
        ((pixel >> 8) & 255) as u8,
        ((pixel >> 16) & 255) as u8,
    ])
}
fn delta(black: Option<[u8; 3]>, white: Option<[u8; 3]>) -> Option<f64> {
    let (black, white) = (black?, white?);
    Some(
        white
            .into_iter()
            .zip(black)
            .map(|(w, b)| w as f64 - b as f64)
            .sum::<f64>()
            / 3.,
    )
}
unsafe fn unoccluded(popup: HWND, probe: HWND, points: &[POINT]) -> bool {
    IsWindowVisible(popup).as_bool()
        && GetWindow(probe, GW_HWNDPREV).ok() == Some(popup)
        && points.iter().all(|&p| WindowFromPoint(p) == popup)
}
unsafe fn inspect(popup: HWND, report: &mut Report) -> Result<(), String> {
    let mut owner = 0;
    GetWindowThreadProcessId(popup, Some(&mut owner));
    if !IsWindow(popup).as_bool() || owner != std::process::id() {
        return Err("The popup is not owned by this process.".into());
    }
    if !IsWindowVisible(popup).as_bool() {
        return Err("Open a folder before requesting this check.".into());
    }
    let mut bounds = RECT::default();
    GetWindowRect(popup, &mut bounds).map_err(|e| e.to_string())?;
    let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    if width < 100 || height < 100 {
        return Err("Wait until the folder expansion has settled.".into());
    }
    report.popup_bounds = Some([bounds.left, bounds.top, bounds.right, bounds.bottom]);
    let inset = (width / 60).clamp(6, 12);
    let points: Vec<_> = [0.40, 0.55, 0.65]
        .into_iter()
        .flat_map(|fraction| {
            [bounds.left + inset, bounds.right - inset - 1].map(|x| POINT {
                x,
                y: bounds.top + (height as f64 * fraction) as i32,
            })
        })
        .collect();
    if points.iter().any(|&p| WindowFromPoint(p) != popup) {
        return Err("A background sample is occluded or intersects a child control.".into());
    }
    let dc = ScreenDc(GetDC(HWND::default()));
    if dc.0 .0.is_null() {
        return Err("The screen device context is unavailable.".into());
    }
    let probe = Probe::new(popup, bounds)?;
    probe.paint(0x000000)?;
    if !unoccluded(popup, probe.hwnd, &points) {
        return Err("The popup is occluded or the probe is not directly behind it.".into());
    }
    let black: Vec<_> = points
        .iter()
        .map(|p| rgb(GetPixel(dc.0, p.x, p.y).0))
        .collect();
    probe.paint(0xffffff)?;
    if !unoccluded(popup, probe.hwnd, &points) {
        return Err("The popup became occluded during the white-underlay sample.".into());
    }
    for (point, black) in points.into_iter().zip(black) {
        let white = rgb(GetPixel(dc.0, point.x, point.y).0);
        report.samples.push(Sample {
            x: point.x,
            y: point.y,
            black_underlay_rgb: black,
            white_underlay_rgb: white,
            mean_channel_delta: delta(black, white),
        });
    }
    // Probe is destroyed on every path, restoring the actual underlay unchanged.
    drop(probe);
    let _ = DwmFlush();
    report.valid_samples = report
        .samples
        .iter()
        .filter(|s| s.mean_channel_delta.is_some())
        .count();
    if report.valid_samples != report.samples.len() || report.valid_samples < 4 {
        return Err("Too few valid compositor samples; no transparency claim can be made.".into());
    }
    let mean = report
        .samples
        .iter()
        .filter_map(|s| s.mean_channel_delta)
        .sum::<f64>()
        / report.valid_samples as f64;
    report.mean_channel_delta = Some(mean);
    report.passed = mean >= MIN_DELTA;
    report.status = if report.passed { "passed" } else { "failed" }.into();
    if !report.passed {
        report.reason = Some(format!("The black/white underlay changed the popup by only {mean:.2} channel levels; at least {MIN_DELTA:.0} are required."));
    }
    Ok(())
}

/// Explicit UI-thread QA only. Samples six owned-popup background pixels twice.
pub(crate) unsafe fn verify(popup: HWND, path: &Path) -> Result<(), String> {
    let mut report = Report {
        passed: false, status: "inconclusive".into(), reason: None,
        popup_bounds: None, valid_samples: 0, mean_channel_delta: None,
        required_mean_channel_delta: MIN_DELTA,
        limitations: "Bounded black/white owned-underlay comparison on this compositor only. HDR/color conversion, material transitions and occlusion can affect results. It verifies visible background transmission, not blur quality, every pixel, or other Windows/DPI settings.",
        samples: Vec::new(),
    };
    if let Err(error) = inspect(popup, &mut report) {
        report.reason = Some(error);
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Could not write the popup transparency report: {e}"))?;
    if report.passed {
        Ok(())
    } else {
        Err(report
            .reason
            .unwrap_or_else(|| "Popup transparency was not verified.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_pixels_cannot_establish_transparency_and_color_order_is_correct() {
        assert_eq!(rgb(0x123456), Some([0x56, 0x34, 0x12]));
        assert_eq!(rgb(CLR_INVALID), None);
        assert_eq!(delta(None, Some([255; 3])), None);
        assert_eq!(delta(Some([10, 20, 30]), Some([30, 40, 50])), Some(20.));
        assert_eq!(delta(Some([30; 3]), Some([10; 3])), Some(-20.));
    }
}
