//! Read Explorer's icon grid through its supported COM view interface.
//! No remote-process memory, injection, or pointer-bearing ListView messages.
use windows::{
    core::{Interface, VARIANT},
    Win32::{
        Foundation::POINT,
        Graphics::Gdi::ClientToScreen,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IServiceProvider,
            CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
        },
        UI::{
            HiDpi::GetSystemMetricsForDpi,
            Shell::{
                IFolderView, IShellBrowser, IShellWindows, SID_STopLevelBrowser, ShellWindows,
                SWC_DESKTOP, SWFO_NEEDDISPATCH,
            },
            WindowsAndMessaging::{FindWindowExW, SM_CXICONSPACING, SM_CYICONSPACING},
        },
    },
};

#[derive(Clone, Copy, Debug)]
pub struct GridMetrics {
    pub cell_w: f32,
    pub cell_h: f32,
    /// First cell's origin in logical coordinates relative to primary work area.
    pub origin_x: f32,
    pub origin_y: f32,
}

fn valid_cell(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && (24.0..=512.0).contains(&value) {
        value
    } else {
        fallback
    }
}

/// Call on startup, a display/settings change, or when snapping is enabled.
/// Explorer's current spacing wins; system icon-spacing metrics are a fallback.
pub fn metrics(dpi: u32) -> GridMetrics {
    unsafe {
        let dpi = dpi.max(96);
        let scale = dpi as f32 / 96.;
        let mut grid = GridMetrics {
            cell_w: valid_cell(
                GetSystemMetricsForDpi(SM_CXICONSPACING, dpi) as f32 / scale,
                75.,
            ),
            cell_h: valid_cell(
                GetSystemMetricsForDpi(SM_CYICONSPACING, dpi) as f32 / scale,
                75.,
            ),
            origin_x: 0.,
            origin_y: 0.,
        };
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if init.is_err() {
            return grid;
        }
        let _ = (|| -> windows::core::Result<()> {
            let windows: IShellWindows =
                CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
            let empty = VARIANT::default();
            let mut desktop_handle = 0;
            let dispatch = windows.FindWindowSW(
                &empty,
                &empty,
                SWC_DESKTOP,
                &mut desktop_handle,
                SWFO_NEEDDISPATCH,
            )?;
            let provider = dispatch.cast::<IServiceProvider>()?;
            let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
            let view = browser.QueryActiveShellView()?.cast::<IFolderView>()?;
            let mut spacing = POINT::default();
            view.GetSpacing(&mut spacing)?;
            grid.cell_w = valid_cell(spacing.x as f32 / scale, grid.cell_w);
            grid.cell_h = valid_cell(spacing.y as f32 / scale, grid.cell_h);
            // Native item positions reveal the current grid phase, including a
            // user-adjusted desktop layout. The first item's PIDL is scoped.
            if let Ok(pidl) = view.Item(0) {
                if !pidl.is_null() {
                    let position = view.GetItemPosition(pidl);
                    CoTaskMemFree(Some(pidl.cast()));
                    if let Ok(mut point) = position {
                        if let Some(host) = crate::platform::desktop_host() {
                            let list =
                                FindWindowExW(host, None, windows::core::w!("SysListView32"), None)
                                    .unwrap_or(host);
                            let _ = ClientToScreen(list, &mut point);
                            let work = crate::platform::primary_work_area();
                            grid.origin_x =
                                ((point.x - work.left) as f32 / scale).rem_euclid(grid.cell_w);
                            grid.origin_y =
                                ((point.y - work.top) as f32 / scale).rem_euclid(grid.cell_h);
                        }
                    }
                }
            }
            Ok(())
        })();
        CoUninitialize();
        grid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metric_validation_rejects_missing_or_unreasonable_values() {
        for bad in [0., -1., f32::NAN, f32::INFINITY, 4096.] {
            assert_eq!(valid_cell(bad, 75.), 75.);
        }
        assert_eq!(valid_cell(96., 75.), 96.);
    }
}
