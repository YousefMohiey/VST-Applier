//! Minimal manual host: loads the plugin DLL and drives the exact call
//! sequence our VST2 host uses, with an SEH guard around the process
//! call so a fault is reported (code) instead of killing the process.
//! Trace lines from the plugin appear when ADN_TRACE=1.
//!
//!   ADN_TRACE=1 cargo run -p applier-denoise-vst2 --release --example manual_host -- <dll>

use std::ffi::c_void;

use windows::core::PCSTR;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

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

fn main() {
    let dll = std::env::args().nth(1).expect("usage: manual_host <dll>");
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

    xdisp(0, 0, 0, std::ptr::null_mut(), 0.0); // open
    xdisp(10, 0, 0, std::ptr::null_mut(), 44_100.0); // sample rate
    xdisp(11, 0, 512, std::ptr::null_mut(), 0.0); // block size
    xdisp(12, 0, 1, std::ptr::null_mut(), 0.0); // mains changed
    println!("open/setup ok");

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
    for block in 0..40 {
        let mut in_ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
        let mut out_ptrs = [ol.as_mut_ptr(), orr.as_mut_ptr()];
        let res = microseh::try_seh(|| unsafe {
            process(
                effect as *mut c_void,
                in_ptrs.as_mut_ptr(),
                out_ptrs.as_mut_ptr(),
                n as i32,
            )
        });
        match res {
            Ok(()) => {
                if block % 10 == 0 {
                    let rms = (ol.iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
                    println!("block {block}: ok, out rms {rms:.5}");
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
    println!("done, faults = {faults}");

    if std::env::var_os("ADN_EDITOR_HOLD").is_some() {
        let parent = unsafe {
            windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
                Default::default(),
                windows::core::w!("STATIC"),
                windows::core::w!("VSTApplierDenoise editor"),
                windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW,
                40,
                40,
                372,
                230,
                None,
                None,
                None,
                None,
            )
            .expect("parent")
        };
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                parent,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
        println!("EDITOR_PARENT={:x}", parent.0 as usize);
        let _ = xdisp(14, 0, 0, parent.0 as *mut c_void, 0.0); // effEditOpen
        println!("editor open — holding 25 s");
        for _ in 0..600 {
            xdisp(19, 0, 0, std::ptr::null_mut(), 0.0); // effEditIdle
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
                };
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = xdisp(15, 0, 0, std::ptr::null_mut(), 0.0); // effEditClose
    }
}
