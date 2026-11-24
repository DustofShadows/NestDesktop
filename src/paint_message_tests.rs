//! Native regression for paint messages delivered inside a borrowed App call.
//! Fixtures stay hidden and are never attached to Explorer or the real Desktop.

use crate::{drain_deferred, paint_retry, wndproc, APP, DEFERRED, TILE};
use std::sync::atomic::{AtomicU64, Ordering};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::*,
        Graphics::Gdi::{GetUpdateRect, InvalidateRect},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);

struct HiddenWindow {
    hwnd: HWND,
    class: Vec<u16>,
    instance: HINSTANCE,
}

impl HiddenWindow {
    unsafe fn new() -> Self {
        let class: Vec<_> = format!(
            "NestDesktop.PaintMessageTest.{}.{}",
            std::process::id(),
            NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let instance: HINSTANCE = GetModuleHandleW(None).unwrap().into();
        assert_ne!(
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            }),
            0
        );
        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class.as_ptr()),
            w!("NestDesktop hidden paint regression"),
            WS_POPUP,
            0,
            0,
            104,
            114,
            None,
            None,
            instance,
            Some(TILE as *const _),
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                let _ = UnregisterClassW(PCWSTR(class.as_ptr()), instance);
                panic!("Could not create hidden regression window: {error}");
            }
        };
        let fixture = Self {
            hwnd,
            class,
            instance,
        };
        assert_eq!(GetWindowLongPtrW(hwnd, GWLP_USERDATA), TILE);
        assert!(!IsWindowVisible(hwnd).as_bool());
        fixture
    }

    unsafe fn invalidate_and_send_paint(&self) {
        let _ = InvalidateRect(self.hwnd, None, false);
        // Windows can auto-validate a hidden window's update region. Force
        // synchronous dispatch to test reentrancy without showing the fixture;
        // no assertion about retained hidden-window invalidation is possible.
        SendMessageW(self.hwnd, WM_PAINT, WPARAM(0), LPARAM(0));
        assert!(
            !GetUpdateRect(self.hwnd, None, false).as_bool(),
            "Busy paint handling must leave no native update region"
        );
    }
}

impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(PCWSTR(self.class.as_ptr()), self.instance);
        }
    }
}

struct QueueCleanup;

impl QueueCleanup {
    fn new() -> Self {
        APP.with(|app| assert!(app.borrow().is_none()));
        DEFERRED.with(|queue| assert!(queue.borrow().is_empty()));
        Self
    }
}

impl Drop for QueueCleanup {
    fn drop(&mut self) {
        // These fixtures enqueue only WM_PAINT, with no heap-owned payloads.
        DEFERRED.with(|queue| queue.borrow_mut().clear());
    }
}

#[test]
fn synchronous_busy_paints_are_coalesced_and_drained_after_release() {
    let _cleanup = QueueCleanup::new();
    unsafe {
        let window = HiddenWindow::new();
        APP.with(|app| {
            let _borrow = app.borrow_mut();
            window.invalidate_and_send_paint();
            window.invalidate_and_send_paint();
            DEFERRED.with(|queue| {
                let queue = queue.borrow();
                assert_eq!(queue.len(), 1, "Duplicate paint must not multiply work");
                assert_eq!(queue[0].0, window.hwnd);
                assert_eq!(queue[0].1, WM_PAINT);
            });
        });
        drain_deferred();
        DEFERRED.with(|queue| assert!(queue.borrow().is_empty()));
        assert!(!GetUpdateRect(window.hwnd, None, false).as_bool());
        assert!(!IsWindowVisible(window.hwnd).as_bool());
    }
}

#[test]
fn synchronous_paint_during_presentation_does_not_schedule_feedback() {
    let _cleanup = QueueCleanup::new();
    unsafe {
        let drawing = HiddenWindow::new();
        let other = HiddenWindow::new();
        {
            let _painting = paint_retry::Painting::begin(drawing.hwnd.0 as isize);
            APP.with(|app| {
                let _borrow = app.borrow_mut();
                drawing.invalidate_and_send_paint();
                drawing.invalidate_and_send_paint();
                DEFERRED.with(|queue| assert!(queue.borrow().is_empty()));
                // Suppression must be local to the surface being presented.
                other.invalidate_and_send_paint();
                DEFERRED.with(|queue| {
                    let queue = queue.borrow();
                    assert_eq!(queue.len(), 1);
                    assert_eq!(queue[0].0, other.hwnd);
                    assert_eq!(queue[0].1, WM_PAINT);
                });
            });
        }
        assert!(!paint_retry::is_rendering(drawing.hwnd.0 as isize));
        drain_deferred();
        DEFERRED.with(|queue| assert!(queue.borrow().is_empty()));
    }
}
