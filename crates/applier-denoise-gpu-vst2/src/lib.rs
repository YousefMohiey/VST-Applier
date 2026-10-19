//! VST-Applier denoise GPU — DeepFilterNet3-ll on ONNX Runtime (DirectML).
//!
//! The model is the same DFN3-ll net the CPU plugin embeds, but the three
//! graphs (encoder, ERB decoder, DF decoder) run as ONNX Runtime sessions on
//! the GPU through the DirectML execution provider, with the recurrent state
//! carried frame to frame inside ONNX (no tract, no pulsed model). If
//! DirectML is unavailable the same sessions run on the CPU EP; if ONNX
//! Runtime is missing entirely the plugin still loads and passes audio
//! through, and the editor says so.
//!
//! The model runs at 48 kHz in 10 ms hops. When the host's chain runs at
//! another rate the plugin resamples around the model (windowed-sinc, 16
//! taps), same as the CPU plugin.
//!
//! ABI notes: field-for-field the C `AEffect`; the host only uses the subset
//! implemented here (open/close, sample rate, block size, mains, editor
//! open/get-rect/close/idle, chunks, the three name strings).

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(non_snake_case)]

mod editor;
mod engine;

mod ui_kit;

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use engine::{GpuDenoiser, Resampler};

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
pub struct AEffect {
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

pub(crate) const P_ATTEN: usize = 0;
pub(crate) const P_PF: usize = 1;
pub(crate) const NUM_PARAMS: usize = 2;

/// name (already in the micro-label style), min, max
pub(crate) const PARAMS: [(&str, f32, f32); NUM_PARAMS] = [
    ("ATTENUATION LIMIT", 0.0, 100.0),
    ("POST-FILTER", 0.0, 0.2),
];

/// `repr(C)` is load-bearing: the ABI passes `*mut AEffect` and both sides
/// cast the SAME address back to `*mut Plugin`, which is only sound while
/// `effect` sits at offset 0.
#[repr(C)]
pub struct Plugin {
    effect: AEffect,
    pub(crate) denoiser: Option<GpuDenoiser>,
    /// Why the denoiser failed to initialise, if it did (shown in the editor).
    pub(crate) init_error: Option<String>,
    sample_rate: f32,
    params: [AtomicU32; NUM_PARAMS],
    // audio-thread only
    applied_atten: f32,
    applied_pf: f32,
    resampler_fwd: Option<Resampler>,
    resampler_back: Option<Resampler>,
    mono_in: Vec<f32>,
    up48: Vec<f32>,
    den_out: Vec<f32>,
    back: Vec<f32>,
    chunk: Vec<u8>,
    editor: Option<HWND>,
    rect: ERect,
    /// Captured once — never look at the environment from the audio thread.
    trace: bool,
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
            initial_delay: 480,
            real_qualities: 0,
            off_qualities: 0,
            io_ratio: 0.0,
            object: std::ptr::null_mut(),
            user: std::ptr::null_mut(),
            unique_id: 0x4144_4731, // 'ADG1'
            version: 1,
            process_replacing: Some(process_replacing),
            process_double_replacing: None,
            future: [0; 56],
        };
        Self {
            effect,
            denoiser: None,
            init_error: None,
            sample_rate: 48_000.0,
            params: [
                AtomicU32::new(100.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
            ],
            applied_atten: f32::NAN,
            applied_pf: f32::NAN,
            resampler_fwd: None,
            resampler_back: None,
            mono_in: Vec::new(),
            up48: Vec::new(),
            den_out: Vec::new(),
            back: Vec::new(),
            chunk: Vec::new(),
            editor: None,
            rect: ERect {
                top: 0,
                left: 0,
                bottom: ui_kit::BASE_H as i16,
                right: ui_kit::BASE_W as i16,
            },
            trace: std::env::var_os("ADG_TRACE").is_some(),
        }
    }

    pub(crate) fn param(&self, i: usize) -> f32 {
        f32::from_bits(self.params[i].load(Ordering::Relaxed))
    }

    pub(crate) fn set_param(&self, i: usize, v: f32) {
        let (_, lo, hi) = PARAMS[i];
        self.params[i].store(v.clamp(lo, hi).to_bits(), Ordering::Relaxed);
    }

    fn serialize(&mut self) {
        self.chunk.clear();
        self.chunk.extend_from_slice(b"ADG1");
        for i in 0..NUM_PARAMS {
            self.chunk.extend_from_slice(&self.param(i).to_le_bytes());
        }
    }

    fn deserialize(&mut self, bytes: &[u8]) {
        if bytes.len() >= 4 + 4 * NUM_PARAMS && &bytes[..4] == b"ADG1" {
            for i in 0..NUM_PARAMS {
                let o = 4 + 4 * i;
                let v = f32::from_le_bytes([
                    bytes[o],
                    bytes[o + 1],
                    bytes[o + 2],
                    bytes[o + 3],
                ]);
                self.set_param(i, v);
            }
        }
    }

    fn ensure_resamplers(&mut self) {
        if self.resampler_fwd.is_none() && (self.sample_rate - 48_000.0).abs() > 0.5 {
            let ratio = 48_000.0 / self.sample_rate as f64;
            self.resampler_fwd = Some(Resampler::new(ratio));
            self.resampler_back = Some(Resampler::new(1.0 / ratio));
        } else if (self.sample_rate - 48_000.0).abs() <= 0.5 {
            self.resampler_fwd = None;
            self.resampler_back = None;
        }
    }

