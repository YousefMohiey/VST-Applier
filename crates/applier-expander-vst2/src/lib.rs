//! Applier Expander — a VST2 downward expander for the VST-Applier chain.
//!
//! The editor mirrors the reference "Expander" panel the user asked for:
//! Threshold, Ratio, Attack, Release and Output Gain sliders with dB/ratio
//! readout boxes and up/down steppers, drawn in the app's palette with the
//! standard credit footer. Signal path: RMS-ish (dB-domain, one-pole) level
//! detector; below the threshold the signal is attenuated by
//! (env - threshold) * (ratio - 1) dB; zero latency.
//!
//! ABI notes: field-for-field the C `AEffect`; the host (our Rust VST2
//! host DLL) uses the same subset as for the other bundled plugins.

#![allow(unsafe_op_in_unsafe_fn)]

mod editor;
mod ui_kit;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::HWND;

// ---------------------------------------------------------------------------
// VST2 ABI
// ---------------------------------------------------------------------------

const K_EFFECT_MAGIC: i32 = 0x5673_7450; // 'VstP'

const EFF_OPEN: i32 = 0;
const EFF_CLOSE: i32 = 1;
const EFF_SET_SAMPLE_RATE: i32 = 10;
const EFF_SET_BLOCK_SIZE: i32 = 11;
const EFF_MAINS_CHANGED: i32 = 12;
const EFF_EDIT_GET_RECT: i32 = 13;
const EFF_EDIT_OPEN: i32 = 14;
const EFF_EDIT_CLOSE: i32 = 15;
const EFF_EDIT_IDLE: i32 = 19;
const EFF_GET_CHUNK: i32 = 23;
const EFF_SET_CHUNK: i32 = 24;
const EFF_GET_EFFECT_NAME: i32 = 45;
const EFF_GET_VENDOR_STRING: i32 = 47;
const EFF_GET_PRODUCT_STRING: i32 = 48;

const EFF_FLAGS_HAS_EDITOR: i32 = 1 << 0;
const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;

type AudioMasterCallback =
    unsafe extern "system" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;

#[repr(C)]
struct AEffect {
    magic: i32,
    dispatcher: Option<
        unsafe extern "system" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize,
    >,
    process: Option<unsafe extern "system" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32)>,
    set_parameter: Option<unsafe extern "system" fn(*mut AEffect, i32, f32)>,
    get_parameter: Option<unsafe extern "system" fn(*mut AEffect, i32) -> f32>,
    num_programs: i32,
    num_params: i32,
    num_inputs: i32,
    num_outputs: i32,
    flags: i32,
    reserved1: isize,
    reserved2: isize,
    initial_delay: i32,
    real_qualities: i32,
    off_qualities: i32,
    io_ratio: f32,
    object: *mut c_void,
    user: *mut c_void,
    unique_id: i32,
    version: i32,
    process_replacing:
        Option<unsafe extern "system" fn(*mut AEffect, *mut *mut f32, *mut *mut f32, i32)>,
    process_double_replacing:
        Option<unsafe extern "system" fn(*mut AEffect, *mut *mut f64, *mut *mut f64, i32)>,
    future: [u8; 56],
}

#[repr(C)]
struct ERect {
    top: i16,
    left: i16,
    bottom: i16,
    right: i16,
}

// ---------------------------------------------------------------------------
// Parameters — High / Mid / Low, ±12 dB, laid out like the reference panel.
// ---------------------------------------------------------------------------

const P_THRESHOLD: usize = 0;
const P_RATIO: usize = 1;
const P_ATTACK: usize = 2;
const P_RELEASE: usize = 3;
const P_OUT: usize = 4;
const NUM_PARAMS: usize = 5;

const PARAMS: [(&str, f32, f32); NUM_PARAMS] = [
    ("Threshold", -80.0, 0.0),
    ("Ratio", 1.0, 10.0),
    ("Attack", 0.5, 100.0),
    ("Release", 10.0, 1000.0),
    ("Output Gain", -24.0, 24.0),
];

// ---------------------------------------------------------------------------
// Biquads (RBJ cookbook, transposed direct form II), zero latency.
// ---------------------------------------------------------------------------

