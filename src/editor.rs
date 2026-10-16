//! Native VST editor windows (mirrors the per-plugin editor Forms).
//!
//! Each editor is a plain Win32 overlapped window whose HWND is handed to
//! the native host (`open_editor`). VST2 editors get their preferred size
//! from the host and a periodic idle pump; VST3 editors open at a default
//! size. Windows are owned here; the host is told before destroy.

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Close requests from editor windows (X-click / Alt+F4): the app must
/// close the PLUGIN's editor before the window dies — destroying the
/// window under a live plugin editor corrupts the GL surface (the
/// black-window bug). The wnd_proc defers WM_CLOSE and records the
/// HWND here; the UI loop drains this list.
static CLOSE_REQ: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());

/// Drain deferred close requests (raw HWNDs, still open).
pub fn take_close_requests() -> Vec<isize> {
    CLOSE_REQ.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CLOSE {
        if let Ok(mut v) = CLOSE_REQ.lock() {
            v.push(hwnd.0 as isize);
        }
        return LRESULT(0); // handled: the app closes the editor properly
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn hwnd_of(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

/// Create + show an editor window. `width`/`height` are the desired
/// CLIENT size (what the plugin asked for); the frame is grown with
/// `AdjustWindowRectEx` so the content keeps its exact aspect instead
/// of being squeezed under the caption and borders.
/// Returns an opaque handle for [`is_open`] / [`close`].
pub fn open_editor_window(title: &str, width: i32, height: i32) -> windows::core::Result<isize> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        let hinstance = HINSTANCE(module.0);
        let cls = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: windows::core::w!("VstApplierEditor"),
            ..Default::default()
        };
        // Already-registered from a previous editor is fine.
        RegisterClassW(&cls);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX;
        let mut rect = RECT { left: 0, top: 0, right: width, bottom: height };
        AdjustWindowRectEx(&mut rect, style, windows::Win32::Foundation::BOOL(0), WINDOW_EX_STYLE(0))?;
        let title_w: Vec<u16> = title.encode_utf16().chain([0]).collect();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("VstApplierEditor"),
            windows::core::PCWSTR(title_w.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            hinstance,
            None,
        )?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(hwnd.0 as isize)
    }
}

/// True while the user hasn't closed the window.
pub fn is_open(raw: isize) -> bool {
    unsafe { IsWindow(hwnd_of(raw)).as_bool() }
}

/// Simulate the user clicking the window's X: posts WM_CLOSE (tests
/// only). The window is destroyed by the default processing unless a
/// close interceptor is in place.
pub fn request_close(raw: isize) {
    unsafe {
        let _ = PostMessageW(hwnd_of(raw), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Destroy the window. Call the host's `close_editor` first.
pub fn close(raw: isize) {
    unsafe {
        let _ = DestroyWindow(hwnd_of(raw));
    }
}

/// Raw HWND value for the host's `open_editor`. Valid while [`is_open`].
///
/// # Safety
/// Only pass handles returned by [`open_editor_window`] that are still open.
pub unsafe fn hwnd_raw(raw: isize) -> *mut core::ffi::c_void {
    raw as *mut core::ffi::c_void
}
