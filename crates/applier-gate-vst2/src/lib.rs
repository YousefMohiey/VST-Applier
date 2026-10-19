//! Yousef Gate — a VST2 noise gate for the VST-Applier microphone chain.
//!
//! Signal path: stereo-linked peak detector (0.5 ms attack / 5 ms
//! release follower) → threshold comparator with 4 dB hysteresis and a
//! hold-down timer → attack/release gain ramp. Zero latency, no
//! lookahead, state flushed so denormals cannot creep into the audio
//! thread.
//!
//! The editor is the new big family panel: 640x400 logical pixels, a
//! 22 px title, 13 px mono uppercase labels, 16 px values in framed
//! readout boxes with up/down steppers, an open/closed indicator plus an
//! attenuation meter, and the standard credit footer. Same palette and
//! hairlines as the EQ kit, scaled up ~1.6x so the text is readable.
//!
//! ABI notes: field-for-field the C `AEffect`; the host (our Rust VST2
//! host DLL) uses the same subset as for the other bundled plugins.

#![allow(unsafe_op_in_unsafe_fn)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, FrameRect, GetTextExtentPoint32W, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, TextOutW, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, FW_BOLD, FW_NORMAL, HDC, HFONT, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, ScreenToClient, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowLongPtrW,
    IDC_ARROW, IDC_HAND, KillTimer, LoadCursorW, RegisterClassW, SW_SHOWNORMAL, SetCursor,
    SetTimer, SetWindowLongPtrW, GWLP_USERDATA, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_PRINT, WM_PRINTCLIENT,
    WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_CHILD, WS_VISIBLE,
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

const P_THRESHOLD: usize = 0;
const P_RANGE: usize = 1;
const P_ATTACK: usize = 2;
const P_HOLD: usize = 3;
const P_RELEASE: usize = 4;
const NUM_PARAMS: usize = 5;

const PARAMS: [(&str, f32, f32); NUM_PARAMS] = [
    ("Threshold", -80.0, 0.0),
    ("Range", -80.0, 0.0),
    ("Attack", 0.1, 100.0),
    ("Hold", 0.0, 500.0),
    ("Release", 5.0, 2000.0),
];

/// Stepper increment per parameter (a click on the up/down chevrons).
const PARAM_STEP: [f32; NUM_PARAMS] = [1.0, 1.0, 0.5, 5.0, 5.0];

/// Fixed hysteresis around the threshold, in dB: opens at
/// `threshold + 2 dB`, closes at `threshold - 2 dB`. Normal gate
/// behaviour — stops the gain chattering on a signal sitting right at
/// the threshold.
const HYSTERESIS_DB: f32 = 4.0;

