//! Applier Compressor — a VST2 feed-forward compressor for the VST-Applier
//! microphone chain.
//!
//! Signal path: stereo-linked peak detector (|L|/|R| max, one shared gain)
//! → dB-domain soft-knee gain computer → attack/release one-pole on the
//! gain reduction → makeup gain. Zero latency (no lookahead), the
//! detector/gain state is flushed so no denormals can creep into the
//! audio thread.
//!
//! The editor is the new big family panel: 640x400 logical pixels, a
//! 22 px title, 13 px mono uppercase labels, 16 px values in framed
//! readout boxes with up/down steppers, a gain-reduction meter and the
//! standard credit footer. Same palette and hairlines as the EQ kit,
//! scaled up ~1.6x so the text is readable.
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
// Parameters
// ---------------------------------------------------------------------------

const P_THRESHOLD: usize = 0;
const P_RATIO: usize = 1;
const P_ATTACK: usize = 2;
const P_RELEASE: usize = 3;
const P_KNEE: usize = 4;
const P_MAKEUP: usize = 5;
const NUM_PARAMS: usize = 6;

const PARAMS: [(&str, f32, f32); NUM_PARAMS] = [
    ("Threshold", -60.0, 0.0),
    ("Ratio", 1.0, 20.0),
    ("Attack", 0.1, 200.0),
    ("Release", 5.0, 1000.0),
    ("Knee", 0.0, 24.0),
    ("Makeup", 0.0, 24.0),
];

/// Stepper increment per parameter (a click on the up/down chevrons).
const PARAM_STEP: [f32; NUM_PARAMS] = [1.0, 0.5, 0.5, 5.0, 1.0, 0.5];

/// Soft-knee gain computer, dB in / dB out. Returns the gain change
/// (<= 0, i.e. gain reduction) for a detector level. Quadratic blend
/// across `knee_db` centred on the threshold; exact hard-knee curve
/// above/below, continuous in value and slope at both knee edges.
fn gain_computer_db(level_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
    let ratio = ratio.max(1.0);
    let slope = 1.0 - 1.0 / ratio; // 0 at 1:1, ->1 at huge ratios
    let over = level_db - threshold_db;
    if knee_db <= 0.0 {
        if over > 0.0 {
            -over * slope
        } else {
            0.0
        }
    } else {
        let half = knee_db * 0.5;
        if over <= -half {
            0.0
        } else if over < half {
            -slope * (over + half) * (over + half) / (2.0 * knee_db)
        } else {
            -over * slope
        }
    }
}

/// One-pole coefficient for a time constant in milliseconds. Guarded so
/// a 0.1 ms attack at 8 kHz never goes non-finite.
fn one_pole_coef(ms: f32, fs: f32) -> f32 {
    let tau = (ms.max(0.01) * 0.001 * fs).max(1.0);
    (-1.0 / tau).exp()
}

/// Flush anything that could drift into denormal territory. The family
/// convention is an explicit threshold rather than MXCSR FTZ so the
/// test build and the host build behave identically.
#[inline]
fn flush_denormal(x: f32) -> f32 {
    if x.abs() < 1e-25 {
        0.0
    } else {
        x
    }
}

