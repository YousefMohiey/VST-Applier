//! Win32 window reveal — the deterministic wake for tray and
//! second-instance actions.
//!
//! Root cause of the flaky tray: while the window is hidden
//! (`ViewportCommand::Visible(false)`), Windows delivers no paint work,
//! and `Context::request_repaint()` only schedules a redraw request that
//! a hidden window may never receive — so a queued tray action could sit
//! unprocessed until some unrelated event woke the loop ("sometimes it
//! works"). Showing the window at the OS level always works: the window
//! becomes visible, the OS paints it, eframe runs `update()`, and the
//! queued action is drained there.

use std::sync::atomic::{AtomicIsize, Ordering};

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// Cache the main window handle (refreshed each frame from eframe's
/// `HasWindowHandle`; a single atomic store, negligible cost).
pub fn set_main_hwnd(raw: isize) {
    MAIN_HWND.store(raw, Ordering::Relaxed);
}

/// Un-minimize + show + focus the main window. Safe from any thread.
/// Returns true when a live window handle was known.
pub fn reveal_main_window() -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };
    let raw = MAIN_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return false;
    }
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return false;
        }
        // SW_RESTORE un-minimizes and shows in one step; the extra
        // SW_SHOW also covers hidden-but-not-minimized state.
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    true
}
