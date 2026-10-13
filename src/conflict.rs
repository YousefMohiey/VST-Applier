//! Old-version mutual exclusion.
//!
//! Only one VST-Applier flavor can own the virtual cable: both the C# app
//! and this port run a session guard that mutes foreign renderers, so
//! running both means each one mutes the other within seconds. We detect
//! `VstApplier.exe` (any PID except our own) and politely ask its windows
//! to close (WM_CLOSE, so it saves its session) instead of fighting it.

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
};

const OLD_EXE: &str = "VstApplier.exe";

fn my_pid() -> u32 {
    unsafe { windows::Win32::System::Threading::GetCurrentProcessId() }
}

/// PIDs of running old-app processes, excluding ourselves.
pub fn old_app_pids() -> Vec<u32> {
    pids_of(OLD_EXE)
}

/// PIDs of running processes named `exe_name` (case-insensitive),
/// excluding ourselves.
pub fn pids_of(exe_name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        if snap.is_invalid() {
            return out;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                let end = pe
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe.szExeFile.len());
                let name = String::from_utf16_lossy(&pe.szExeFile[..end]);
                if name.eq_ignore_ascii_case(exe_name) && pe.th32ProcessID != my_pid() {
                    out.push(pe.th32ProcessID);
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
    }
    out
}

pub fn old_app_running() -> bool {
    !old_app_pids().is_empty()
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lp: LPARAM) -> BOOL {
    let pid = {
        let mut id = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut id as *mut u32)) };
        id
    };
    if pid == my_pid() {
        return TRUE;
    }
    // Re-open the target list through lParam: (pids, hits).
    let (wanted, hits) =
        unsafe { &mut *(lp.0 as *mut (Vec<u32>, Vec<*mut core::ffi::c_void>)) };
    if wanted.contains(&pid) {
        hits.push(hwnd.0);
    }
    TRUE
}

/// Post WM_CLOSE to every top-level window owned by the old app.
/// Returns the number of windows asked. The old app exits on its own terms
/// (saving its session); callers should re-check after a moment.
pub fn quit_old_app() -> usize {
    let wanted = old_app_pids();
    if wanted.is_empty() {
        return 0;
    }
    unsafe {
        let mut ctx: (Vec<u32>, Vec<*mut core::ffi::c_void>) = (wanted, Vec::new());
        let _ = EnumWindows(
            Some(enum_cb),
            LPARAM(&mut ctx as *mut (Vec<u32>, Vec<*mut core::ffi::c_void>) as isize),
        );
        let mut asked = 0;
        for raw in ctx.1 {
            if PostMessageW(HWND(raw), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok() {
                asked += 1;
            }
        }
        // No visible windows (tray-only app): nothing further to ask
        // politely here — the banner keeps pointing at the running process.
        asked
    }
}