#[inline]
fn sane(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
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
    /// Smoothed gain reduction in dB (<= 0), carried across blocks.
    gr_db: f32,
    /// Meter value for the UI thread: the same gain reduction in dB.
    meter: AtomicU32,
    /// Last seen detector level in dB (meter symmetry / debugging).
    level_db: f32,
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
            initial_delay: 0, // no lookahead: zero latency
            real_qualities: 0,
            off_qualities: 0,
            io_ratio: 0.0,
            object: std::ptr::null_mut(),
            user: std::ptr::null_mut(),
            unique_id: 0x4143_4F31, // 'ACO1'
            version: 1,
            process_replacing: Some(process_replacing),
            process_double_replacing: None,
            future: [0; 56],
        };
        Self {
            effect,
            sample_rate: 48_000.0,
            params: [
                AtomicU32::new((-18.0f32).to_bits()),
                AtomicU32::new(4.0f32.to_bits()),
                AtomicU32::new(10.0f32.to_bits()),
                AtomicU32::new(120.0f32.to_bits()),
                AtomicU32::new(6.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            gr_db: 0.0,
            meter: AtomicU32::new(0.0f32.to_bits()),
            level_db: -120.0,
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

    /// Current gain reduction in dB (<= 0) for the meter.
    fn meter_db(&self) -> f32 {
        f32::from_bits(self.meter.load(Ordering::Relaxed))
    }

    fn serialize(&mut self) {
        self.chunk.clear();
        self.chunk.extend_from_slice(b"ACO1");
        for i in 0..NUM_PARAMS {
            self.chunk.extend_from_slice(&self.param(i).to_le_bytes());
        }
    }

    fn deserialize(&mut self, bytes: &[u8]) {
        if bytes.len() >= 4 + 4 * NUM_PARAMS && &bytes[..4] == b"ACO1" {
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
        let fs = self.sample_rate.max(8_000.0);

        let thr = self.param(P_THRESHOLD);
        let ratio = self.param(P_RATIO);
        let knee = self.param(P_KNEE);
        let a_coef = one_pole_coef(self.param(P_ATTACK), fs);
        let r_coef = one_pole_coef(self.param(P_RELEASE), fs);
        let makeup = 10f32.powf(self.param(P_MAKEUP) / 20.0);

        let in_l = *inputs.add(0);
        let in_r = *inputs.add(1);
        let out_l = *outputs.add(0);
        let out_r = *outputs.add(1);
        if in_l.is_null() || out_l.is_null() {
            return;
        }
        let stereo = !in_r.is_null() && !out_r.is_null();

        let mut gr = self.gr_db;
        let mut last_level = self.level_db;

        for i in 0..n {
            let l = sane(*in_l.add(i));
            let r = if stereo { sane(*in_r.add(i)) } else { l };
            // Stereo-linked peak detector: one gain for both channels.
            let peak = l.abs().max(r.abs());
            let level_db = 20.0 * peak.max(1e-9).log10();
            last_level = level_db;

            let target = gain_computer_db(level_db, thr, ratio, knee);
            // More reduction is the "attack" direction; letting go is
            // the release direction.
            let coef = if target < gr { a_coef } else { r_coef };
            gr = flush_denormal(target + (gr - target) * coef);

            let g = 10f32.powf(gr / 20.0) * makeup;
            let yl = l * g;
            *out_l.add(i) = if yl.is_finite() { yl } else { 0.0 };
            if stereo {
                let yr = r * g;
                *out_r.add(i) = if yr.is_finite() { yr } else { 0.0 };
            }
        }

        self.gr_db = gr;
        self.level_db = last_level;
        self.meter.store(gr.to_bits(), Ordering::Relaxed);
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
            write_vst_string(ptr, "Applier Compressor", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "Applier", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "Applier Compressor", 64);
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

    const FS: f32 = 48_000.0;

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64).sqrt()
    }

    /// Runs `blocks` x 512 frames of a sine, returns (input rms, output
    /// rms) measured over the last 8 blocks (settled state).
    fn drive(p: &mut Plugin, amp: f32, freq: f32, blocks: usize) -> (f64, f64) {
        let n = 512usize;
        let mut rin = 0.0f64;
        let mut rout = 0.0f64;
        let mut cnt = 0usize;
        for block in 0..blocks {
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for i in 0..n {
                let t = (block * n + i) as f32 / FS;
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
            if block + 8 >= blocks {
                rin += rms(&l);
                rout += rms(&ol);
                cnt += 1;
            }
        }
        (rin / cnt as f64, rout / cnt as f64)
    }

    fn ratio_db(p: &mut Plugin, amp: f32, blocks: usize) -> f64 {
        let (i, o) = drive(p, amp, 1000.0, blocks);
        20.0 * (o / i).log10()
    }

    /// Runs `samples` frames of a constant value in 100-frame blocks.
    fn run_dc(p: &mut Plugin, value: f32, samples: usize) {
        let n = 100usize;
        let mut l = vec![value; n];
        let mut r = vec![value; n];
        let mut ol = vec![0.0f32; n];
        let mut orr = vec![0.0f32; n];
        let mut done = 0usize;
        while done < samples {
            let take = (samples - done).min(n);
            let mut in_ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
            let mut out_ptrs = [ol.as_mut_ptr(), orr.as_mut_ptr()];
            unsafe {
                p.process(in_ptrs.as_mut_ptr(), out_ptrs.as_mut_ptr(), take as i32);
            }
            done += take;
        }
    }

    #[test]
    fn gain_computer_matches_ratio_math() {
        // Hard knee: 20 dB over a 4:1 threshold -> 15 dB reduction.
        assert_eq!(gain_computer_db(-100.0, -20.0, 4.0, 0.0), 0.0);
        assert_eq!(gain_computer_db(-20.0, -20.0, 4.0, 0.0), 0.0);
        assert!((gain_computer_db(0.0, -20.0, 4.0, 0.0) + 15.0).abs() < 1e-5);
        assert!((gain_computer_db(-19.0, -20.0, 4.0, 0.0) + 0.75).abs() < 1e-5);
        // 1:1 is a bypass.
        assert_eq!(gain_computer_db(0.0, -40.0, 1.0, 0.0), 0.0);
    }

    #[test]
    fn soft_knee_is_continuous_and_half_slope_at_threshold() {
        let (thr, ratio, knee) = (-20.0f32, 4.0f32, 12.0f32);
        // No reduction at/below the lower knee edge.
        assert_eq!(gain_computer_db(-26.0, thr, ratio, knee), 0.0);
        // At the threshold: slope * knee / 8 = 0.75 * 1.5 = 1.125 dB.
        let at = gain_computer_db(thr, thr, ratio, knee);
        assert!((at + 1.125).abs() < 1e-4, "at-threshold GR {at}");
        // At the upper knee edge the curve meets the hard-knee line.
        let edge_soft = gain_computer_db(thr + 6.0, thr, ratio, knee);
        let edge_hard = gain_computer_db(thr + 6.0, thr, ratio, 0.0);
        assert!((edge_soft + 4.5).abs() < 1e-4);
        assert!((edge_soft - edge_hard).abs() < 1e-4);
        // And the transition is monotone through the knee.
        let mut prev = 0.0f32;
        for step in 0..=24 {
            let lv = thr - 6.0 + step as f32 * 0.5;
            let g = gain_computer_db(lv, thr, ratio, knee);
            assert!(g <= prev + 1e-6, "knee not monotone at {lv} dB");
            prev = g;
        }
    }

    #[test]
    fn below_threshold_passes_clean() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -20.0);
        p.set_param(P_RATIO, 4.0);
        p.set_param(P_KNEE, 0.0);
        let g = ratio_db(&mut p, 0.05, 40); // -26 dBFS, below -20
        assert!(g.abs() < 0.1, "expected passthrough, got {g:.2} dB");
    }

    #[test]
    fn above_threshold_reduces_by_ratio() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -26.0);
        p.set_param(P_RATIO, 4.0);
        p.set_param(P_KNEE, 0.0);
        p.set_param(P_ATTACK, 0.1);
        p.set_param(P_RELEASE, 1000.0);
        // 20 dB over 4:1 -> 15 dB of gain reduction.
        let g = ratio_db(&mut p, 0.5, 80);
        assert!((g + 15.0).abs() < 0.8, "expected -15 dB, got {g:.2} dB");
    }

    #[test]
    fn attack_timing_follows_the_time_constant() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -30.0);
        p.set_param(P_RATIO, 20.0);
        p.set_param(P_KNEE, 0.0);
        p.set_param(P_ATTACK, 50.0);
        p.set_param(P_RELEASE, 1000.0);
        // 0 dBFS DC steps in; target GR = 30 * (1 - 1/20) = 28.5 dB.
        run_dc(&mut p, 1.0, (0.050 * FS) as usize);
        let gr = -p.meter_db();
        let expect = 28.5 * (1.0 - (-1.0f32).exp());
        assert!(
            (gr - expect).abs() < 3.0,
            "GR after one attack tau {gr:.2} dB, expected ~{expect:.2} dB"
        );
    }

    #[test]
    fn release_timing_follows_the_time_constant() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -30.0);
        p.set_param(P_RATIO, 20.0);
        p.set_param(P_KNEE, 0.0);
        p.set_param(P_ATTACK, 1.0);
        p.set_param(P_RELEASE, 100.0);
        run_dc(&mut p, 1.0, FS as usize); // settle at 28.5 dB reduction
        let settled = -p.meter_db();
        assert!(settled > 27.0, "did not reach full reduction: {settled:.2}");
        run_dc(&mut p, 0.0, (0.100 * FS) as usize); // one release tau
        let gr = -p.meter_db();
        let expect = settled * (-1.0f32).exp();
        assert!(
            (gr - expect).abs() < 3.0,
            "GR after one release tau {gr:.2} dB, expected ~{expect:.2} dB"
        );
    }

    #[test]
    fn makeup_gain_scales_output() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -60.0); // no compression
        p.set_param(P_RATIO, 1.0);
        p.set_param(P_MAKEUP, 6.02);
        let g = ratio_db(&mut p, 0.05, 40);
        assert!((g - 6.02).abs() < 0.1, "expected +6 dB, got {g:.2} dB");
    }

    #[test]
    fn detector_is_stereo_linked() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -30.0);
        p.set_param(P_RATIO, 4.0);
        p.set_param(P_KNEE, 0.0);
        p.set_param(P_ATTACK, 0.1);
        p.set_param(P_RELEASE, 1000.0);
        let n = 512usize;
        let mut gl = 0.0f64;
        let mut gr = 0.0f64;
        let mut in_l = 0.0f64;
        let mut in_r = 0.0f64;
        for block in 0..60 {
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for i in 0..n {
                let t = (block * n + i) as f32 / FS;
                let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
                l[i] = s * 0.01; // quiet channel
                r[i] = s * 0.5; // loud channel drives the detector
            }
            let mut ip = [l.as_mut_ptr(), r.as_mut_ptr()];
            let mut ol = vec![0.0f32; n];
            let mut orr = vec![0.0f32; n];
            let mut op = [ol.as_mut_ptr(), orr.as_mut_ptr()];
            unsafe { p.process(ip.as_mut_ptr(), op.as_mut_ptr(), n as i32) };
            if block >= 52 {
                in_l += rms(&l);
                in_r += rms(&r);
                gl += rms(&ol);
                gr += rms(&orr);
            }
        }
        let db_l = 20.0 * (gl / in_l).log10();
        let db_r = 20.0 * (gr / in_r).log10();
        assert!((db_l - db_r).abs() < 0.5, "channels not linked: {db_l:.2} vs {db_r:.2}");
        // 0.5 peak (-6.02 dBFS) is 23.98 dB over -30 at 4:1 -> 17.98 dB.
        let expect = -(20.0 * 0.5f64.log10() + 30.0) * 0.75;
        assert!((db_r - expect).abs() < 1.0, "expected {expect:.2} dB, got {db_r:.2}");
    }

    #[test]
    fn chunk_round_trip() {
        let mut p = Plugin::new();
        p.set_param(P_THRESHOLD, -33.5);
        p.set_param(P_RATIO, 7.5);
        p.set_param(P_ATTACK, 2.5);
        p.set_param(P_RELEASE, 350.0);
        p.set_param(P_KNEE, 18.0);
        p.set_param(P_MAKEUP, 9.5);
        p.serialize();
        let blob = p.chunk.clone();
        assert_eq!(blob.len(), 4 + 4 * NUM_PARAMS);
        let mut q = Plugin::new();
        q.deserialize(&blob);
        assert_eq!(q.param(P_THRESHOLD), -33.5);
        assert_eq!(q.param(P_RATIO), 7.5);
        assert_eq!(q.param(P_ATTACK), 2.5);
        assert_eq!(q.param(P_RELEASE), 350.0);
        assert_eq!(q.param(P_KNEE), 18.0);
        assert_eq!(q.param(P_MAKEUP), 9.5);
    }

    #[test]
    fn nan_and_inf_never_reach_the_output() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        let n = 256usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        for i in 0..n {
            l[i] = match i % 5 {
                0 => f32::NAN,
                1 => f32::INFINITY,
                2 => f32::NEG_INFINITY,
                3 => 1e30,
                _ => 0.3,
            };
            r[i] = -l[i];
        }
        let mut ip = [l.as_mut_ptr(), r.as_mut_ptr()];
        let mut ol = vec![0.0f32; n];
        let mut orr = vec![0.0f32; n];
        let mut op = [ol.as_mut_ptr(), orr.as_mut_ptr()];
        unsafe { p.process(ip.as_mut_ptr(), op.as_mut_ptr(), n as i32) };
        assert!(ol.iter().all(|v| v.is_finite()));
        assert!(orr.iter().all(|v| v.is_finite()));
        assert!(p.meter_db().is_finite());
    }

    #[test]
    fn params_clamp_to_their_documented_ranges() {
        let p = Plugin::new();
        p.set_param(P_THRESHOLD, -400.0);
        assert_eq!(p.param(P_THRESHOLD), -60.0);
        p.set_param(P_RATIO, 100.0);
        assert_eq!(p.param(P_RATIO), 20.0);
        p.set_param(P_ATTACK, 0.0);
        assert_eq!(p.param(P_ATTACK), 0.1);
        p.set_param(P_KNEE, 40.0);
        assert_eq!(p.param(P_KNEE), 24.0);
    }
}
