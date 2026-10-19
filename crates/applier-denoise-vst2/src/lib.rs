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

mod editor;
mod ui_kit;

use std::ffi::c_void;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use ai_denoise::Denoiser;

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

// ---------------------------------------------------------------------------
// Stall diagnostics: a tiny watchdog thread that appends to the app's log if
// a process() call ever runs pathologically long (or forever). This is how we
// catch a wedged audio thread in the act.
// ---------------------------------------------------------------------------

static PROCESSING_SINCE_MS: AtomicU64 = AtomicU64::new(0); // 0 = idle
static WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);
static LAST_REPORT_MS: AtomicU64 = AtomicU64::new(0);
static PROF_FWD_US: AtomicU64 = AtomicU64::new(0);
static PROF_MODEL_US: AtomicU64 = AtomicU64::new(0);
static PROF_BACK_US: AtomicU64 = AtomicU64::new(0);
static PROF_CALLS: AtomicU64 = AtomicU64::new(0);
static PROF_LAST_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn diag_append(msg: &str) {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let path = format!("{appdata}\\VstApplier\\rust-log.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[{}] {msg}", now_ms() / 1000);
        }
    }
}

fn ensure_watchdog() {
    if WATCHDOG_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let since = PROCESSING_SINCE_MS.load(Ordering::Relaxed);
        if since != 0 {
            let elapsed = now_ms().saturating_sub(since);
            if elapsed > 5_000 && now_ms().saturating_sub(LAST_REPORT_MS.load(Ordering::Relaxed)) > 10_000 {
                LAST_REPORT_MS.store(now_ms(), Ordering::Relaxed);
                diag_append(&format!(
                    "adn watchdog: process() running for {elapsed} ms and counting"
                ));
            }
        }
    });
}

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
        ensure_watchdog();
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
                bottom: ui_kit::BASE_H as i16,
                right: ui_kit::BASE_W as i16,
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

        let t0 = std::time::Instant::now();
        PROCESSING_SINCE_MS.store(now_ms(), Ordering::Relaxed);

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
        let t_fwd = std::time::Instant::now();
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
        let fwd_us = t_fwd.elapsed().as_micros() as u64;
        let t_model = std::time::Instant::now();
        // The model itself (1:1, inherent 10 ms latency).
        // The denoiser returns how many samples it produced; the rest
        // is `model_in` (dry) for this call — a short sliver (< 1 ms)
        // whose content comes back on the next call. Never zero-fill:
        // that injected silence and let internal queues creep.
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
        let wet48: &[f32] = if den_n > 0 { &self.den_out[..model_in.len()] } else { model_in };

        if trace {
            eprintln!("[adn] model ok, den_out len {}", self.den_out.len());
        }
        let model_us = t_model.elapsed().as_micros() as u64;
        let t_back = std::time::Instant::now();
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

        let back_us = t_back.elapsed().as_micros() as u64;
        if std::env::var_os("ADN_PROF").is_some() {
            let f = PROF_FWD_US.fetch_add(fwd_us, Ordering::Relaxed) + fwd_us;
            let m = PROF_MODEL_US.fetch_add(model_us, Ordering::Relaxed) + model_us;
            let b = PROF_BACK_US.fetch_add(back_us, Ordering::Relaxed) + back_us;
            let c = PROF_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
            if c % 500 == 0 && now_ms().saturating_sub(PROF_LAST_MS.load(Ordering::Relaxed)) > 900 {
                PROF_LAST_MS.store(now_ms(), Ordering::Relaxed);
                diag_append(&format!(
                    "adn PROF: {c} calls | fwd {:.0}us model {:.0}us back {:.0}us per call",
                    f as f64 / c as f64,
                    m as f64 / c as f64,
                    b as f64 / c as f64
                ));
            }
        }
        PROCESSING_SINCE_MS.store(0, Ordering::Relaxed);
        let ms = t0.elapsed().as_millis();
        if ms >= 20
            && now_ms().saturating_sub(LAST_REPORT_MS.load(Ordering::Relaxed)) > 2_000
        {
            LAST_REPORT_MS.store(now_ms(), Ordering::Relaxed);
            diag_append(&format!("adn slow block: {ms} ms (frames={n})"));
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
        // Keep only the samples the next call's windows can still need:
        // from `pos - RES_HALF` onward. (The old `total - pos - HALF - 2`
        // assumed `pos` stays near 0; after a full emission pos is close
        // to `total`, the expression clamped to 0, and the whole buffer
        // became the new tail every call — the tail grew ~input_len per
        // call and the copy cost climbed quadratically. That was the
        // 44.1 kHz slowdown that made the plugin burn a full core after
        // a few minutes in a real session.)
        let keep_from = (self.pos - RES_HALF as f64 - 1.0).max(0.0) as usize;
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
            write_vst_string(ptr, "Applier Noise Suppression", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "Applier", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "Applier Noise Suppression", 64);
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
