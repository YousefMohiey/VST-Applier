//! Applier 3-Band EQ — a VST2 three-band equalizer (High / Mid / Low,
//! ±12 dB) for the VST-Applier chain.
//!
//! The editor mirrors the "Audio Filters → 3-Band Equalizer" layout the
//! user asked for: one horizontal slider per band with a dB readout box
//! and up/down steppers, drawn in the app's palette with the standard
//! credit footer. Signal path: low shelf → mid peak → high shelf,
//! per channel, RBJ cookbook coefficients, zero latency.
//!
//! ABI notes: field-for-field the C `AEffect`; the host (our Rust VST2
//! host DLL) uses the same subset as for the denoise plugin — open/
//! close, sample-rate/block size/mains, editor open/get-rect/close,
//! chunks, and the three name strings.

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

const P_HIGH: usize = 0;
const P_MID: usize = 1;
const P_LOW: usize = 2;
const NUM_PARAMS: usize = 3;

const EQ_MAX_DB: f32 = 12.0;

const PARAMS: [(&str, f32, f32); NUM_PARAMS] = [
    ("High", -EQ_MAX_DB, EQ_MAX_DB),
    ("Mid", -EQ_MAX_DB, EQ_MAX_DB),
    ("Low", -EQ_MAX_DB, EQ_MAX_DB),
];

/// Band design: low shelf 200 Hz, mid peak 1.2 kHz (Q 1.0), high
/// shelf 6 kHz — the classic voice 3-band split. Gains are the knobs.
const LOW_SHELF_HZ: f32 = 200.0;
const MID_PEAK_HZ: f32 = 1200.0;
const MID_PEAK_Q: f32 = 1.0;
const HIGH_SHELF_HZ: f32 = 6000.0;

// ---------------------------------------------------------------------------
// Biquads (RBJ cookbook, transposed direct form II), zero latency.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    const IDENTITY: Biquad = Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn from_parts(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    fn low_shelf(fs: f32, f: f32, db: f32) -> Self {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * std::f32::consts::PI * f / fs;
        let (c, s) = (w.cos(), w.sin());
        let alpha = s / 2.0 * 2f32.sqrt(); // slope S = 1
        let sqa = a.sqrt();
        let b0 = a * ((a + 1.0) - (a - 1.0) * c + 2.0 * sqa * alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * c);
        let b2 = a * ((a + 1.0) - (a - 1.0) * c - 2.0 * sqa * alpha);
        let a0 = (a + 1.0) + (a - 1.0) * c + 2.0 * sqa * alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * c);
        let a2 = (a + 1.0) + (a - 1.0) * c - 2.0 * sqa * alpha;
        Self::from_parts(b0, b1, b2, a0, a1, a2)
    }

    fn high_shelf(fs: f32, f: f32, db: f32) -> Self {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * std::f32::consts::PI * f / fs;
        let (c, s) = (w.cos(), w.sin());
        let alpha = s / 2.0 * 2f32.sqrt();
        let sqa = a.sqrt();
        let b0 = a * ((a + 1.0) + (a - 1.0) * c + 2.0 * sqa * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * c);
        let b2 = a * ((a + 1.0) + (a - 1.0) * c - 2.0 * sqa * alpha);
        let a0 = (a + 1.0) - (a - 1.0) * c + 2.0 * sqa * alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * c);
        let a2 = (a + 1.0) - (a - 1.0) * c - 2.0 * sqa * alpha;
        Self::from_parts(b0, b1, b2, a0, a1, a2)
    }

    fn peaking(fs: f32, f: f32, q: f32, db: f32) -> Self {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * std::f32::consts::PI * f / fs;
        let (c, s) = (w.cos(), w.sin());
        let alpha = s / (2.0 * q);
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * c;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * c;
        let a2 = 1.0 - alpha / a;
        Self::from_parts(b0, b1, b2, a0, a1, a2)
    }
}

