//! End-to-end check of the VST-Applier AI Denoise VST2 plugin through
//! our own host: load at 44.1 k (exercises the internal resampler),
//! process noise (expect heavy gating) and silence, save state, size
//! the editor, open it on a real parent window, idle, close, and keep
//! processing.

use std::ffi::c_void;

use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, TranslateMessage,
    MSG, PM_REMOVE, WS_OVERLAPPED,
};

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn main() {
    let root = std::path::PathBuf::from(r"C:\Users\Administrator\vst-applier-rs\target\release");
    let host_dll = root.join("Vst2HostNative.dll");
    let plugin = root.join("VSTApplierDenoise.dll");
    println!("host:   {}", host_dll.display());
    println!("plugin: {}", plugin.display());

    let mut host = vst_host_sys::PluginHost::load(
        &host_dll,
        vst_host_sys::PluginFormat::Vst2,
        &plugin,
    )
    .expect("load plugin");
    println!("loaded OK");
    host.setup_processing(44_100.0, 512, 2).expect("setup");
    println!("setup OK (44.1 kHz — internal resampler path)");

    // Noise: steady white noise should be gated hard.
    let mut seed = 0x1234_5678u32;
    let mut noise_in = vec![0.0f32; 512 * 2];
    for v in noise_in.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *v = (seed as f32 / u32::MAX as f32 - 0.5) * 0.12;
    }
    let mut out = vec![0.0f32; 512 * 2];
    let mut in_acc = Vec::new();
    let mut out_acc = Vec::new();
    for _ in 0..80 {
        host.process_f32(&noise_in, &mut out).expect("process noise");
        in_acc.extend_from_slice(&noise_in);
        out_acc.extend_from_slice(&out);
    }
    let tail = (in_acc.len() / 2).min(88_200); // last chunk only
    let (ni, no) = (
        rms(&in_acc[in_acc.len() - tail..]),
        rms(&out_acc[out_acc.len() - tail..]),
    );
    let atten_db = 20.0 * (ni / no.max(1e-9)).log10();
    println!(
        "noise: in rms {ni:.4} -> out rms {no:.4} ({atten_db:.1} dB)"
    );
    assert!(atten_db > 3.0, "noise was not gated (only {atten_db:.1} dB)");

    // Silence stays silence, no NaN.
    let silent = vec![0.0f32; 512 * 2];
    host.process_f32(&silent, &mut out).expect("process silence");
    assert!(out.iter().all(|v| v.is_finite()));
    println!("silence OK (finite)");

    // State: the 16-byte params chunk.
    match host.save_state().expect("save_state") {
        Some(blob) => println!("state: {} bytes", blob.len()),
        None => println!("state: none"),
    }

    println!("editor size: {:?}", host.editor_size());

    // Editor open/close on a real parent window.
    unsafe {
        let parent = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("denoise editor test"),
            WS_OVERLAPPED,
            0,
            0,
            520,
            320,
            None,
            None,
            None,
            None,
        )
        .expect("parent window");
        host.open_editor(parent.0 as *mut c_void).expect("open editor");
        println!("editor opened OK");
        for _ in 0..20 {
            host.editor_idle();
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        host.close_editor().expect("close editor");
        println!("editor closed OK");
        let _ = DestroyWindow(parent);
        let _ = HWND::default();
    }

    // Still processing after the editor round-trip.
    host.process_f32(&noise_in, &mut out).expect("process after editor");
    assert!(out.iter().all(|v| v.is_finite()));
    println!("ALL OK");
}