// (No filter state: the detector below is the whole DSP.)

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// `repr(C)` is load-bearing: the ABI passes `*mut AEffect` and both
/// sides cast the SAME address back to `*mut Plugin`, which is only
/// sound while `effect` sits at offset 0.
#[repr(C)]
struct Plugin {
    effect: AEffect,
    sample_rate: f32,
    params: [AtomicU32; NUM_PARAMS],
    // audio-thread only
    /// Detector state, dB domain, one per channel.
    env_db: [f32; 2],
    chunk: Vec<u8>,
    editor: Option<HWND>,
    rect: ERect,
}

impl Plugin {
    fn new() -> Self {
        let effect = AEffect {
            magic: K_EFFECT_MAGIC,
            dispatcher: Some(dispatcher),
            process: Some(process_replacing),
            set_parameter: Some(set_parameter),
            get_parameter: Some(get_parameter),
            num_programs: 0,
            num_params: NUM_PARAMS as i32,
            num_inputs: 2,
            num_outputs: 2,
            flags: EFF_FLAGS_HAS_EDITOR | EFF_FLAGS_CAN_REPLACING,
            reserved1: 0,
            reserved2: 0,
            initial_delay: 0, // biquads are zero-latency
            real_qualities: 0,
            off_qualities: 0,
            io_ratio: 0.0,
            object: std::ptr::null_mut(),
            user: std::ptr::null_mut(),
            unique_id: 0x4145_5831, // 'AEX1'
            version: 1,
            process_replacing: Some(process_replacing),
            process_double_replacing: None,
            future: [0; 56],
        };
        Self {
            effect,
            sample_rate: 48_000.0,
            params: [
                AtomicU32::new((-50.0f32).to_bits()),
                AtomicU32::new(3.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(100.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            env_db: [-120.0; 2],
            chunk: Vec::new(),
            editor: None,
            rect: ERect {
                top: 0,
                left: 0,
                bottom: ui_kit::BASE_H as i16,
                right: ui_kit::BASE_W as i16,
            },
        }
    }

    fn param(&self, i: usize) -> f32 {
        f32::from_bits(self.params[i].load(Ordering::Relaxed))
    }

    fn set_param(&self, i: usize, v: f32) {
        let (_, lo, hi) = PARAMS[i];
        self.params[i].store(v.clamp(lo, hi).to_bits(), Ordering::Relaxed);
    }

    fn serialize(&mut self) {
        self.chunk.clear();
        self.chunk.extend_from_slice(b"AEX1");
        for i in 0..NUM_PARAMS {
            self.chunk.extend_from_slice(&self.param(i).to_le_bytes());
        }
    }

    fn deserialize(&mut self, bytes: &[u8]) {
        if bytes.len() >= 4 + 4 * NUM_PARAMS && &bytes[..4] == b"AEX1" {
            for i in 0..NUM_PARAMS {
                let o = 4 + 4 * i;
                let v = f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                self.set_param(i, v);
            }
        }
    }

    unsafe fn process(&mut self, inputs: *mut *mut f32, outputs: *mut *mut f32, frames: i32) {
        if frames <= 0 || inputs.is_null() || outputs.is_null() {
            return;
        }
        let n = frames as usize;

        // Knobs + coefficients are (re)computed per block: cheap, and the
        // detector state is the only thing carried across blocks.
        let thr = self.param(P_THRESHOLD);
        let slope = (self.param(P_RATIO) - 1.0).max(0.0);
        let fs = self.sample_rate.max(8_000.0);
        let a_coef = (-1.0f32 / (self.param(P_ATTACK) * 0.001 * fs)).exp();
        let r_coef = (-1.0f32 / (self.param(P_RELEASE) * 0.001 * fs)).exp();
        let out_lin = 10f32.powf(self.param(P_OUT) / 20.0);

        for ch in 0..2 {
            let inp = *inputs.add(ch);
            let outp = *outputs.add(ch);
            if inp.is_null() || outp.is_null() {
                continue;
            }
            let mut env = self.env_db[ch];
            for i in 0..n {
                let x = *inp.add(i);
                let x = if x.is_finite() { x } else { 0.0 };
                // dB-domain level detector: attack when rising, release
                // when falling (one-pole).
                let level_db = 20.0 * x.abs().max(1e-9).log10();
                let coef = if level_db > env { a_coef } else { r_coef };
                env = level_db + (env - level_db) * coef;
                // Below the threshold, attenuate by (env - thr) * slope —
                // the standard downward-expander curve (slope 0 above).
                let gain_db = if env < thr { (env - thr) * slope } else { 0.0 };
                let y = x * 10f32.powf(gain_db / 20.0) * out_lin;
                *outp.add(i) = if y.is_finite() { y } else { 0.0 };
            }
            self.env_db[ch] = env;
        }
    }

    unsafe fn open_editor(&mut self, parent: HWND) {
        if self.editor.is_some() {
            return;
        }
        if let Some(hwnd) = editor::create(parent, self as *mut Plugin) {
            self.editor = Some(hwnd);
        }
    }

    fn close_editor(&mut self) {
        if let Some(hwnd) = self.editor.take() {
            unsafe { editor::close(hwnd) };
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        self.close_editor();
    }
}

// ---------------------------------------------------------------------------
// Dispatcher / process entry points
// ---------------------------------------------------------------------------

unsafe fn write_vst_string(ptr: *mut c_void, s: &str, max: usize) {
    if ptr.is_null() {
        return;
    }
    let out = std::slice::from_raw_parts_mut(ptr as *mut u16, max);
    let mut i = 0;
    for c in s.encode_utf16() {
        if i >= max - 1 {
            break;
        }
        out[i] = c;
        i += 1;
    }
    out[i] = 0;
}

unsafe extern "system" fn dispatcher(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    _opt: f32,
) -> isize {
    if effect.is_null() {
        return 0;
    }
    let p = &mut *(effect as *mut Plugin);
    match opcode {
        EFF_OPEN => 1,
        EFF_CLOSE => {
            drop(Box::from_raw(effect as *mut Plugin));
            0
        }
        EFF_SET_SAMPLE_RATE => {
            if _opt > 0.0 {
                p.sample_rate = _opt;
            }
            1
        }
        EFF_SET_BLOCK_SIZE => 1,
        EFF_MAINS_CHANGED => 1,
        EFF_EDIT_GET_RECT => {
            if !ptr.is_null() {
                editor::sync_rect(p);
                *(ptr as *mut *mut ERect) = &mut p.rect;
            }
            1
        }
        EFF_EDIT_OPEN => {
            p.open_editor(HWND(ptr));
            1
        }
        EFF_EDIT_CLOSE => {
            p.close_editor();
            1
        }
        EFF_EDIT_IDLE => 0,
        EFF_GET_CHUNK => {
            if index != 0 || ptr.is_null() {
                return 0;
            }
            p.serialize();
            *(ptr as *mut *mut c_void) = p.chunk.as_mut_ptr() as *mut c_void;
            p.chunk.len() as isize
        }
        EFF_SET_CHUNK => {
            if index != 0 || ptr.is_null() || value <= 0 {
                return 0;
            }
            let bytes = std::slice::from_raw_parts(ptr as *const u8, value as usize);
            p.deserialize(bytes);
            1
        }
        EFF_GET_EFFECT_NAME => {
            write_vst_string(ptr, "Applier Expander", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "Applier", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "Applier Expander", 64);
            1
        }
        _ => 0,
    }
}

unsafe extern "system" fn process_replacing(
    effect: *mut AEffect,
    inputs: *mut *mut f32,
    outputs: *mut *mut f32,
    frames: i32,
) {
    if effect.is_null() {
        return;
    }
    let p = &mut *(effect as *mut Plugin);
    p.process(inputs, outputs, frames);
}

unsafe extern "system" fn set_parameter(effect: *mut AEffect, index: i32, value: f32) {
    if effect.is_null() || index < 0 || index as usize >= NUM_PARAMS {
        return;
    }
    let p = &mut *(effect as *mut Plugin);
    p.set_param(index as usize, value);
}

unsafe extern "system" fn get_parameter(effect: *mut AEffect, index: i32) -> f32 {
    if effect.is_null() || index < 0 || index as usize >= NUM_PARAMS {
        return 0.0;
    }
    let p = &*(effect as *mut Plugin);
    p.param(index as usize)
}

// ---------------------------------------------------------------------------
// DLL entry points
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn VSTPluginMain(_host: AudioMasterCallback) -> *mut AEffect {
    let boxed = Box::new(Plugin::new());
    let raw = Box::into_raw(boxed);
    unsafe { std::ptr::addr_of_mut!((*raw).effect) }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "system" fn main(_host: AudioMasterCallback) -> *mut AEffect {
    VSTPluginMain(_host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_of(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64).sqrt()
    }

    fn drive(p: &mut Plugin, amp: f32, freq: f32, blocks: usize) -> f64 {
        let n = 512usize;
        let fs = 48_000.0f32;
        let mut rin = 0.0f64;
        let mut rout = 0.0f64;
        let mut cnt = 0usize;
        for block in 0..blocks {
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for i in 0..n {
                let t = (block * n + i) as f32 / fs;
                l[i] = (2.0 * std::f32::consts::PI * freq * t).sin() * amp;
                r[i] = l[i];
            }
            let mut in_ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
            let mut ol = vec![0.0f32; n];
            let mut orr = vec![0.0f32; n];
            let mut out_ptrs = [ol.as_mut_ptr(), orr.as_mut_ptr()];
            unsafe {
                p.process(in_ptrs.as_mut_ptr(), out_ptrs.as_mut_ptr(), n as i32);
            }
            if block + 10 >= blocks {
                rin += rms_of(&l);
                rout += rms_of(&ol);
                cnt += 1;
            }
        }
        (rout / cnt as f64) / (rin / cnt as f64)
    }

    /// Quiet signal below the threshold must be EXPANDED (attenuated):
    /// at -60 dB with threshold -50 and ratio 3 the curve says -20 dB.
    #[test]
    fn quiet_below_threshold_is_expanded() {
        let mut p = Plugin::new();
        p.sample_rate = 48_000.0;
        p.set_param(P_THRESHOLD, -50.0);
        p.set_param(P_RATIO, 3.0);
        let ratio = drive(&mut p, 0.001, 1000.0, 60);
        assert!(ratio < 0.2, "expected heavy expansion, got ratio {ratio:.3}");
    }

    /// Loud signal above the threshold passes essentially unchanged.
    #[test]
    fn loud_passes() {
        let mut p = Plugin::new();
        p.sample_rate = 48_000.0;
        p.set_param(P_THRESHOLD, -50.0);
        p.set_param(P_RATIO, 3.0);
        let ratio = drive(&mut p, 0.5, 1000.0, 60);
        assert!((ratio - 1.0).abs() < 0.1, "expected passthrough, got {ratio:.3}");
    }

    /// Output gain scales linearly: +6 dB doubles.
    #[test]
    fn output_gain_scales() {
        let mut p = Plugin::new();
        p.sample_rate = 48_000.0;
        p.set_param(P_THRESHOLD, -80.0);
        p.set_param(P_RATIO, 1.0); // expansion curve effectively off
        p.set_param(P_OUT, 6.02);
        let ratio = drive(&mut p, 0.1, 1000.0, 60);
        assert!((ratio - 2.0).abs() < 0.15, "expected ~2x, got {ratio:.3}");
    }

    #[test]
    fn chunk_round_trip() {
        let mut p = Plugin::new();
        p.set_param(P_RATIO, 4.5);
        p.set_param(P_THRESHOLD, -33.0);
        p.set_param(P_ATTACK, 5.0);
        p.set_param(P_RELEASE, 250.0);
        p.set_param(P_OUT, -3.0);
        p.serialize();
        let blob = p.chunk.clone();
        let mut q = Plugin::new();
        q.deserialize(&blob);
        assert_eq!(q.param(P_RATIO), 4.5);
        assert_eq!(q.param(P_THRESHOLD), -33.0);
        assert_eq!(q.param(P_ATTACK), 5.0);
        assert_eq!(q.param(P_RELEASE), 250.0);
        assert_eq!(q.param(P_OUT), -3.0);
    }
}