/// Per-channel state for one biquad (16 bytes; two channels per band).
#[derive(Clone, Copy, Default)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    #[inline]
    fn run(&mut self, c: &Biquad, x: f32) -> f32 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

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
    applied: [f32; NUM_PARAMS],
    coeffs: [Biquad; 3], // [low, mid, high]
    state: [[BiquadState; 3]; 2], // [channel][band]
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
            unique_id: 0x4145_5131, // 'AEQ1'
            version: 1,
            process_replacing: Some(process_replacing),
            process_double_replacing: None,
            future: [0; 56],
        };
        Self {
            effect,
            sample_rate: 48_000.0,
            params: [
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            applied: [f32::NAN; NUM_PARAMS],
            coeffs: [Biquad::IDENTITY; 3],
            state: [[BiquadState::default(); 3]; 2],
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
        self.chunk.extend_from_slice(b"AEQ1");
        for i in 0..NUM_PARAMS {
            self.chunk.extend_from_slice(&self.param(i).to_le_bytes());
        }
    }

    fn deserialize(&mut self, bytes: &[u8]) {
        if bytes.len() >= 4 + 4 * NUM_PARAMS && &bytes[..4] == b"AEQ1" {
            for i in 0..NUM_PARAMS {
                let o = 4 + 4 * i;
                let v = f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                self.set_param(i, v);
            }
        }
    }

    fn rebuild_coeffs(&mut self, high: f32, mid: f32, low: f32) {
        let fs = self.sample_rate.max(8_000.0);
        self.coeffs = [
            Biquad::low_shelf(fs, LOW_SHELF_HZ, low),
            Biquad::peaking(fs, MID_PEAK_HZ, MID_PEAK_Q, mid),
            Biquad::high_shelf(fs, HIGH_SHELF_HZ, high),
        ];
        self.applied = [high, mid, low];
    }

    unsafe fn process(&mut self, inputs: *mut *mut f32, outputs: *mut *mut f32, frames: i32) {
        if frames <= 0 || inputs.is_null() || outputs.is_null() {
            return;
        }
        let n = frames as usize;

        // Apply knob changes at block boundary (the only thread that
        // touches the filter state is this one).
        let high = self.param(P_HIGH);
        let mid = self.param(P_MID);
        let low = self.param(P_LOW);
        if high != self.applied[P_HIGH] || mid != self.applied[P_MID] || low != self.applied[P_LOW]
        {
            self.rebuild_coeffs(high, mid, low);
        }

        for ch in 0..2 {
            let inp = *inputs.add(ch);
            let outp = *outputs.add(ch);
            if inp.is_null() || outp.is_null() {
                continue;
            }
            let (c_low, c_mid, c_high) = (self.coeffs[0], self.coeffs[1], self.coeffs[2]);
            let st = &mut self.state[ch];
            for i in 0..n {
                let x = *inp.add(i);
                // A corrupted upstream sample must not poison the
                // recursive state forever.
                let x = if x.is_finite() { x } else { 0.0 };
                let x = st[0].run(&c_low, x);
                let x = st[1].run(&c_mid, x);
                let y = st[2].run(&c_high, x);
                *outp.add(i) = if y.is_finite() { y } else { 0.0 };
            }
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
                p.rebuild_coeffs(p.param(P_HIGH), p.param(P_MID), p.param(P_LOW));
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
            write_vst_string(ptr, "Applier 3-Band EQ", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "Applier", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "Applier 3-Band EQ", 64);
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

    fn run(p: &mut Plugin, l: &mut [f32], r: &mut [f32], n: usize) -> (Vec<f32>, Vec<f32>) {
        let mut ol = vec![0.0f32; n];
        let mut orr = vec![0.0f32; n];
        let mut in_ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
        let mut out_ptrs = [ol.as_mut_ptr(), orr.as_mut_ptr()];
        unsafe {
            p.process(in_ptrs.as_mut_ptr(), out_ptrs.as_mut_ptr(), n as i32);
        }
        (ol, orr)
    }

    /// At 0 dB all three bands are exact identities → passthrough.
    #[test]
    fn flat_is_passthrough() {
        let mut p = Plugin::new();
        p.sample_rate = 48_000.0;
        p.rebuild_coeffs(0.0, 0.0, 0.0);
        let n = 512usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        for i in 0..n {
            l[i] = (i as f32 * 0.05).sin() * 0.3;
            r[i] = l[i];
        }
        let (ol, _) = run(&mut p, &mut l, &mut r, n);
        for i in 0..n {
            assert!(
                (ol[i] - l[i]).abs() < 1e-6,
                "flat EQ must be identity at sample {i}: {} vs {}",
                ol[i],
                l[i]
            );
        }
    }

    /// Low shelf +12 dB on a 80 Hz tone must raise RMS; -12 must drop it.
    /// High shelf -12 dB on an 11 kHz tone must drop it.
    #[test]
    fn bands_move_their_range() {
        let n = 512usize;
        let fs = 48_000.0f32;

        let measure = |low_db: f32, freq: f32| -> f32 {
            let mut p = Plugin::new();
            p.sample_rate = fs;
            p.set_param(P_LOW, low_db); // the real knob path
            let mut rms_in = 0.0f64;
            let mut rms_out = 0.0f64;
            let mut count = 0usize;
            for block in 0..40 {
                let mut l = vec![0.0f32; n];
                let mut r = vec![0.0f32; n];
                for i in 0..n {
                    let t = (block * n + i) as f32 / fs;
                    l[i] = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.2;
                    r[i] = l[i];
                }
                let (ol, _) = run(&mut p, &mut l, &mut r, n);
                if block >= 30 {
                    for i in 0..n {
                        rms_in += (l[i] as f64) * (l[i] as f64);
                        rms_out += (ol[i] as f64) * (ol[i] as f64);
                        count += 1;
                    }
                }
            }
            ((rms_out / count as f64).sqrt() / (rms_in / count as f64).sqrt()) as f32
        };

        let boost = measure(12.0, 80.0);
        assert!(boost > 2.5, "low +12 should strongly boost 80 Hz, got {boost:.2}");
        let cut = measure(-12.0, 80.0);
        assert!(cut < 0.45, "low -12 should strongly cut 80 Hz, got {cut:.2}");

        // High band on a high tone.
        let n2 = 512usize;
        let mut p = Plugin::new();
        p.sample_rate = fs;
        p.set_param(P_HIGH, -12.0); // the real knob path
        let mut rms_in = 0.0f64;
        let mut rms_out = 0.0f64;
        let mut count = 0usize;
        for block in 0..40 {
            let mut l = vec![0.0f32; n2];
            let mut r = vec![0.0f32; n2];
            for i in 0..n2 {
                let t = (block * n2 + i) as f32 / fs;
                l[i] = (2.0 * std::f32::consts::PI * 11_000.0 * t).sin() * 0.2;
                r[i] = l[i];
            }
            let (ol, _) = run(&mut p, &mut l, &mut r, n2);
            if block >= 30 {
                for i in 0..n2 {
                    rms_in += (l[i] as f64) * (l[i] as f64);
                    rms_out += (ol[i] as f64) * (ol[i] as f64);
                    count += 1;
                }
            }
        }
        let ratio = (rms_out / count as f64).sqrt() / (rms_in / count as f64).sqrt();
        assert!(ratio < 0.45, "high -12 should cut 11 kHz, got {ratio:.2}");
    }

    #[test]
    fn chunk_round_trip() {
        let mut p = Plugin::new();
        p.set_param(P_HIGH, 3.5);
        p.set_param(P_MID, -4.0);
        p.set_param(P_LOW, 1.0);
        p.serialize();
        let blob = p.chunk.clone();
        let mut q = Plugin::new();
        q.deserialize(&blob);
        assert_eq!(q.param(P_HIGH), 3.5);
        assert_eq!(q.param(P_MID), -4.0);
        assert_eq!(q.param(P_LOW), 1.0);
    }
}
