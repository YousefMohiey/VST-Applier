//! VST-Applier AI Denoise — a VST2 plugin around the built-in
//! DeepFilterNet3-ll denoiser (the same model the app embeds).
//!
//! Why a plugin: it drops into the app's chain like any other effect,
//! gets an editor with live knobs, and its state rides in the usual
//! profile chunks. The model is 48 kHz; the host may run the chain at
//! the capture rate (often 44.1 k), so the plugin resamples internally
//! (windowed-sinc, 16 taps) around the model.
//!
//! ABI notes: field-for-field the C `AEffect`; the host (our Rust VST2
//! host DLL) only uses this subset — open/close, sample-rate/block
//! size/mains, editor open/get-rect/close/idle, chunks, and the three
//! name strings — so that is exactly what is implemented here.

#![allow(unsafe_op_in_unsafe_fn)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use ai_denoise::Denoiser;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, FrameRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, TextOutW, ANTIALIASED_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FW_BOLD, FW_NORMAL, HBRUSH, HDC, HFONT,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW, LoadCursorW,
    RegisterClassW, SetWindowLongPtrW, GWLP_USERDATA, IDC_ARROW, WINDOW_EX_STYLE, WM_CLOSE,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_CHILD,
    WS_VISIBLE,
};

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

const P_ATTEN: usize = 0;
const P_PF: usize = 1;
const P_MIX: usize = 2;
const NUM_PARAMS: usize = 3;

/// name, slider range, display format
const PARAMS: [(&str, f32, f32); NUM_PARAMS] = [
    ("Attenuation", 0.0, 100.0),
    ("Post-filter", 0.0, 0.2),
    ("Mix", 0.0, 1.0),
];

