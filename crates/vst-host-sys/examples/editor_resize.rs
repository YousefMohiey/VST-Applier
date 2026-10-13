//! Proves the sizeWindow fix: opens the ReaEQ editor, pumps messages,
//! then measures the parent CLIENT rect — it must match/exceed the
//! plugin's reported size (pre-open 400x300 is a placeholder).
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn main() {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let mut host = vst_host_sys::PluginHost::load(
        &exe.join("Vst2HostNative.dll"),
        vst_host_sys::PluginFormat::Vst2,
        std::path::Path::new(
            r"C:\Program Files\VST-Applier\Plugins\ReaPlugs\reaeq-standalone.dll",
        ),
    )
    .expect("load");
    host.setup_processing(48000.0, 512, 2).expect("setup");
    let pre = host.editor_size();
    println!("pre-open size: {pre:?}");
    unsafe {
        let module = GetModuleHandleW(None).unwrap();
        let cls = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: windows::core::w!("ProbeResize"),
            ..Default::default()
        };
        RegisterClassW(&cls);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            windows::core::w!("ProbeResize"),
            windows::core::w!("resize-probe"),
            WS_OVERLAPPEDWINDOW,
            100, 100, 520, 420,
            None, None,
            HINSTANCE(module.0),
            None,
        )
        .unwrap();
        let _ = ShowWindow(hwnd, SW_SHOW);
        host.open_editor(hwnd.0 as *mut _).expect("open");
        // Pump messages ~2s so any deferred sizeWindow lands.
        let t0 = std::time::Instant::now();
        let mut msg = MSG::default();
        while t0.elapsed() < std::time::Duration::from_secs(2) {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            host.editor_idle();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        println!("post-open reported size: {:?}", host.editor_size());
        let mut r = RECT::default();
        GetClientRect(hwnd, &mut r).unwrap();
        println!("parent CLIENT: {}x{}", r.right - r.left, r.bottom - r.top);
        host.close_editor();
        let _ = DestroyWindow(hwnd);
    }
    println!("RESIZE-PROBE DONE");
}
