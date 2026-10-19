//! Editor screenshot harness for the bundled VST2 panels.
//!
//! Loads a plugin DLL, asks it for its editor rect, hosts the editor in a
//! throwaway parent window of exactly that size, and captures the child
//! window with PrintWindow into a 32-bit BMP. The BMP is then converted
//! to PNG (the repo uses Pillow for that).
//!
//! The harness makes itself Per-Monitor-V2 aware first, so on a scaled
//! display it sees the same physical size the plugin reports and the
//! capture is 1:1 native pixels (no virtualisation blur).
//!
//!   cargo run -p applier-compressor-vst2 --release --example editor_shot -- \
//!       target/release/VSTApplierCompressor.dll scratch/compressor_editor.bmp

use std::ffi::c_void;
use std::io::Write;

use windows::core::{w, PCSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, ReleaseDC, SelectObject,
    UpdateWindow, BITMAPINFO, BI_RGB, DIB_RGB_COLORS, HDC,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExA, LOAD_WITH_ALTERED_SEARCH_PATH,
};
use windows::Win32::UI::HiDpi::{
    GetAwarenessFromDpiAwarenessContext, GetDpiForWindow, GetThreadDpiAwarenessContext,
    GetWindowDpiAwarenessContext, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetWindow, PeekMessageW, ShowWindow, TranslateMessage,
    GW_CHILD, MSG, PM_REMOVE, SW_SHOW, WM_QUIT, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

#[repr(C)]
struct AEffectPrefix {
    magic: i32,
    dispatcher: Option<
        unsafe extern "system" fn(*mut c_void, i32, i32, isize, *mut c_void, f32) -> isize,
    >,
    process: Option<unsafe extern "system" fn(*mut c_void, *mut *mut f32, *mut *mut f32, i32)>,
    set_parameter: Option<unsafe extern "system" fn(*mut c_void, i32, f32)>,
    get_parameter: Option<unsafe extern "system" fn(*mut c_void, i32) -> f32>,
}

#[repr(C)]
struct ERect {
    top: i16,
    left: i16,
    bottom: i16,
    right: i16,
}

type PluginMain = unsafe extern "system" fn(
    unsafe extern "system" fn(*mut c_void, i32, i32, isize, *mut c_void, f32) -> isize,
) -> *mut c_void;

unsafe extern "system" fn audio_master(
    _e: *mut c_void,
    _op: i32,
    _i: i32,
    _v: isize,
    _p: *mut c_void,
    _o: f32,
) -> isize {
    0
}

// windows-0.58 does not project PrintWindow; user32 is the same DLL the
// `windows` crate already links against.
unsafe extern "system" {
    fn PrintWindow(hwnd: isize, hdc: isize, flags: u32) -> i32;
}

fn pump(duration_ms: u64) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(duration_ms) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn write_bmp(path: &str, w: i32, h: i32, bgra: &[u8]) -> std::io::Result<()> {
    let image_size = (w * h * 4) as u32;
    let mut out = Vec::with_capacity(54 + image_size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + image_size).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    out.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes()); // positive -> bottom-up rows
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(bgra);
    let mut f = std::fs::File::create(path)?;
    f.write_all(&out)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dll = args.get(1).expect("usage: editor_shot <dll> <out.bmp>");
    let out = args.get(2).expect("usage: editor_shot <dll> <out.bmp>");

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // LOAD_WITH_ALTERED_SEARCH_PATH: resolve the plugin's own sibling
    // DLLs from its folder, not from this harness's target/debug dir.
    let module = unsafe {
        LoadLibraryExA(
            PCSTR(format!("{dll}\0").as_ptr()),
            None,
            LOAD_WITH_ALTERED_SEARCH_PATH,
        )
    }
    .expect("LoadLibraryEx");
    let main_fn = unsafe { GetProcAddress(module, PCSTR(b"VSTPluginMain\0".as_ptr())) }
        .expect("VSTPluginMain");
    let main_fn: PluginMain = unsafe { std::mem::transmute(main_fn) };
    let effect = unsafe { main_fn(audio_master) } as *mut AEffectPrefix;
    assert!(!effect.is_null(), "VSTPluginMain returned null");
    let disp = unsafe { (*effect).dispatcher.expect("dispatcher") };
    let xdisp = |op: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32| unsafe {
        disp(effect as *mut c_void, op, index, value, ptr, opt)
    };

    xdisp(0, 0, 0, std::ptr::null_mut(), 0.0); // open
    xdisp(10, 0, 0, std::ptr::null_mut(), 44_100.0); // sample rate
    xdisp(11, 0, 512, std::ptr::null_mut(), 0.0); // block size

    let mut rect_ptr: *mut ERect = std::ptr::null_mut();
    xdisp(13, 0, 0, &mut rect_ptr as *mut _ as *mut c_void, 0.0);
    let (ew, eh) = if rect_ptr.is_null() {
        (640, 400)
    } else {
        unsafe { ((*rect_ptr).right as i32, (*rect_ptr).bottom as i32) }
    };
    println!("editor rect: {ew}x{eh}");

    let parent = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("editor shot"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            40,
            40,
            ew + 40,
            eh + 70,
            None,
            None,
            None,
            None,
        )
        .expect("parent window")
    };
    unsafe {
        let _ = ShowWindow(parent, SW_SHOW);
        let _ = UpdateWindow(parent);
    }
    let _ = xdisp(14, 0, 0, parent.0 as *mut c_void, 0.0); // effEditOpen
    pump(900); // let the panel lay out, paint and start its meter timer

    let child = unsafe { GetWindow(parent, GW_CHILD) }.expect("editor child window");
    let mut crect = RECT::default();
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClientRect(child, &mut crect) };
    let (cw, ch) = (crect.right, crect.bottom);
    let dpi = unsafe { GetDpiForWindow(child) };
    let awareness = unsafe {
        let ctx = GetThreadDpiAwarenessContext();
        GetAwarenessFromDpiAwarenessContext(ctx)
    };
    let win_awareness = unsafe {
        let ctx = GetWindowDpiAwarenessContext(child);
        GetAwarenessFromDpiAwarenessContext(ctx)
    };
    println!("child client: {cw}x{ch}");
    println!("GetDpiForWindow: {dpi}");
    println!("thread awareness: {}", awareness.0);
    println!("editor window awareness: {}", win_awareness.0);

    let wnd_dc: HDC = unsafe { windows::Win32::Graphics::Gdi::GetDC(child) };
    let mem = unsafe { CreateCompatibleDC(wnd_dc) };
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>()
        as u32;
    bmi.bmiHeader.biWidth = cw;
    bmi.bmiHeader.biHeight = ch; // bottom-up
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;
    let mut bits: *mut c_void = std::ptr::null_mut();
    let dib = unsafe {
        CreateDIBSection(wnd_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).expect("dib section")
    };
    let old = unsafe { SelectObject(mem, dib) };
    let ok = unsafe { PrintWindow(child.0 as isize, mem.0 as isize, 0) };
    println!("PrintWindow -> {ok}");

    if bits.is_null() {
        panic!("no DIB bits");
    }
    let px = unsafe { std::slice::from_raw_parts(bits as *const u8, (cw * ch * 4) as usize) };

    // Sanity: PrintWindow of a dead/blank window returns a flat buffer.
    let mut ink = 0usize;
    for p in px.chunks_exact(4) {
        if p[0] > 40 || p[1] > 40 || p[2] > 40 {
            ink += 1;
        }
    }
    let ratio = ink as f64 / (cw * ch) as f64;
    println!("ink coverage: {:.1}%", ratio * 100.0);

    write_bmp(out, cw, ch, px).expect("write bmp");
    println!("wrote {out} ({cw}x{ch})");

    unsafe {
        SelectObject(mem, old);
        let _ = DeleteObject(dib);
        let _ = DeleteDC(mem);
        ReleaseDC(child, wnd_dc);
    }
    // DPI-change path: a synthetic 120-DPI WM_DPICHANGED must resize the
    // panel to the native 800x500 (the paint path then re-queries the
    // window DPI, which on this 96-DPI session stays 96).
    unsafe {
        use windows::Win32::Foundation::{LPARAM as LP, WPARAM as WP};
        use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_DPICHANGED};
        let mut suggested = RECT {
            left: 0,
            top: 0,
            right: 800,
            bottom: 500,
        };
        let dpi_wp = ((120u32) | (120u32 << 16)) as usize;
        let _ = SendMessageW(
            child,
            WM_DPICHANGED,
            WP(dpi_wp),
            LP(&mut suggested as *mut _ as isize),
        );
        let mut after = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(child, &mut after);
        println!("after WM_DPICHANGED(120): {}x{}", after.right, after.bottom);
        assert_eq!(
            (after.right, after.bottom),
            (800, 500),
            "WM_DPICHANGED resize"
        );
    }

    let _ = xdisp(15, 0, 0, std::ptr::null_mut(), 0.0); // effEditClose
    let _ = xdisp(1, 0, 0, std::ptr::null_mut(), 0.0); // effClose
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(parent);
    }
}
