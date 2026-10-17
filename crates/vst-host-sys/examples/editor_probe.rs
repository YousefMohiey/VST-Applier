//! Editor open / close / drop TIMING probe: measures the three UI-thread
//! operations the app performs around adding, editing and removing a
//! plugin — used to pin down "not responding" reports on a specific
//! plugin call.
//!
//! Usage (from the repo root):
//!   cargo run -q -p vst-host-sys --example editor_probe -- "<plugin path>" vst2|vst3
//!
//! The parent window is NEVER shown, so editor windows do not flash on
//! the user's screen.

use std::time::Instant;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let plugin = args
        .next()
        .expect("usage: editor_probe <plugin path> vst2|vst3");
    let fmt = match args.next().as_deref() {
        Some("vst2") => vst_host_sys::PluginFormat::Vst2,
        _ => vst_host_sys::PluginFormat::Vst3,
    };
    let exe = std::path::PathBuf::from("target/release");
    let host_dll = fmt.dll_name();

    println!("plugin  : {plugin}");
    let mut host = match vst_host_sys::PluginHost::load(
        &exe.join(host_dll),
        fmt,
        std::path::Path::new(&plugin),
    ) {
        Ok(h) => h,
        Err(e) => {
            println!("LOAD FAILED: {e}");
            std::process::exit(2);
        }
    };
    println!("load    : OK");
    match host.setup_processing(48_000.0, 512, 2) {
        Ok(()) => println!("setup   : OK (channels {})", host.channels),
        Err(e) => println!("setup   : FAILED -> {e}"),
    }

    // State harvest: the app calls save_state on EVERY slot for every
    // chain edit (checkpoint) and every autosave. A plugin whose state
    // is huge (sampler sample banks) or slow (web-UI marshaling) hangs
    // the UI right here.
    for round in 0..3 {
        let t = Instant::now();
        match host.save_state() {
            Ok(Some(blob)) => println!(
                "SAVEST  : round {round} = {} ms, {} bytes",
                t.elapsed().as_millis(),
                blob.len()
            ),
            Ok(None) => println!("SAVEST  : round {round} = {} ms, no state", t.elapsed().as_millis()),
            Err(e) => println!("SAVEST  : round {round} FAILED -> {e}"),
        }
    }

    unsafe {
        let module = GetModuleHandleW(None).unwrap();
        let cls = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: windows::core::w!("ProbeEditor"),
            ..Default::default()
        };
        RegisterClassW(&cls);
        // Never shown: the editor opens as an invisible child.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("ProbeEditor"),
            windows::core::w!("probe"),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            700,
            450,
            None,
            None,
            HINSTANCE(module.0),
            None,
        )
        .unwrap();

        let t = Instant::now();
        match host.open_editor(hwnd.0 as *mut _) {
            Ok(()) => println!("OPEN    : {} ms", t.elapsed().as_millis()),
            Err(e) => println!("OPEN    : FAILED -> {e}"),
        }

        // Let a CEF-style editor boot, like a user would before closing.
        std::thread::sleep(std::time::Duration::from_millis(1500));

        let t = Instant::now();
        let r = host.close_editor();
        println!("CLOSE   : {} ms ({r:?})", t.elapsed().as_millis());

        let t = Instant::now();
        drop(host);
        println!("DROP    : {} ms", t.elapsed().as_millis());

        let _ = DestroyWindow(hwnd);
    }
    println!("PROBE DONE");
}
