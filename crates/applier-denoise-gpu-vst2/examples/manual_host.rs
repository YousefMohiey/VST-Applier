//! Manual host for the GPU denoise plugin: loads the DLL, drives the exact
//! call sequence the app's VST2 host uses, and (optionally) opens the editor,
//! reports effEditGetRect and grabs the window into a BMP for inspection.
//!
//!   cargo run -p applier-denoise-gpu-vst2 --release --example manual_host -- \
//!       <dll> [--editor]
//!
//! Notes:
//! * `--editor` needs a desktop session (it shows a window for a moment).
//! * faults are caught with SEH so a crash is reported, not fatal.
//! * ADG_TRACE=1 makes the plugin print per-block counters to stderr.

use std::ffi::c_void;

use windows::core::PCSTR;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SelectObject, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PW_CLIENTONLY};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetWindow,
    PeekMessageW, RegisterClassW, ShowWindow, TranslateMessage, GW_CHILD, MSG, PM_REMOVE, SW_SHOW,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
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
#[derive(Default)]
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

unsafe extern "system" fn parent_proc(
    hwnd: HWND,
    msg: u32,
    wp: windows::Win32::Foundation::WPARAM,
    lp: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Save a 32-bit bottom-up BGRA buffer as a BMP.
fn write_bmp(path: &str, w: i32, h: i32, bgra: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    let size = 14 + 40 + bgra.len() as u32;
    f.write_all(b"BM")?;
    f.write_all(&size.to_le_bytes())?;
    f.write_all(&0u16.to_le_bytes())?;
    f.write_all(&0u16.to_le_bytes())?;
    f.write_all(&54u32.to_le_bytes())?;
    f.write_all(&40u32.to_le_bytes())?;
    f.write_all(&w.to_le_bytes())?;
    f.write_all(&h.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    f.write_all(&(w * h * 4).to_le_bytes())?;
    f.write_all(&2835i32.to_le_bytes())?;
    f.write_all(&2835i32.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    f.write_all(&0u32.to_le_bytes())?;
    f.write_all(bgra)?;
    Ok(())
}

/// Grab a window's client area into a BGRA buffer, bottom-up.
unsafe fn grab(hwnd: HWND, w: i32, h: i32, use_print: bool) -> Vec<u8> {
    let src = GetDC(hwnd);
    let dc = CreateCompatibleDC(src);
    let bmp = CreateCompatibleBitmap(src, w, h);
    let old = SelectObject(dc, bmp);

    if use_print {
        let _ = PrintWindow(hwnd, dc, PW_CLIENTONLY);
    } else {
        let _ = BitBlt(dc, 0, 0, w, h, src, 0, 0, SRCCOPY);
    }

    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let got = GetDIBits(
        dc,
        bmp,
        0,
        h as u32,
        Some(buf.as_mut_ptr().cast()),
        &mut bi,
        DIB_RGB_COLORS,
    );
    println!("   GetDIBits scanned {got} lines");

    SelectObject(dc, old);
    let _ = DeleteObject(HGDIOBJ(bmp.0));
    let _ = DeleteDC(dc);
    ReleaseDC(hwnd, src);
    buf
}

fn distinct_colors(bgra: &[u8]) -> usize {
    let mut set = std::collections::HashSet::new();
    for px in bgra.chunks_exact(4).step_by(37) {
        set.insert((px[0], px[1], px[2]));
        if set.len() > 64 {
            break;
        }
    }
    set.len()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dll = args.next().expect("usage: manual_host <dll> [--editor]");
    let want_editor = args.any(|a| a == "--editor");

    let module = unsafe { LoadLibraryA(PCSTR(format!("{dll}\0").as_ptr())) }.expect("LoadLibrary");
    let main_fn = unsafe { GetProcAddress(module, PCSTR(b"VSTPluginMain\0".as_ptr())) }
        .expect("VSTPluginMain");
    let main_fn: PluginMain = unsafe { std::mem::transmute(main_fn) };
    let effect = unsafe { main_fn(audio_master) } as *mut AEffectPrefix;
    println!("effect = {effect:p}");
    let disp = unsafe { (*effect).dispatcher.expect("dispatcher") };
    let xdisp = |op: i32, index: i32, value: isize, ptr: *mut c_void, opt: f32| unsafe {
        disp(effect as *mut c_void, op, index, value, ptr, opt)
    };

    let t0 = std::time::Instant::now();
    xdisp(0, 0, 0, std::ptr::null_mut(), 0.0); // open (loads ORT + models)
    println!("open ok in {} ms", t0.elapsed().as_millis());

    let mut name = [0u16; 64];
    xdisp(45, 0, 0, name.as_mut_ptr().cast(), 0.0);
    let n: usize = name.iter().position(|&c| c == 0).unwrap_or(0);
    println!("effect name = {}", String::from_utf16_lossy(&name[..n]));

    xdisp(10, 0, 0, std::ptr::null_mut(), 48_000.0); // sample rate
    xdisp(11, 0, 512, std::ptr::null_mut(), 0.0); // block size
    xdisp(12, 0, 1, std::ptr::null_mut(), 0.0); // mains changed

    let n = 512usize;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    let mut ol = vec![0.0f32; n];
    let mut orr = vec![0.0f32; n];
    let mut seed = 0x1234_5678u32;
    for v in l.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *v = (seed as f32 / u32::MAX as f32 - 0.5) * 0.12;
        r.fill(*v);
    }

    let process = unsafe { (*effect).process.expect("process") };
    let mut faults = 0;
    let mut worst_us = 0u128;
    let blocks = 60;
    for block in 0..blocks {
        let mut in_ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
        let mut out_ptrs = [ol.as_mut_ptr(), orr.as_mut_ptr()];
        let t = std::time::Instant::now();
        let res = microseh::try_seh(|| unsafe {
            process(
                effect as *mut c_void,
                in_ptrs.as_mut_ptr(),
                out_ptrs.as_mut_ptr(),
                n as i32,
            )
        });
        let us = t.elapsed().as_micros();
        worst_us = worst_us.max(us);
        match res {
            Ok(()) => {
                if block % 15 == 0 {
                    let rms = (ol.iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
                    println!("block {block:2}: ok, out rms {rms:.5}, {us} us");
                }
            }
            Err(c) => {
                faults += 1;
                println!("block {block}: FAULT {c:?}");
                if faults > 2 {
                    break;
                }
            }
        }
    }
    let rms = (ol.iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
    println!("done: {blocks} blocks, faults = {faults}, worst block = {worst_us} us, last rms {rms:.5}");
    assert!(ol.iter().all(|v| v.is_finite()), "non-finite output");
    assert_eq!(faults, 0, "process faults");

    if want_editor {
        unsafe {
            let class = windows::core::w!("VstApplierGpuHostParent");
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap();
            let _ = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(parent_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            });
            let parent = CreateWindowExW(
                Default::default(),
                class,
                windows::core::w!("Applier Noise Suppression GPU editor"),
                WS_OVERLAPPEDWINDOW,
                60,
                60,
                700,
                500,
                None,
                None,
                instance,
                None,
            )
            .expect("parent window");
            let _ = ShowWindow(parent, SW_SHOW);

            let mut rect_ptr: *mut ERect = std::ptr::null_mut();
            let ok = xdisp(
                13,
                0,
                0,
                (&mut rect_ptr as *mut *mut ERect).cast(),
                0.0,
            );
            if !rect_ptr.is_null() {
                let r = &*rect_ptr;
                println!(
                    "effEditGetRect: ok={ok} right={} bottom={} => {}x{}",
                    r.right,
                    r.bottom,
                    r.right - r.left,
                    r.bottom - r.top
                );
            } else {
                println!("effEditGetRect: no rect");
            }

            let _ = xdisp(14, 0, 0, parent.0 as *mut c_void, 0.0);
            println!("editor opened (child of parent)");

            // The editor is a CHILD window: FindWindowW won't see it.
            let editor = GetWindow(parent, GW_CHILD).expect("editor window");
            let mut rc = RECT::default();
            let _ = GetClientRect(editor, &mut rc);
            println!(
                "editor window client = {}x{}",
                rc.right - rc.left,
                rc.bottom - rc.top
            );

            // Let it paint, then grab it twice: PrintWindow (what the task
            // asks for) and a window-DC BitBlt as a fallback for custom paint.
            for _ in 0..20 {
                pump_messages();
                let _ = UpdateWindow(editor);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;
            let shot_print = grab(editor, w, h, true);
            let shot_dc = grab(editor, w, h, false);
            let (shot, how) = if distinct_colors(&shot_print) > 8 {
                (shot_print, "PrintWindow")
            } else {
                (shot_dc, "BitBlt(window DC)")
            };
            println!(
                "captured via {how}: {} distinct colours",
                distinct_colors(&shot)
            );
            let path = std::env::var("ADG_SHOT").unwrap_or_else(|_| "gpu_editor.bmp".to_string());
            if let Err(e) = write_bmp(&path, w, h, &shot) {
                println!("write_bmp failed: {e}");
            } else {
                println!("wrote {path} ({w}x{h})");
            }

            // Slider interaction smoke test: click the first track at 25 %.
            let _ = xdisp(24, 0, 0, std::ptr::null_mut(), 0.0); // effSetChunk no-op guard
            let gp = |i: i32| ((*effect).get_parameter.expect("get_parameter"))(effect as *mut c_void, i);
            let before = gp(0);
            let x = 24 + (616 - 24) / 4;
            let y = 100 + 7;
            let lp = ((y as isize) << 16) | (x as isize & 0xFFFF);
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP};
                SendMessageW(editor, WM_LBUTTONDOWN, windows::Win32::Foundation::WPARAM(1), LPARAM(lp));
                SendMessageW(editor, WM_LBUTTONUP, windows::Win32::Foundation::WPARAM(0), LPARAM(lp));
            }
            let after = gp(0);
            println!("slider click: attenuation {before:.0} -> {after:.0} dB");
            assert!((after - 25.0).abs() < 4.0, "slider hit test wrong: {after}");

            // Let the audio path run once with the new setting.
            let mut in_ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
            let mut out_ptrs = [ol.as_mut_ptr(), orr.as_mut_ptr()];
            unsafe {
                process(
                    effect as *mut c_void,
                    in_ptrs.as_mut_ptr(),
                    out_ptrs.as_mut_ptr(),
                    n as i32,
                )
            };
            println!("post-click block ok");

            let _ = xdisp(15, 0, 0, std::ptr::null_mut(), 0.0); // effEditClose
            let _ = DestroyWindow(parent);
        }
    }

    xdisp(1, 0, 0, std::ptr::null_mut(), 0.0); // close
    println!("closed");
}
