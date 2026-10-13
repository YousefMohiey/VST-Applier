use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn main() {
    let exe = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\debug");
    let cases: &[(&str, vst_host_sys::PluginFormat, &str)] = &[
        (
            "Vst2HostNative.dll",
            vst_host_sys::PluginFormat::Vst2,
            r"C:\Users\Administrator\AppData\Local\hermes\cache\scratch\VST-Applier\defaults\Plugins\ReaPlugs\reaeq-standalone.dll",
        ),
        (
            "Vst3HostNative.dll",
            vst_host_sys::PluginFormat::Vst3,
            r"C:\Users\Administrator\AppData\Local\hermes\cache\scratch\VST-Applier\defaults\Plugins\rnnoise\rnnoise.vst3",
        ),
    ];
    for (dll, fmt, plugin) in cases {
        println!("=== {dll} + {plugin} ===");
        let mut host =
            vst_host_sys::PluginHost::load(&exe.join(dll), *fmt, std::path::Path::new(plugin))
                .expect("load plugin");
        println!("loaded OK");
        host.setup_processing(48000.0, 512, 2).expect("setup");
        println!("size: {:?}", host.editor_size());
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
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("ProbeEditor"),
                windows::core::w!("probe"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                520,
                420,
                None,
                None,
                HINSTANCE(module.0),
                None,
            )
            .unwrap();
            let _ = ShowWindow(hwnd, SW_SHOW);
            match host.open_editor(hwnd.0 as *mut _) {
                Ok(()) => println!("open_editor OK"),
                Err(e) => println!("open_editor FAILED: {e} | {} ", host.last_error()),
            }
            host.editor_idle();
            println!("close_editor: {:?}", host.close_editor());
            let _ = DestroyWindow(hwnd);
        }
    }
    println!("PROBE DONE");
}