/// `repr(C)` is load-bearing: the ABI passes `*mut AEffect` and both
/// sides cast the SAME address back to `*mut Plugin`, which is only
/// sound while `effect` sits at offset 0.
#[repr(C)]
struct Plugin {
    effect: AEffect,
    denoiser: Option<Denoiser>,
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
    /// ADN_TRACE captured ONCE — never look at the environment from the
    /// audio thread (the process call runs per block).
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
            unique_id: 0x4144_4E31, // 'ADN1'
            version: 1,
            process_replacing: Some(process_replacing),
            process_double_replacing: None,
            future: [0; 56],
        };
        Self {
            effect,
            denoiser: None,
            sample_rate: 48_000.0,
            params: [
                AtomicU32::new(100.0f32.to_bits()),
                AtomicU32::new(0.0f32.to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
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
                bottom: 190,
                right: 340,
            },
            trace: std::env::var_os("ADN_TRACE").is_some(),
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
        self.chunk.extend_from_slice(b"ADN1");
        for i in 0..NUM_PARAMS {
            self.chunk.extend_from_slice(&self.param(i).to_le_bytes());
        }
    }

    fn deserialize(&mut self, bytes: &[u8]) {
        if bytes.len() >= 4 + 4 * NUM_PARAMS && &bytes[..4] == b"ADN1" {
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
        let trace = self.trace;
        if trace {
            let sr = self.sample_rate;
            let sr_bits = sr.to_bits();
            let selfp = self as *mut Plugin;
            let sr_addr = &self.sample_rate as *const f32;
            let il = *inputs.add(0);
            let ir = *inputs.add(1);
            eprintln!("[adn] process n={n} sr={sr} srbits=0x{sr_bits:08x} self={selfp:p} sr_addr={sr_addr:p} in_l={il:p} in_r={ir:p}");
        }
        let in_l = *inputs.add(0);
        let in_r = *inputs.add(1);
        let out_l = *outputs.add(0);
        let out_r = *outputs.add(1);
        if in_l.is_null() || in_r.is_null() || out_l.is_null() || out_r.is_null() {
            return;
        }

        // Mono downmix of the chain lane (the mic is duplicated there).
        if self.mono_in.len() < n {
            self.mono_in.resize(n, 0.0);
        }
        for i in 0..n {
            self.mono_in[i] = (*in_l.add(i) + *in_r.add(i)) * 0.5;
        }

        if trace {
            eprintln!("[adn] downmix done");
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

        if trace {
            eprintln!("[adn] downmix ok, params ok");
        }
        if trace {
            eprintln!("[adn] params applied");
        }
        // Rate-convert to the model rate when the chain isn't at 48 k.
        let needs_resample = (self.sample_rate - 48_000.0).abs() > 0.5;
        let model_in: &[f32] = if needs_resample {
            self.ensure_resamplers();
            if trace {
                eprintln!("[adn] resamplers ensured");
            }
            let reserve = (n as f64 * 48_000.0 / self.sample_rate.max(1.0) as f64) as usize + 32;
            if self.up48.len() < reserve {
                self.up48.resize(reserve, 0.0);
            }
            let got = self
                .resampler_fwd
                .as_mut()
                .map(|r| r.process(&self.mono_in[..n], &mut self.up48))
                .unwrap_or(0);
            if trace {
                eprintln!("[adn] fwd resample done, got={got}");
            }
            &self.up48[..got]
        } else {
            &self.mono_in[..n]
        };

        if trace {
            eprintln!("[adn] fwd resample ok, model_in len {}", model_in.len());
        }
        // The model itself (1:1, inherent 10 ms latency).
        let ok = if let Some(d) = self.denoiser.as_mut() {
            if self.den_out.len() < model_in.len() {
                self.den_out.resize(model_in.len(), 0.0);
            }
            d.process(model_in, &mut self.den_out[..model_in.len()]);
            true
        } else {
            false
        };
        let wet48: &[f32] = if ok { &self.den_out[..model_in.len()] } else { model_in };

        if trace {
            eprintln!("[adn] model ok, den_out len {}", self.den_out.len());
        }
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

        if trace {
            eprintln!("[adn] back resample ok, wet len {}", wet.len());
        }
        // Dry/wet by the Mix knob, to both channels.
        let mix = self.param(P_MIX);
        for i in 0..n {
            let dry = self.mono_in[i];
            let w = wet.get(i).copied().unwrap_or(dry);
            let s = dry + (w - dry) * mix;
            *out_l.add(i) = s;
            *out_r.add(i) = s;
        }
    }

    unsafe fn open_editor(&mut self, parent: HWND) {
        if self.editor.is_some() {
            return;
        }
        ensure_editor_class();
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = windows::core::w!("VstApplierDenoiseEditor");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            windows::core::w!("AI Denoise"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            (self.rect.right - self.rect.left) as i32,
            (self.rect.bottom - self.rect.top) as i32,
            parent,
            None,
            instance,
            Some(self as *mut Plugin as *const c_void),
        );
        if let Ok(hwnd) = hwnd {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, self as *mut Plugin as isize);
            self.editor = Some(hwnd);
        }
    }

    fn close_editor(&mut self) {
        if let Some(hwnd) = self.editor.take() {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        self.close_editor();
    }
}

// ---------------------------------------------------------------------------
// Resampler — windowed-sinc, 16 taps, 128 precomputed phases per ratio.
// ---------------------------------------------------------------------------

const RES_TAPS: usize = 16;
const RES_HALF: usize = RES_TAPS / 2; // 8
const RES_PHASES: usize = 128;

struct Resampler {
    ratio: f64, // output_rate / input_rate
    pos: f64,   // read position, in the caller's timebase, of the next output sample
    tail: Vec<f32>,
    table: Vec<f32>, // [RES_PHASES][RES_TAPS]
}

fn kaiser(t: f64, beta: f64) -> f64 {
    // I0(beta * sqrt(1-t^2)) / I0(beta), t in [-1, 1]
    let arg = beta * (1.0 - t * t).max(0.0).sqrt();
    let mut s = 1.0f64;
    let mut term = 1.0f64;
    for k in 1..24 {
        term *= (arg / 2.0) * (arg / 2.0) / (k as f64 * k as f64);
        s += term;
    }
    let mut s0 = 1.0f64;
    let mut term0 = 1.0f64;
    for k in 1..24 {
        term0 *= (beta / 2.0) * (beta / 2.0) / (k as f64 * k as f64);
        s0 += term0;
    }
    s / s0
}

impl Resampler {
    fn new(ratio: f64) -> Self {
        let beta = 8.0;
        let mut table = vec![0.0f32; RES_PHASES * RES_TAPS];
        for ph in 0..RES_PHASES {
            let frac = ph as f64 / RES_PHASES as f64;
            let mut sum = 0.0f64;
            for k in 0..RES_TAPS {
                let t = k as f64 - (RES_HALF as f64 - 1.0) - frac;
                let sinc = if t.abs() < 1e-9 {
                    1.0
                } else {
                    (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t)
                };
                let w = kaiser((t / RES_HALF as f64).abs().min(1.0), beta);
                table[ph * RES_TAPS + k] = (sinc * w) as f32;
            }
            // normalise each phase so DC gain stays unity
            for k in 0..RES_TAPS {
                sum += table[ph * RES_TAPS + k] as f64;
            }
            if sum.abs() > 1e-9 {
                for k in 0..RES_TAPS {
                    table[ph * RES_TAPS + k] = (table[ph * RES_TAPS + k] as f64 / sum) as f32;
                }
            }
        }
        Self {
            ratio,
            pos: 0.0,
            tail: Vec::new(),
            table,
        }
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> usize {
        if input.is_empty() {
            return 0;
        }
        let tail_len = self.tail.len();
        let total = tail_len + input.len();
        let get = |i: isize| -> f32 {
            if i < 0 || i as usize >= total {
                0.0
            } else if (i as usize) < tail_len {
                self.tail[i as usize]
            } else {
                input[i as usize - tail_len]
            }
        };
        let step = 1.0 / self.ratio;
        let mut out_n = 0usize;
        // Only produce samples whose full 16-tap window lies inside the
        // combined buffer; the rest waits for the next call.
        while self.pos + RES_HALF as f64 <= (total - RES_HALF) as f64 && out_n < output.len() {
            let center = self.pos.floor();
            let frac = self.pos - center;
            let ph = ((frac * RES_PHASES as f64) as usize).min(RES_PHASES - 1);
            let taps = &self.table[ph * RES_TAPS..(ph + 1) * RES_TAPS];
            let mut acc = 0.0f32;
            for k in 0..RES_TAPS {
                let idx = center as isize - (RES_HALF as isize - 1) + k as isize;
                acc += get(idx) * taps[k];
            }
            output[out_n] = acc;
            out_n += 1;
            self.pos += step;
        }
        // Keep the tail needed for the next call's windows.
        let keep_from = (total as f64 - self.pos - RES_HALF as f64 - 2.0).max(0.0);
        let keep_from = keep_from as usize;
        let mut new_tail: Vec<f32> = Vec::with_capacity(total - keep_from);
        for i in keep_from..total {
            new_tail.push(if i < tail_len {
                self.tail[i]
            } else {
                input[i - tail_len]
            });
        }
        self.pos -= keep_from as f64;
        self.tail = new_tail;
        out_n
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
    if std::env::var_os("ADN_TRACE").is_some() {
        eprintln!(
            "[adn] dispatch op={opcode} index={index} value={value} opt={opt} opt_bits=0x{:08x} self={:p}",
            opt.to_bits(),
            p as *mut Plugin
        );
    }
    match opcode {
        EFF_OPEN => {
            if p.denoiser.is_none() {
                p.denoiser = Denoiser::new().ok();
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
            if std::env::var_os("ADN_TRACE").is_some() {
                eprintln!(
                    "[adn] sr write: value={} addr={:p} struct={} aeffect={}",
                    p.sample_rate,
                    &p.sample_rate as *const f32,
                    std::mem::size_of::<Plugin>(),
                    std::mem::size_of::<AEffect>()
                );
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
            write_vst_string(ptr, "AI Denoise", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "VST-Applier", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "VST-Applier AI Denoise (DFN3-ll)", 64);
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
// Editor window — dark, hairline sliders, drawn by hand in GDI.
// ---------------------------------------------------------------------------

const COL_BG: u32 = rgb(18, 19, 22);
const COL_BED: u32 = rgb(6, 6, 7);
const COL_LINE: u32 = rgb(42, 44, 49);
const COL_ACCENT: u32 = rgb(45, 129, 255);
const COL_INK: u32 = rgb(242, 241, 238);
const COL_MUTED: u32 = rgb(150, 150, 156);

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

const TRACK_X0: i32 = 108;
const TRACK_X1: i32 = 280;
const TRACK_H: i32 = 10;
const ROW_Y: [i32; NUM_PARAMS] = [46, 92, 138];

fn ensure_editor_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(editor_wndproc),
            hInstance: instance.into(),
            hCursor: cursor,
            lpszClassName: windows::core::w!("VstApplierDenoiseEditor"),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
}

unsafe extern "system" fn editor_wndproc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    let plugin_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Plugin;
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint_editor(hwnd, plugin_ptr);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if !plugin_ptr.is_null() {
                let x = (lp.0 & 0xFFFF) as i16 as i32;
                let y = (lp.0 >> 16) as i16 as i32;
                for (i, &row) in ROW_Y.iter().enumerate() {
                    if (y - (row + TRACK_H / 2)).abs() <= 14 {
                        let p = &mut *plugin_ptr;
                        let f = ((x - TRACK_X0) as f32
                            / (TRACK_X1 - TRACK_X0) as f32)
                            .clamp(0.0, 1.0);
                        let (_, lo, hi) = PARAMS[i];
                        p.set_param(i, lo + f * (hi - lo));
                        let _ = SetCapture(hwnd);
                        let _ = InvalidateRect(hwnd, None, false);
                        break;
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !plugin_ptr.is_null() && wp.0 & 1 != 0 {
                let x = (lp.0 & 0xFFFF) as i16 as i32;
                // apply to whichever slider is being dragged (nearest row)
                let y = (lp.0 >> 16) as i16 as i32;
                let mut best = 0usize;
                let mut best_d = i32::MAX;
                for (i, &row) in ROW_Y.iter().enumerate() {
                    let d = (y - (row + TRACK_H / 2)).abs();
                    if d < best_d {
                        best_d = d;
                        best = i;
                    }
                }
                let p = &mut *plugin_ptr;
                let f = ((x - TRACK_X0) as f32 / (TRACK_X1 - TRACK_X0) as f32)
                    .clamp(0.0, 1.0);
                let (_, lo, hi) = PARAMS[best];
                p.set_param(best, lo + f * (hi - lo));
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            LRESULT(0)
        }
        WM_CLOSE => LRESULT(0), // host owns the close flow
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn param_display(i: usize, v: f32) -> String {
    match i {
        P_ATTEN => format!("{:.0} dB", v),
        P_PF => {
            if v <= 0.0001 {
                "off".to_string()
            } else {
                format!("{:.3}", v)
            }
        }
        P_MIX => format!("{:.0}%", v * 100.0),
        _ => String::new(),
    }
}

unsafe fn paint_editor(hwnd: HWND, plugin_ptr: *mut Plugin) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);

    let brush = |c: u32| CreateSolidBrush(windows::Win32::Foundation::COLORREF(c));
    let bg = brush(COL_BG);
    FillRect(hdc, &rc, bg);
    DeleteObject(bg);

    let font = CreateFontW(
        -13,
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        ANTIALIASED_QUALITY.0 as u32,
        DEFAULT_PITCH.0 as u32,
        windows::core::w!("Segoe UI"),
    );
    let font_bold = CreateFontW(
        -14,
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        ANTIALIASED_QUALITY.0 as u32,
        DEFAULT_PITCH.0 as u32,
        windows::core::w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, font);
    SetBkMode(hdc, TRANSPARENT);

    // Title row
    let p = if plugin_ptr.is_null() {
        None
    } else {
        Some(&*plugin_ptr)
    };
    if let Some(p) = p {
        SelectObject(hdc, font_bold);
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(COL_INK));
        let title: Vec<u16> = "AI DENOISE".encode_utf16().collect();
        let _ = TextOutW(hdc, 14, 14, &title);
        SelectObject(hdc, font);
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(COL_MUTED));
        let sub: Vec<u16> = "DeepFilterNet3-ll".encode_utf16().collect();
        let _ = TextOutW(hdc, 226, 15, &sub);

        for (i, &row) in ROW_Y.iter().enumerate() {
            let v = p.param(i);
            let (name, lo, hi) = PARAMS[i];
            let ty = row + (TRACK_H / 2) - 9;

            // label
            SetTextColor(hdc, windows::Win32::Foundation::COLORREF(COL_MUTED));
            let label: Vec<u16> = name.encode_utf16().collect();
            let _ = TextOutW(hdc, 14, ty, &label);

            // track bed + hairline
            let bed = RECT {
                left: TRACK_X0,
                top: row,
                right: TRACK_X1,
                bottom: row + TRACK_H,
            };
            let bed_brush = brush(COL_BED);
            FillRect(hdc, &bed, bed_brush);
            DeleteObject(bed_brush);
            let line_brush = brush(COL_LINE);
            FrameRect(hdc, &bed, line_brush);

            // fill + thumb
            let frac = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            let fx = TRACK_X0 + ((TRACK_X1 - TRACK_X0) as f32 * frac) as i32;
            let fillr = RECT {
                left: TRACK_X0 + 1,
                top: row + 1,
                right: fx.max(TRACK_X0 + 1),
                bottom: row + TRACK_H - 1,
            };
            let fill_brush = brush(COL_ACCENT);
            FillRect(hdc, &fillr, fill_brush);
            DeleteObject(fill_brush);
            let thr = RECT {
                left: fx - 3,
                top: row - 5,
                right: fx + 3,
                bottom: row + TRACK_H + 5,
            };
            let thr_brush = brush(COL_INK);
            FillRect(hdc, &thr, thr_brush);
            DeleteObject(thr_brush);
            DeleteObject(line_brush);

            // value
            SetTextColor(hdc, windows::Win32::Foundation::COLORREF(COL_INK));
            let val: Vec<u16> = param_display(i, v).encode_utf16().collect();
            let _ = TextOutW(hdc, TRACK_X1 + 10, ty, &val);
        }
    }

    SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
    let _ = DeleteObject(font_bold);
    let _ = EndPaint(hwnd, &ps);
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

    /// Full plugin path at 44.1 kHz (internal resampler) — the same
    /// call the host makes, minus the DLL boundary.
    #[test]
    fn processes_at_44100() {
        let mut p = Plugin::new();
        p.denoiser = Denoiser::new().ok();
        assert!(p.denoiser.is_some(), "model load");
        p.sample_rate = 44_100.0;
        let n = 512usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        let mut ol = vec![0.0f32; n];
        let mut orr = vec![0.0f32; n];
        for block in 0..200 {
            for i in 0..n {
                let t = (block * n + i) as f32 / 44_100.0;
                l[i] = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.2;
                r[i] = l[i];
            }
            let mut in_ptrs = [l.as_mut_ptr(), r.as_mut_ptr()];
            let mut out_ptrs = [ol.as_mut_ptr(), orr.as_mut_ptr()];
            unsafe {
                p.process(in_ptrs.as_mut_ptr(), out_ptrs.as_mut_ptr(), n as i32);
            }
            assert!(
                ol.iter().all(|v| v.is_finite()) && orr.iter().all(|v| v.is_finite()),
                "non-finite output at block {block}"
            );
        }
        let last = &ol[..];
        let rms = (last.iter().map(|v| v * v).sum::<f32>() / last.len() as f32).sqrt();
        println!("out rms = {rms:.4}");
    }
}
