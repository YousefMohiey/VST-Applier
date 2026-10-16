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

/// Windows 11: ask DWM for rounded corners on the main window. The window
/// is undecorated (custom header), so without this it sits square among
/// all the rounded system windows. Self-guarding: only the first call
/// does work. No-op on Windows 10 (the attribute fails, harmlessly).
pub fn round_main_window_corners() {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let raw = MAIN_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        // No handle yet: retry next frame.
        DONE.store(false, Ordering::Relaxed);
        return;
    }
    use windows::Win32::Foundation::HWND;
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const core::ffi::c_void,
            std::mem::size_of_val(&pref) as u32,
        );
    }
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

/// Native folder picker (IFileOpenDialog + FOS_PICKFOLDERS), parented to
/// the main window. BLOCKING — call from a worker thread, never the UI
/// thread. Returns the chosen filesystem path, or None when cancelled.
pub fn pick_folder(initial: &str) -> Option<String> {
    use windows::core::{IUnknown, HSTRING};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, IBindCtx,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let picked = (|| -> Option<String> {
            let dlg: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None::<&IUnknown>, CLSCTX_INPROC_SERVER)
                    .ok()?;
            let mut opts = dlg.GetOptions().ok()?;
            opts |= FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
            dlg.SetOptions(opts).ok()?;
            let _ = dlg.SetTitle(&HSTRING::from("Choose the plugins folder"));
            if !initial.is_empty() {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                    &HSTRING::from(initial),
                    None::<&IBindCtx>,
                ) {
                    let _ = dlg.SetFolder(&item);
                }
            }
            let raw = MAIN_HWND.load(Ordering::Relaxed);
            let hwnd = HWND(raw as *mut core::ffi::c_void);
            if dlg.Show(hwnd).is_err() {
                return None; // cancelled (E_CANCELLED) or failed
            }
            let item = dlg.GetResult().ok()?;
            let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let s = p.to_string().ok()?;
            CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
            Some(s)
        })();
        CoUninitialize();
        picked
    }
}