    unsafe fn process(&mut self, inputs: *mut *mut f32, outputs: *mut *mut f32, frames: i32) {
        if frames <= 0 || inputs.is_null() || outputs.is_null() {
            return;
        }
        let n = frames as usize;
        let in_l = *inputs.add(0);
        let in_r = *inputs.add(1);
        let out_l = *outputs.add(0);
        let out_r = *outputs.add(1);
        if in_l.is_null() || out_l.is_null() {
            return;
        }

        // Mono downmix of the chain lane (the mic is duplicated there).
        if self.mono_in.len() < n {
            self.mono_in.resize(n, 0.0);
        }
        for i in 0..n {
            let l = *in_l.add(i);
            let r = if in_r.is_null() { l } else { *in_r.add(i) };
            self.mono_in[i] = (l + r) * 0.5;
        }

        // Apply knob changes on the audio thread (the only thread that
        // touches the model).
        let atten = self.param(P_ATTEN);
        if atten != self.applied_atten {
            if let Some(d) = self.denoiser.as_mut() {
                d.set_atten_lim_db(atten);
            }
            self.applied_atten = atten;
        }
        let pf = self.param(P_PF);
        if pf != self.applied_pf {
            if let Some(d) = self.denoiser.as_mut() {
                d.set_post_filter_beta(pf);
            }
            self.applied_pf = pf;
        }

        // Rate-convert to the model rate when the chain isn't at 48 k.
        let needs_resample = (self.sample_rate - 48_000.0).abs() > 0.5;
        let model_in: &[f32] = if needs_resample {
            self.ensure_resamplers();
            let reserve = (n as f64 * 48_000.0 / self.sample_rate.max(1.0) as f64) as usize + 32;
            if self.up48.len() < reserve {
                self.up48.resize(reserve, 0.0);
            }
            let got = self
                .resampler_fwd
                .as_mut()
                .map(|r| r.process(&self.mono_in[..n], &mut self.up48))
                .unwrap_or(0);
            &self.up48[..got]
        } else {
            &self.mono_in[..n]
        };

        // The model (1:1 over time, inherent ~10 ms latency). A short sliver
        // (< 1 hop) comes back on the next call; never zero-fill it.
        let den_n = if let Some(d) = self.denoiser.as_mut() {
            if self.den_out.len() < model_in.len() {
                self.den_out.resize(model_in.len(), 0.0);
            }
            let produced = d.process(model_in, &mut self.den_out[..model_in.len()]);
            if produced < model_in.len() {
                self.den_out[produced..model_in.len()]
                    .copy_from_slice(&model_in[produced..model_in.len()]);
            }
            produced
        } else {
            0
        };
        let wet48: &[f32] = if den_n > 0 {
            &self.den_out[..model_in.len()]
        } else {
            model_in
        };

        // Back to the chain rate.
        let wet: &[f32] = if needs_resample {
            let reserve = n + 32;
            if self.back.len() < reserve {
                self.back.resize(reserve, 0.0);
            }
            let got = self
                .resampler_back
                .as_mut()
                .map(|r| r.process(wet48, &mut self.back))
                .unwrap_or(0);
            &self.back[..got]
        } else {
            wet48
        };

        for i in 0..n {
            let dry = self.mono_in[i];
            let s = wet.get(i).copied().unwrap_or(dry);
            *out_l.add(i) = s;
            if !out_r.is_null() {
                *out_r.add(i) = s;
            }
        }

        if self.trace && n > 0 {
            eprintln!(
                "[adg] n={n} sr={} model_in={} wet={} den_n={den_n}",
                self.sample_rate,
                model_in.len(),
                wet.len()
            );
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
    opt: f32,
) -> isize {
    if effect.is_null() {
        return 0;
    }
    let p = &mut *(effect as *mut Plugin);
    match opcode {
        EFF_OPEN => {
            if p.denoiser.is_none() {
                match GpuDenoiser::new() {
                    Ok(d) => {
                        p.denoiser = Some(d);
                        p.init_error = None;
                    }
                    Err(e) => {
                        p.init_error = Some(e);
                    }
                }
            }
            1
        }
        EFF_CLOSE => {
            // Host is done: free ourselves (it releases the module next).
            drop(Box::from_raw(effect as *mut Plugin));
            0
        }
        EFF_SET_SAMPLE_RATE => {
            if opt > 0.0 {
                p.sample_rate = opt;
            }
            let delay = if (p.sample_rate - 48_000.0).abs() <= 0.5 {
                480.0
            } else {
                480.0 * (p.sample_rate as f64 / 48_000.0) + 16.0
            };
            p.effect.initial_delay = delay as i32;
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
            write_vst_string(ptr, "Applier Noise Suppression GPU", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "Applier", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "Applier Noise Suppression GPU", 64);
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

    #[test]
    fn params_clamp_and_roundtrip_through_chunk() {
        let p = Plugin::new();
        p.set_param(P_ATTEN, 1000.0);
        assert_eq!(p.param(P_ATTEN), 100.0);
        p.set_param(P_PF, 0.05);
        let mut p = p;
        p.serialize();
        let chunk = p.chunk.clone();
        let mut q = Plugin::new();
        q.deserialize(&chunk);
        assert_eq!(q.param(P_ATTEN), 100.0);
        assert_eq!(q.param(P_PF), 0.05);
    }

    #[test]
    fn editor_rect_is_the_big_one() {
        let p = Plugin::new();
        assert_eq!(p.rect.right - p.rect.left, 640);
        assert_eq!(p.rect.bottom - p.rect.top, 400);
    }
}