/// Sidechain detector ballistics: fast enough to catch a syllable
/// onset, slow enough not to chatter on a waveform cycle.
const DET_ATTACK_MS: f32 = 0.5;
const DET_RELEASE_MS: f32 = 5.0;

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
    /// Sidechain peak envelope (linear), stereo-linked.
    env: f32,
    /// Gate state machine.
    open: bool,
    hold_left: i32,
    /// Smoothed gate gain in dB (range_db .. 0), carried across blocks.
    gain_db: f32,
    /// Meter value for the UI thread.
    meter: AtomicU32,
    /// Open/closed flag for the UI thread (1.0 = open).
    open_flag: AtomicU32,
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
            unique_id: 0x4147_5431, // 'AGT1'
            version: 1,
            process_replacing: Some(process_replacing),
            process_double_replacing: None,
            future: [0; 56],
        };
        Self {
            effect,
            sample_rate: 48_000.0,
            params: [
                AtomicU32::new((-45.0f32).to_bits()),
                AtomicU32::new((-60.0f32).to_bits()),
                AtomicU32::new(1.0f32.to_bits()),
                AtomicU32::new(50.0f32.to_bits()),
                AtomicU32::new(200.0f32.to_bits()),
            ],
            env: 0.0,
            open: false,
            hold_left: 0,
            gain_db: -60.0,
            meter: AtomicU32::new((-60.0f32).to_bits()),
            open_flag: AtomicU32::new(0.0f32.to_bits()),
            chunk: Vec::new(),
            editor: None,
            rect: ERect {
                top: 0,
                left: 0,
                bottom: WIN_H as i16,
                right: WIN_W as i16,
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

    /// Current gate gain in dB (range_db when closed, ~0 when open).
    fn meter_db(&self) -> f32 {
        f32::from_bits(self.meter.load(Ordering::Relaxed))
    }

    fn is_open(&self) -> bool {
        f32::from_bits(self.open_flag.load(Ordering::Relaxed)) > 0.5
    }

    fn serialize(&mut self) {
        self.chunk.clear();
        self.chunk.extend_from_slice(b"AGT1");
        for i in 0..NUM_PARAMS {
            self.chunk.extend_from_slice(&self.param(i).to_le_bytes());
        }
    }

    fn deserialize(&mut self, bytes: &[u8]) {
        if bytes.len() >= 4 + 4 * NUM_PARAMS && &bytes[..4] == b"AGT1" {
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
        let range_db = self.param(P_RANGE).min(0.0);
        let open_at = thr + HYSTERESIS_DB * 0.5;
        let close_at = thr - HYSTERESIS_DB * 0.5;
        let hold_samples = (self.param(P_HOLD) * 0.001 * fs).round() as i32;
        let a_coef = one_pole_coef(self.param(P_ATTACK), fs);
        let r_coef = one_pole_coef(self.param(P_RELEASE), fs);
        let det_a = one_pole_coef(DET_ATTACK_MS, fs);
        let det_r = one_pole_coef(DET_RELEASE_MS, fs);

        let in_l = *inputs.add(0);
        let in_r = *inputs.add(1);
        let out_l = *outputs.add(0);
        let out_r = *outputs.add(1);
        if in_l.is_null() || out_l.is_null() {
            return;
        }
        let stereo = !in_r.is_null() && !out_r.is_null();

        let mut env = self.env;
        let mut open = self.open;
        let mut hold_left = self.hold_left;
        let mut gain_db = self.gain_db;

        for i in 0..n {
            let l = sane(*in_l.add(i));
            let r = if stereo { sane(*in_r.add(i)) } else { l };
            // Stereo-linked peak detector: one gain for both channels.
            let peak = l.abs().max(r.abs());
            let det_coef = if peak > env { det_a } else { det_r };
            env = flush_denormal(peak + (env - peak) * det_coef);
            let level_db = 20.0 * env.max(1e-9).log10();

            // Comparator with hysteresis + hold. While open, the hold
            // timer is re-armed until the level falls under the close
            // threshold; then it counts down before the gate shuts.
            if !open {
                if level_db >= open_at {
                    open = true;
                    hold_left = hold_samples;
                }
            } else if level_db >= close_at {
                hold_left = hold_samples;
            } else if hold_left > 0 {
                hold_left -= 1;
            } else {
                open = false;
            }

            let target = if open { 0.0 } else { range_db };
            let coef = if target > gain_db { a_coef } else { r_coef };
            gain_db = flush_denormal(target + (gain_db - target) * coef);

            let g = 10f32.powf(gain_db / 20.0);
            let yl = l * g;
            *out_l.add(i) = if yl.is_finite() { yl } else { 0.0 };
            if stereo {
                let yr = r * g;
                *out_r.add(i) = if yr.is_finite() { yr } else { 0.0 };
            }
        }

        self.env = env;
        self.open = open;
        self.hold_left = hold_left;
        self.gain_db = gain_db;
        self.meter.store(gain_db.to_bits(), Ordering::Relaxed);
        self.open_flag
            .store(if open { 1.0f32 } else { 0.0f32 }.to_bits(), Ordering::Relaxed);
    }

    unsafe fn open_editor(&mut self, parent: HWND) {
        if self.editor.is_some() {
            return;
        }
        ensure_editor_class();
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = windows::core::w!("VstApplierGateEditor");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            windows::core::w!("Yousef Gate"),
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
            // The open/closed panel animates from the audio-thread
            // atomics; a UI timer keeps it repainting at ~30 fps.
            let _ = SetTimer(hwnd, METER_TIMER_ID, 33, None);
            self.editor = Some(hwnd);
        }
    }

    fn close_editor(&mut self) {
        if let Some(hwnd) = self.editor.take() {
            let _ = unsafe { KillTimer(hwnd, METER_TIMER_ID) };
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
            write_vst_string(ptr, "Yousef Gate", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "Yousef Mohiey", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "Yousef Gate", 64);
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
// Editor window — the new big family panel (640x400): 22 px title, 13 px
// mono uppercase labels, 16 px values in framed boxes with up/down
// steppers, an open/closed indicator + attenuation meter, credit footer.
// Drawn by hand in GDI, double-buffered.
// ---------------------------------------------------------------------------

const COL_BG: u32 = rgb(18, 19, 22);
const COL_BED: u32 = rgb(6, 6, 7);
const COL_LINE: u32 = rgb(42, 44, 49);
const COL_ACCENT: u32 = rgb(45, 129, 255);
const COL_INK: u32 = rgb(242, 241, 238);
const COL_MUTED: u32 = rgb(150, 150, 156);
const COL_BOX: u32 = rgb(28, 30, 34);
/// Warm meter accent — the one new tone, used only by the gate panel.
const COL_AMBER: u32 = rgb(232, 168, 72);
const COL_DIM: u32 = rgb(122, 122, 128);
/// Slightly stronger hairline for boxes/steppers (the page rules stay
/// on COL_LINE so the panel keeps its quiet frame).
const COL_LINE2: u32 = rgb(60, 62, 68);

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

const WIN_W: i32 = 640;
const WIN_H: i32 = 400;
const PAD: i32 = 26;

const TITLE_Y: i32 = 12;
const HEADER_LINE_Y: i32 = 50;

const TRACK_X0: i32 = 190;
const TRACK_X1: i32 = 470;
const TRACK_H: i32 = 14;
const ROW_Y: [i32; NUM_PARAMS] = [88, 128, 168, 208, 248];
/// Value readout box + stepper column geometry (right aligned).
const BOX_X0: i32 = 484;
const BOX_X1: i32 = 588;
const STEP_X0: i32 = 594;
const STEP_X1: i32 = 622;

const STATUS_Y0: i32 = 306;
const STATUS_Y1: i32 = 350;
const METER_X0: i32 = 210;
const METER_X1: i32 = 470;
const METER_Y0: i32 = 310;
const METER_Y1: i32 = 342;
const METER_RANGE_DB: f32 = 80.0;

const CREDIT_URL: &str = "https://github.com/YousefMohiey/VST-Applier";
/// Footer band top; the whole band is the credit link hit area.
const FOOTER_Y: i32 = 366;
const METER_TIMER_ID: usize = 1;

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
            lpszClassName: windows::core::w!("VstApplierGateEditor"),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
}

// Editor paint state. Thread-local: the editor lives on the app UI
// thread only. The memory-DC back buffer and the fonts are created
// once and reused.
thread_local! {
    static MEM_DC: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static MEM_BMP: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static MEM_SIZE: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
    static FONT_TITLE: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_LABEL: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_VALUE: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_SMALL: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static DRAG: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
}

unsafe fn cached_font(
    slot: &'static std::thread::LocalKey<std::cell::Cell<isize>>,
    height: i32,
    weight: i32,
    face: windows::core::PCWSTR,
) -> HFONT {
    slot.with(|c| {
        if c.get() == 0 {
            let f = CreateFontW(
                height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                OUT_DEFAULT_PRECIS.0 as u32,
                CLIP_DEFAULT_PRECIS.0 as u32,
                ANTIALIASED_QUALITY.0 as u32,
                DEFAULT_PITCH.0 as u32,
                face,
            );
            c.set(f.0 as isize);
        }
        HFONT(c.get() as *mut core::ffi::c_void)
    })
}

unsafe fn ensure_mem_dc(hdc: HDC, w: i32, h: i32) -> HDC {
    let reuse = MEM_DC.with(|c| c.get() != 0) && MEM_SIZE.with(|s| s.get() == (w, h));
    if !reuse {
        if MEM_DC.with(|c| c.get()) != 0 {
            let oldb = MEM_BMP.with(|b| b.replace(0));
            if oldb != 0 {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(
                    oldb as *mut core::ffi::c_void,
                ));
            }
            let olddc = MEM_DC.with(|c| c.replace(0));
            let _ = DeleteDC(HDC(olddc as *mut core::ffi::c_void));
        }
        let dc = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let _ = SelectObject(dc, bmp);
        MEM_DC.with(|c| c.set(dc.0 as isize));
        MEM_BMP.with(|b| b.set(bmp.0 as isize));
        MEM_SIZE.with(|s| s.set((w, h)));
    }
    HDC(MEM_DC.with(|c| c.get()) as *mut core::ffi::c_void)
}

unsafe fn fill_rect(dc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(COLORREF(color));
    FillRect(dc, r, b);
    let _ = DeleteObject(b);
}

unsafe fn frame_rect(dc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(COLORREF(color));
    FrameRect(dc, r, b);
    let _ = DeleteObject(b);
}

/// Draws text with the given font/colour and returns its pixel width.
unsafe fn draw_text(dc: HDC, x: i32, y: i32, s: &str, font: HFONT, color: u32) -> i32 {
    SelectObject(dc, font);
    SetTextColor(dc, COLORREF(color));
    let w: Vec<u16> = s.encode_utf16().collect();
    let _ = TextOutW(dc, x, y, &w);
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    sz.cx
}

/// Right-aligned variant for the value boxes.
unsafe fn draw_text_right(dc: HDC, right: i32, y: i32, s: &str, font: HFONT, color: u32) {
    let u: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(dc, &u, &mut sz);
    draw_text(dc, right - sz.cx, y, s, font, color);
}

/// A bigger up/down chevron drawn as stacked fills (no glyph fonts).
unsafe fn draw_chevron(dc: HDC, cx: i32, cy: i32, up: bool, color: u32) {
    let brush = CreateSolidBrush(COLORREF(color));
    for k in 0..7 {
        let half = 1 + k as i32; // 1..7 -> 15 px wide at the base
        let y = if up { cy - 3 + k as i32 } else { cy + 3 - k as i32 };
        let r = RECT {
            left: cx - half,
            top: y,
            right: cx + half + 1,
            bottom: y + 1,
        };
        FillRect(dc, &r, brush);
    }
    let _ = DeleteObject(brush);
}

/// Which stepper (if any) is under (x, y): (row, +1 up / -1 down).
fn stepper_at(x: i32, y: i32) -> Option<(usize, f32)> {
    if !(STEP_X0..STEP_X1).contains(&x) {
        return None;
    }
    for (i, &row) in ROW_Y.iter().enumerate() {
        if (row - 16..=row - 2).contains(&y) {
            return Some((i, 1.0));
        }
        if (row + 2..=row + 16).contains(&y) {
            return Some((i, -1.0));
        }
    }
    None
}

fn param_display(i: usize, v: f32) -> String {
    match i {
        P_THRESHOLD => format!("{v:.1} dB"),
        P_RANGE => format!("{v:.0} dB"),
        P_ATTACK => format!("{v:.1} ms"),
        P_HOLD => format!("{v:.0} ms"),
        P_RELEASE => format!("{v:.0} ms"),
        _ => String::new(),
    }
}

unsafe extern "system" fn editor_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let plugin_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Plugin;
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint_editor(hwnd, plugin_ptr);
            LRESULT(0)
        }
        // PrintWindow (editor screenshots, some hosts) asks through
        // WM_PRINT; render the panel straight into the supplied DC.
        WM_PRINT | WM_PRINTCLIENT => {
            let (w, h) = client_size(hwnd);
            paint_into(HDC(wp.0 as *mut core::ffi::c_void), w, h, plugin_ptr);
            LRESULT(0)
        }
        WM_TIMER => {
            if wp.0 == METER_TIMER_ID {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lp.0 & 0xFFFF) as i16 as i32;
            let y = (lp.0 >> 16) as i16 as i32;
            if y >= FOOTER_Y {
                // Credit link: open the author GitHub page.
                let url = windows::core::HSTRING::from(CREDIT_URL);
                let _ = ShellExecuteW(
                    None,
                    windows::core::w!("open"),
                    windows::core::PCWSTR(url.as_ptr()),
                    windows::core::PCWSTR::null(),
                    windows::core::PCWSTR::null(),
                    SW_SHOWNORMAL,
                );
            } else if !plugin_ptr.is_null() {
                if let Some((i, dir)) = stepper_at(x, y) {
                    let p = &mut *plugin_ptr;
                    p.set_param(i, p.param(i) + dir * PARAM_STEP[i]);
                    let _ = InvalidateRect(hwnd, None, false);
                } else {
                    for (i, &row) in ROW_Y.iter().enumerate() {
                        if (y - row).abs() <= 16 && (TRACK_X0 - 6..BOX_X0).contains(&x) {
                            let p = &mut *plugin_ptr;
                            let f = ((x - TRACK_X0) as f32 / (TRACK_X1 - TRACK_X0) as f32)
                                .clamp(0.0, 1.0);
                            let (_, lo, hi) = PARAMS[i];
                            p.set_param(i, lo + f * (hi - lo));
                            DRAG.with(|d| d.set(i as i32));
                            let _ = SetCapture(hwnd);
                            let _ = InvalidateRect(hwnd, None, false);
                            break;
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !plugin_ptr.is_null() && wp.0 & 1 != 0 {
                let active = DRAG.with(|d| d.get());
                if active >= 0 {
                    let x = (lp.0 & 0xFFFF) as i16 as i32;
                    let p = &mut *plugin_ptr;
                    let f = ((x - TRACK_X0) as f32 / (TRACK_X1 - TRACK_X0) as f32)
                        .clamp(0.0, 1.0);
                    let (_, lo, hi) = PARAMS[active as usize];
                    p.set_param(active as usize, lo + f * (hi - lo));
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            DRAG.with(|d| d.set(-1));
            let _ = ReleaseCapture();
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let mut pt = windows::Win32::Foundation::POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                let _ = ScreenToClient(hwnd, &mut pt);
                let hand = pt.y >= FOOTER_Y
                    || stepper_at(pt.x, pt.y).is_some()
                    || (pt.x < BOX_X0 && ROW_Y.iter().any(|&r| (pt.y - r).abs() <= 16));
                if hand {
                    if let Ok(c) = LoadCursorW(None, IDC_HAND) {
                        let _ = SetCursor(c);
                        return LRESULT(1);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, METER_TIMER_ID);
            let oldb = MEM_BMP.with(|b| b.replace(0));
            if oldb != 0 {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(
                    oldb as *mut core::ffi::c_void,
                ));
            }
            let olddc = MEM_DC.with(|c| c.replace(0));
            if olddc != 0 {
                let _ = DeleteDC(HDC(olddc as *mut core::ffi::c_void));
            }
            MEM_SIZE.with(|s| s.set((0, 0)));
            LRESULT(0)
        }
        WM_CLOSE => LRESULT(0), // host owns the close flow
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn paint_editor(hwnd: HWND, plugin_ptr: *mut Plugin) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let (w, h) = client_size(hwnd);
    let dc = ensure_mem_dc(hdc, w, h);
    paint_into(dc, w, h, plugin_ptr);
    let _ = BitBlt(hdc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    (rc.right, rc.bottom)
}

/// Renders the whole panel into `dc` — used by WM_PAINT's back buffer
/// and by WM_PRINT/WM_PRINTCLIENT so PrintWindow captures the real panel.
unsafe fn paint_into(dc: HDC, w: i32, h: i32, plugin_ptr: *mut Plugin) {
    let rc = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    fill_rect(dc, &rc, COL_BG);

    let f_title = cached_font(&FONT_TITLE, -22, FW_BOLD.0 as i32, windows::core::w!("Segoe UI"));
    let f_label = cached_font(&FONT_LABEL, -13, FW_NORMAL.0 as i32, windows::core::w!("Consolas"));
    let f_value = cached_font(&FONT_VALUE, -16, FW_NORMAL.0 as i32, windows::core::w!("Segoe UI"));
    let f_small = cached_font(&FONT_SMALL, -11, FW_NORMAL.0 as i32, windows::core::w!("Consolas"));
    let old_font = SelectObject(dc, f_label);
    SetBkMode(dc, TRANSPARENT);

    // Header: big product name, section tag, hairline.
    draw_text(dc, PAD, TITLE_Y, "YOUF GATE", f_title, COL_INK);
    draw_text_right(dc, w - PAD, TITLE_Y + 6, "VST2 / DYNAMICS", f_label, COL_DIM);
    let hline = RECT {
        left: PAD,
        top: HEADER_LINE_Y,
        right: w - PAD,
        bottom: HEADER_LINE_Y + 1,
    };
    fill_rect(dc, &hline, COL_LINE);

    // Column captions.
    draw_text(dc, PAD, 58, "PARAMETER", f_small, COL_DIM);
    draw_text(dc, BOX_X0, 58, "VALUE", f_small, COL_DIM);

    let p = if plugin_ptr.is_null() {
        None
    } else {
        Some(&*plugin_ptr)
    };
    if let Some(p) = p {
        for (i, &row) in ROW_Y.iter().enumerate() {
            let v = p.param(i);
            let (name, lo, hi) = PARAMS[i];

            // Mono uppercase micro-label on the left.
            draw_text(dc, PAD, row - 9, &name.to_uppercase(), f_label, COL_MUTED);

            // Track bed + hairline border.
            let bed = RECT {
                left: TRACK_X0,
                top: row - TRACK_H / 2,
                right: TRACK_X1,
                bottom: row + TRACK_H / 2,
            };
            fill_rect(dc, &bed, COL_BED);
            frame_rect(dc, &bed, COL_LINE);

            // Unipolar fill from the left + 9 px thumb.
            let frac = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            let fx = TRACK_X0 + ((TRACK_X1 - TRACK_X0) as f32 * frac) as i32;
            if fx > TRACK_X0 + 1 {
                let fillr = RECT {
                    left: TRACK_X0 + 1,
                    top: row - TRACK_H / 2 + 1,
                    right: fx,
                    bottom: row + TRACK_H / 2 - 1,
                };
                fill_rect(dc, &fillr, COL_ACCENT);
            }
            let thumb = RECT {
                left: (fx - 4).max(TRACK_X0),
                top: row - TRACK_H / 2 - 3,
                right: (fx + 5).min(TRACK_X1),
                bottom: row + TRACK_H / 2 + 3,
            };
            fill_rect(dc, &thumb, COL_INK);

            // Framed value readout.
            let valbox = RECT {
                left: BOX_X0,
                top: row - 14,
                right: BOX_X1,
                bottom: row + 14,
            };
            fill_rect(dc, &valbox, COL_BOX);
            frame_rect(dc, &valbox, COL_LINE2);
            let txt = param_display(i, v);
            let mut sz = SIZE::default();
            let u: Vec<u16> = txt.encode_utf16().collect();
            let _ = GetTextExtentPoint32W(dc, &u, &mut sz);
            draw_text(
                dc,
                BOX_X0 + ((BOX_X1 - BOX_X0) - sz.cx) / 2,
                row - 10,
                &txt,
                f_value,
                COL_INK,
            );

            // Up/down steppers (the whole box is the hit area).
            let up_box = RECT {
                left: STEP_X0,
                top: row - 16,
                right: STEP_X1,
                bottom: row - 2,
            };
            let dn_box = RECT {
                left: STEP_X0,
                top: row + 2,
                right: STEP_X1,
                bottom: row + 16,
            };
            for b in [up_box, dn_box] {
                fill_rect(dc, &b, COL_BOX);
                frame_rect(dc, &b, COL_LINE2);
            }
            let cx = (STEP_X0 + STEP_X1) / 2;
            draw_chevron(dc, cx, row - 9, true, COL_MUTED);
            draw_chevron(dc, cx, row + 9, false, COL_MUTED);
        }

        // Divider above the status panel.
        let div = RECT {
            left: PAD,
            top: 280,
            right: w - PAD,
            bottom: 281,
        };
        fill_rect(dc, &div, COL_LINE);

        draw_text(dc, PAD, 292, "STATUS", f_label, COL_DIM);
        draw_text(dc, 204, 292, "ATTENUATION", f_label, COL_DIM);

        let open = p.is_open();
        let gain_db = p.meter_db();
        let att = (-gain_db).clamp(0.0, METER_RANGE_DB);

        // Open/closed indicator box with an LED.
        let ind = RECT {
            left: PAD,
            top: STATUS_Y0,
            right: 184,
            bottom: STATUS_Y1,
        };
        fill_rect(dc, &ind, COL_BOX);
        frame_rect(dc, &ind, COL_LINE);
        let led = RECT {
            left: PAD + 14,
            top: (STATUS_Y0 + STATUS_Y1) / 2 - 6,
            right: PAD + 26,
            bottom: (STATUS_Y0 + STATUS_Y1) / 2 + 6,
        };
        fill_rect(dc, &led, if open { COL_AMBER } else { COL_BED });
        frame_rect(dc, &led, COL_LINE);
        let state = if open { "OPEN" } else { "CLOSED" };
        draw_text(
            dc,
            PAD + 38,
            STATUS_Y0 + 10,
            state,
            f_title,
            if open { COL_AMBER } else { COL_MUTED },
        );

        // Attenuation meter (0 at the left, 80 dB at the right).
        let bed = RECT {
            left: METER_X0,
            top: METER_Y0,
            right: METER_X1,
            bottom: METER_Y1,
        };
        fill_rect(dc, &bed, COL_BED);
        frame_rect(dc, &bed, COL_LINE2);
        let width = (METER_X1 - METER_X0 - 2) as f32;
        if att > 0.01 {
            let fw = (width * att / METER_RANGE_DB) as i32;
            let fr = RECT {
                left: METER_X0 + 1,
                top: METER_Y0 + 1,
                right: METER_X0 + 1 + fw,
                bottom: METER_Y1 - 1,
            };
            fill_rect(dc, &fr, COL_AMBER);
        }
        for db in [20.0f32, 40.0, 60.0] {
            let tx = METER_X0 + 1 + (width * db / METER_RANGE_DB) as i32;
            let tr = RECT {
                left: tx,
                top: METER_Y0 + 1,
                right: tx + 1,
                bottom: METER_Y1 - 1,
            };
            fill_rect(dc, &tr, COL_LINE);
        }
        SelectObject(dc, f_small);
        for (db, label) in [(0.0f32, "0"), (20.0, "20"), (40.0, "40"), (60.0, "60"), (80.0, "80")] {
            let tick = METER_X0 + 1 + (width * db / METER_RANGE_DB) as i32;
            let mut sz = SIZE::default();
            let u: Vec<u16> = label.encode_utf16().collect();
            let _ = GetTextExtentPoint32W(dc, &u, &mut sz);
            let tx = (tick - sz.cx / 2).clamp(METER_X0, METER_X1 - sz.cx);
            draw_text(dc, tx, METER_Y1 + 4, label, f_small, COL_DIM);
        }

        let mbox = RECT {
            left: BOX_X0,
            top: METER_Y0,
            right: BOX_X1,
            bottom: METER_Y1,
        };
        fill_rect(dc, &mbox, COL_BOX);
        frame_rect(dc, &mbox, COL_LINE2);
        let mtxt = if att < 0.05 {
            "0.0 dB".to_string()
        } else {
            format!("-{att:.1} dB")
        };
        draw_text_right(dc, BOX_X1 - 10, METER_Y0 + 8, &mtxt, f_value, COL_INK);
    }

    // Footer: hairline + credit line (whole band clickable).
    let frect = RECT {
        left: PAD,
        top: FOOTER_Y,
        right: w - PAD,
        bottom: FOOTER_Y + 1,
    };
    fill_rect(dc, &frect, COL_LINE);
    let by_w = draw_text(dc, PAD, FOOTER_Y + 6, "by Yousef Mohiey - ", f_small, COL_MUTED);
    draw_text(
        dc,
        PAD + by_w,
        FOOTER_Y + 6,
        "github.com/YousefMohiey/VST-Applier",
        f_small,
        COL_ACCENT,
    );

    SelectObject(dc, old_font);
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

    /// RMS transfer ratio for the settled part of a sine run.
    fn transfer(p: &mut Plugin, amp: f32, blocks: usize) -> f64 {
        let n = 512usize;
        let mut rin = 0.0f64;
        let mut rout = 0.0f64;
        for block in 0..blocks {
            let mut l = vec![0.0f32; n];
            let mut r = vec![0.0f32; n];
            for i in 0..n {
                let t = (block * n + i) as f32 / FS;
                let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * amp;
                l[i] = s;
                r[i] = s;
            }
            let mut ip = [l.as_mut_ptr(), r.as_mut_ptr()];
            let mut ol = vec![0.0f32; n];
            let mut orr = vec![0.0f32; n];
            let mut op = [ol.as_mut_ptr(), orr.as_mut_ptr()];
            unsafe { p.process(ip.as_mut_ptr(), op.as_mut_ptr(), n as i32) };
            if block + 4 >= blocks {
                rin += rms(&l);
                rout += rms(&ol);
            }
        }
        rout / rin
    }

    #[test]
    fn loud_signal_opens_and_passes() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -40.0);
        p.set_param(P_ATTACK, 0.5);
        p.set_param(P_RANGE, -60.0);
        let t = transfer(&mut p, 0.5, 40); // -6 dBFS, way over the threshold
        assert!(p.is_open(), "gate did not open");
        assert!((t - 1.0).abs() < 0.02, "expected unity transfer, got {t:.3}");
    }

    #[test]
    fn quiet_signal_is_attenuated_by_range() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -20.0);
        p.set_param(P_RANGE, -60.0);
        p.set_param(P_HOLD, 0.0);
        let t = transfer(&mut p, 0.01, 40); // -40 dBFS, far below the threshold
        let want = 10f32.powf(-60.0 / 20.0) as f64;
        assert!(!p.is_open(), "gate should stay closed");
        assert!(
            (t - want).abs() < want * 0.05,
            "expected {want:.5} transfer, got {t:.5}"
        );
    }

    #[test]
    fn hold_keeps_the_gate_open_before_release() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -40.0);
        p.set_param(P_RANGE, -60.0);
        p.set_param(P_ATTACK, 0.1);
        p.set_param(P_HOLD, 100.0);
        p.set_param(P_RELEASE, 100.0);
        run_dc(&mut p, 0.5, (0.100 * FS) as usize); // open
        assert!(p.is_open());
        assert!(p.meter_db() > -0.5, "gain did not reach unity: {:.2}", p.meter_db());
        run_dc(&mut p, 0.0, (0.060 * FS) as usize); // inside the hold window
        assert!(p.is_open(), "gate closed during hold");
        assert!(
            p.meter_db() > -1.0,
            "gain fell during hold: {:.2} dB",
            p.meter_db()
        );
        run_dc(&mut p, 0.0, (0.600 * FS) as usize); // past hold + release
        assert!(!p.is_open(), "gate never closed");
        assert!(
            p.meter_db() < -50.0,
            "gate did not reach the floor: {:.2} dB",
            p.meter_db()
        );
    }

    #[test]
    fn hysteresis_holds_open_between_the_two_thresholds() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -30.0);
        p.set_param(P_RANGE, -60.0);
        p.set_param(P_ATTACK, 0.1);
        p.set_param(P_HOLD, 10.0);
        p.set_param(P_RELEASE, 50.0);
        run_dc(&mut p, 0.1, (0.200 * FS) as usize); // -20 dB: opens
        assert!(p.is_open());
        // -30.5 dB sits below the threshold but above threshold - 2 dB:
        // hysteresis must keep it open.
        run_dc(&mut p, 0.02985, (0.200 * FS) as usize);
        assert!(p.is_open(), "hysteresis failed, gate closed at -30.5 dB");
        assert!(p.meter_db() > -1.0);
        // -34 dB is under threshold - 2 dB: hold expires, gate shuts.
        run_dc(&mut p, 0.01995, (0.400 * FS) as usize);
        assert!(!p.is_open(), "gate did not close below the hysteresis band");
        assert!(p.meter_db() < -50.0);
    }

    #[test]
    fn attack_ramps_open_with_the_time_constant() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -40.0);
        p.set_param(P_RANGE, -60.0);
        p.set_param(P_ATTACK, 20.0);
        p.set_param(P_HOLD, 10.0);
        p.set_param(P_RELEASE, 100.0);
        run_dc(&mut p, 0.5, (0.020 * FS) as usize);
        let g = p.meter_db();
        // One attack tau from a -60 dB start: -60 * e^-1 = -22.1 dB.
        assert!(g > -32.0 && g < -14.0, "attack ramp at {g:.2} dB, expected ~-22 dB");
    }

    #[test]
    fn release_returns_to_the_floor() {
        let mut p = Plugin::new();
        p.sample_rate = FS;
        p.set_param(P_THRESHOLD, -40.0);
        p.set_param(P_RANGE, -48.0);
        p.set_param(P_ATTACK, 0.1);
        p.set_param(P_HOLD, 0.0);
        p.set_param(P_RELEASE, 50.0);
        run_dc(&mut p, 0.5, (0.100 * FS) as usize);
        assert!(p.meter_db() > -0.5);
        run_dc(&mut p, 0.0, (0.400 * FS) as usize);
        assert!(!p.is_open());
        assert!(
            (p.meter_db() + 48.0).abs() < 1.0,
            "floor should be -48 dB, got {:.2}",
            p.meter_db()
        );
    }

    #[test]
    fn chunk_round_trip() {
        let mut p = Plugin::new();
        p.set_param(P_THRESHOLD, -52.5);
        p.set_param(P_RANGE, -37.0);
        p.set_param(P_ATTACK, 3.5);
        p.set_param(P_HOLD, 85.0);
        p.set_param(P_RELEASE, 640.0);
        p.serialize();
        let blob = p.chunk.clone();
        assert_eq!(blob.len(), 4 + 4 * NUM_PARAMS);
        let mut q = Plugin::new();
        q.deserialize(&blob);
        assert_eq!(q.param(P_THRESHOLD), -52.5);
        assert_eq!(q.param(P_RANGE), -37.0);
        assert_eq!(q.param(P_ATTACK), 3.5);
        assert_eq!(q.param(P_HOLD), 85.0);
        assert_eq!(q.param(P_RELEASE), 640.0);
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
        p.set_param(P_THRESHOLD, 20.0);
        assert_eq!(p.param(P_THRESHOLD), 0.0);
        p.set_param(P_RANGE, -200.0);
        assert_eq!(p.param(P_RANGE), -80.0);
        p.set_param(P_ATTACK, 0.0);
        assert_eq!(p.param(P_ATTACK), 0.1);
        p.set_param(P_HOLD, 9000.0);
        assert_eq!(p.param(P_HOLD), 500.0);
        p.set_param(P_RELEASE, 1.0);
        assert_eq!(p.param(P_RELEASE), 5.0);
    }
}
