//! Explicit, bounded QA of our own tiles' on-screen corner compositing.
//!
//! This samples nine pixels per tile, briefly hides only that tile, then
//! restores it without activation. It captures no image and sends no input.
//! A static desktop is required: moving wallpaper, another changing surface,
//! HDR/color conversion or an occluding window can make the result inconclusive.

use serde::Serialize;
use std::{fs, path::Path};
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::{
        Dwm::DwmFlush,
        Gdi::{GetDC, GetPixel, ReleaseDC, UpdateWindow, CLR_INVALID, HDC},
    },
    UI::WindowsAndMessaging::{
        GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible, ShowWindow,
        WindowFromPoint, SW_HIDE, SW_SHOWNOACTIVATE,
    },
};

struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseDC(HWND::default(), self.0);
        }
    }
}

struct RestoreTile(HWND);

impl Drop for RestoreTile {
    fn drop(&mut self) {
        unsafe {
            let _ = ShowWindow(self.0, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(self.0);
            let _ = DwmFlush();
        }
    }
}

#[derive(Serialize)]
struct PixelPair {
    x: i32,
    y: i32,
    visible_rgb: Option<String>,
    hidden_rgb: Option<String>,
    matches: Option<bool>,
}

#[derive(Serialize)]
struct TileReport {
    index: usize,
    hwnd: String,
    bounds: Option<[i32; 4]>,
    status: String,
    reason: Option<String>,
    center_control: Option<PixelPair>,
    center_changed: bool,
    corners: Vec<PixelPair>,
    invalid_samples: usize,
    differing_corners: usize,
}

#[derive(Serialize)]
struct Report {
    passed: bool,
    tested_tiles: usize,
    tested_corner_samples: usize,
    differing_corner_samples: usize,
    invalid_samples: usize,
    limitations: &'static str,
    tiles: Vec<TileReport>,
}

fn rgb(value: u32) -> Option<String> {
    (value != CLR_INVALID).then(|| {
        format!(
            "#{:02X}{:02X}{:02X}",
            value & 0xff,
            (value >> 8) & 0xff,
            (value >> 16) & 0xff
        )
    })
}

fn pair(point: POINT, visible: u32, hidden: u32) -> PixelPair {
    PixelPair {
        x: point.x,
        y: point.y,
        visible_rgb: rgb(visible),
        hidden_rgb: rgb(hidden),
        matches: (visible != CLR_INVALID && hidden != CLR_INVALID).then_some(visible == hidden),
    }
}

/// Call explicitly on the application's UI thread, after revealing the desktop.
/// Every supplied HWND is checked for ownership before it can be hidden.
pub(crate) unsafe fn verify(tiles: &[HWND], path: &Path) -> Result<(), String> {
    let dc = ScreenDc(GetDC(HWND::default()));
    let dc_valid = !dc.0 .0.is_null();
    let mut report = Report {
        passed: false,
        tested_tiles: 0,
        tested_corner_samples: 0,
        differing_corner_samples: 0,
        invalid_samples: 0,
        limitations: "Exact RGB comparison on this desktop at this instant only. Moving wallpaper, other changing content, occlusion, HDR and color conversion can affect results. Unchanged or invalid center controls are inconclusive. This does not establish every rounded-edge pixel, DPI setting or Windows version.",
        tiles: Vec::with_capacity(tiles.len()),
    };
    for (index, &tile) in tiles.iter().enumerate() {
        let mut entry = TileReport {
            index,
            hwnd: format!("0x{:X}", tile.0 as usize),
            bounds: None,
            status: "skipped".into(),
            reason: None,
            center_control: None,
            center_changed: false,
            corners: Vec::new(),
            invalid_samples: 0,
            differing_corners: 0,
        };
        let mut owner = 0;
        GetWindowThreadProcessId(tile, Some(&mut owner));
        let mut bounds = RECT::default();
        let skip = if !IsWindow(tile).as_bool() || owner != std::process::id() {
            Some("Not a window owned by this process")
        } else if !IsWindowVisible(tile).as_bool() {
            Some("Tile is hidden")
        } else if !dc_valid {
            Some("Screen device context unavailable")
        } else if GetWindowRect(tile, &mut bounds).is_err()
            || bounds.right - bounds.left < 6
            || bounds.bottom - bounds.top < 6
        {
            Some("Tile bounds unavailable or too small")
        } else {
            None
        };
        if let Some(reason) = skip {
            entry.reason = Some(reason.into());
            report.tiles.push(entry);
            continue;
        }
        entry.bounds = Some([bounds.left, bounds.top, bounds.right, bounds.bottom]);
        let center = POINT {
            x: bounds.left + (bounds.right - bounds.left) / 2,
            y: bounds.top + (bounds.bottom - bounds.top) / 2,
        };
        if WindowFromPoint(center) != tile {
            entry.reason = Some("Tile center is occluded or does not receive desktop hits".into());
            report.tiles.push(entry);
            continue;
        }
        let points = [
            POINT {
                x: bounds.left + 1,
                y: bounds.top + 1,
            },
            POINT {
                x: bounds.left + 2,
                y: bounds.top + 2,
            },
            POINT {
                x: bounds.right - 2,
                y: bounds.top + 1,
            },
            POINT {
                x: bounds.right - 3,
                y: bounds.top + 2,
            },
            POINT {
                x: bounds.left + 1,
                y: bounds.bottom - 2,
            },
            POINT {
                x: bounds.left + 2,
                y: bounds.bottom - 3,
            },
            POINT {
                x: bounds.right - 2,
                y: bounds.bottom - 2,
            },
            POINT {
                x: bounds.right - 3,
                y: bounds.bottom - 3,
            },
        ];
        let _ = UpdateWindow(tile);
        if DwmFlush().is_err() {
            entry.reason = Some("DWM could not synchronize the visible sample".into());
            report.tiles.push(entry);
            continue;
        }
        let visible = points.map(|p| GetPixel(dc.0, p.x, p.y).0);
        let visible_center = GetPixel(dc.0, center.x, center.y).0;
        let _ = ShowWindow(tile, SW_HIDE);
        let restore = RestoreTile(tile);
        if DwmFlush().is_err() || IsWindowVisible(tile).as_bool() {
            drop(restore);
            entry.reason = Some("Tile hiding or DWM synchronization failed".into());
            report.tiles.push(entry);
            continue;
        }
        let hidden = points.map(|p| GetPixel(dc.0, p.x, p.y).0);
        let hidden_center = GetPixel(dc.0, center.x, center.y).0;
        drop(restore);

        let control = pair(center, visible_center, hidden_center);
        entry.center_changed = control.matches == Some(false);
        entry.invalid_samples += usize::from(control.matches.is_none());
        entry.center_control = Some(control);
        for ((point, before), after) in points.into_iter().zip(visible).zip(hidden) {
            let sample = pair(point, before, after);
            match sample.matches {
                None => entry.invalid_samples += 1,
                Some(false) => {
                    entry.differing_corners += 1;
                    report.tested_corner_samples += 1;
                }
                Some(true) => report.tested_corner_samples += 1,
            }
            entry.corners.push(sample);
        }
        let valid_corners = entry.corners.iter().filter(|p| p.matches.is_some()).count();
        if entry.center_changed && valid_corners > 0 {
            report.tested_tiles += 1;
            entry.status = if entry.differing_corners == 0 {
                "passed"
            } else {
                "failed"
            }
            .into();
        } else {
            entry.status = "inconclusive".into();
            entry.reason = Some("No valid changed center control or valid corner pair".into());
        }
        report.differing_corner_samples += entry.differing_corners;
        report.invalid_samples += entry.invalid_samples;
        report.tiles.push(entry);
    }
    report.passed = report.tested_tiles > 0 && report.differing_corner_samples == 0;
    let json = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| format!("Could not write transparency report: {e}"))?;
    if report.passed {
        Ok(())
    } else if report.differing_corner_samples > 0 {
        Err(format!(
            "{} desktop corner samples differed; see {}",
            report.differing_corner_samples,
            path.display()
        ))
    } else {
        Err(format!(
            "No unoccluded tile had a valid changed center control and corner sample; see {}",
            path.display()
        ))
    }
}
