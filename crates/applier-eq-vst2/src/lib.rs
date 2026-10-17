//! Yousef 3-Band EQ — a VST2 three-band equalizer (High / Mid / Low,
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

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
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
    IDC_ARROW, IDC_HAND, LoadCursorW, RegisterClassW, SW_SHOWNORMAL, SetCursor, SetWindowLongPtrW,
    GWLP_USERDATA, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CHILD, WS_VISIBLE,
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
            rect: ERect { top: 0, left: 0, bottom: 190, right: 340 },
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
        ensure_editor_class();
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = windows::core::w!("VstApplierEqEditor");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            windows::core::w!("Yousef 3-Band EQ"),
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
            write_vst_string(ptr, "Yousef 3-Band EQ", 64);
            1
        }
        EFF_GET_VENDOR_STRING => {
            write_vst_string(ptr, "Yousef Mohiey", 64);
            1
        }
        EFF_GET_PRODUCT_STRING => {
            write_vst_string(ptr, "Yousef 3-Band EQ", 64);
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
// Editor window — the 3-Band Equalizer panel: label, hairline slider,
// dB readout box with up/down steppers per row. Drawn by hand in GDI,
// double-buffered (drag re-paints only ever BLT one finished frame).
// ---------------------------------------------------------------------------

const COL_BG: u32 = rgb(18, 19, 22);
const COL_BED: u32 = rgb(6, 6, 7);
const COL_LINE: u32 = rgb(42, 44, 49);
const COL_ACCENT: u32 = rgb(45, 129, 255);
const COL_INK: u32 = rgb(242, 241, 238);
const COL_MUTED: u32 = rgb(150, 150, 156);
const COL_BOX: u32 = rgb(28, 30, 34);

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

const TRACK_X0: i32 = 84;
const TRACK_X1: i32 = 228;
const TRACK_H: i32 = 10;
const ROW_Y: [i32; NUM_PARAMS] = [46, 92, 138];
/// dB readout box + stepper column geometry (right aligned).
const BOX_X0: i32 = 238;
const BOX_X1: i32 = 306;
const STEP_X0: i32 = 308;
const STEP_X1: i32 = 326;

const CREDIT_URL: &str = "https://github.com/YousefMohiey/VST-Applier";
/// Footer band top; the whole band is the credit link hit area.
const FOOTER_Y: i32 = 160;

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
            lpszClassName: windows::core::w!("VstApplierEqEditor"),
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
    static FONT_MAIN: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_BOLD: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_MONO: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
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

/// Which stepper (if any) is under (x, y): (row, +1 up / -1 down).
fn stepper_at(x: i32, y: i32) -> Option<(usize, f32)> {
    if !(STEP_X0..STEP_X1).contains(&x) {
        return None;
    }
    for (i, &row) in ROW_Y.iter().enumerate() {
        if (row - 8..=row + 4).contains(&y) {
            return Some((i, 1.0));
        }
        if (row + 6..=row + 18).contains(&y) {
            return Some((i, -1.0));
        }
    }
    None
}

unsafe extern "system" fn editor_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let plugin_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Plugin;
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint_editor(hwnd, plugin_ptr);
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
                    p.set_param(i, p.param(i) + dir);
                    let _ = InvalidateRect(hwnd, None, false);
                } else {
                    for (i, &row) in ROW_Y.iter().enumerate() {
                        if (y - (row + TRACK_H / 2)).abs() <= 14 && x < BOX_X0 {
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
                    || ((BOX_X0..BOX_X1).contains(&pt.x) && ROW_Y.iter().any(|&r| (pt.y - r).abs() <= 12));
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

fn param_display(v: f32) -> String {
    format!("{v:.2} dB")
}

/// A small up/down chevron drawn as stacked fills (no glyph fonts).
/// Up: apex at top (narrow row first), widening downward. Down: the
/// mirror. (Getting the two y-progressions swapped makes both arrows
/// point down — caught in the first capture.)
unsafe fn draw_chevron(dc: HDC, cx: i32, cy: i32, up: bool, color: u32) {
    let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(color));
    for k in 0..5 {
        let half = 1 + k as i32; // 1,2,3,4,5
        let y = if up { cy - 2 + k as i32 } else { cy + 2 - k as i32 };
        let r = RECT { left: cx - half, top: y, right: cx + half + 1, bottom: y + 1 };
        FillRect(dc, &r, brush);
    }
    DeleteObject(brush);
}

unsafe fn paint_editor(hwnd: HWND, plugin_ptr: *mut Plugin) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);
    let dc = ensure_mem_dc(hdc, w, h);

    let brush = |c: u32| CreateSolidBrush(windows::Win32::Foundation::COLORREF(c));
    let bg = brush(COL_BG);
    FillRect(dc, &rc, bg);
    DeleteObject(bg);

    let font = cached_font(&FONT_MAIN, -12, FW_NORMAL.0 as i32, windows::core::w!("Segoe UI"));
    let font_bold = cached_font(&FONT_BOLD, -13, FW_BOLD.0 as i32, windows::core::w!("Segoe UI"));
    let font_mono = cached_font(&FONT_MONO, -11, FW_NORMAL.0 as i32, windows::core::w!("Consolas"));
    let old_font = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT);

    let p = if plugin_ptr.is_null() { None } else { Some(&*plugin_ptr) };
    if let Some(p) = p {
        // Header: product name + hairline underneath.
        SelectObject(dc, font_bold);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_INK));
        let title: Vec<u16> = "YOUSEF 3-BAND EQ".encode_utf16().collect();
        let _ = TextOutW(dc, 14, 10, &title);
        let hline = RECT { left: 14, top: 32, right: w - 14, bottom: 33 };
        let hb = brush(COL_LINE);
        FillRect(dc, &hline, hb);
        DeleteObject(hb);

        SelectObject(dc, font);
        for (i, &row) in ROW_Y.iter().enumerate() {
            let v = p.param(i);
            let (name, lo, hi) = PARAMS[i];
            let ty = row + (TRACK_H / 2) - 9;

            // Micro-label in the app mono-uppercase style.
            SelectObject(dc, font_mono);
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_MUTED));
            let label: Vec<u16> = name.to_uppercase().encode_utf16().collect();
            let _ = TextOutW(dc, 14, ty + 1, &label);
            SelectObject(dc, font);

            // Track bed + border.
            let bed = RECT { left: TRACK_X0, top: row, right: TRACK_X1, bottom: row + TRACK_H };
            let bed_brush = brush(COL_BED);
            FillRect(dc, &bed, bed_brush);
            DeleteObject(bed_brush);
            let line_brush = brush(COL_LINE);
            FrameRect(dc, &bed, line_brush);

            // Fill + thumb. Bipolar: fill runs from center to the thumb.
            let frac = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            let fx = TRACK_X0 + ((TRACK_X1 - TRACK_X0) as f32 * frac) as i32;
            let mid = (TRACK_X0 + TRACK_X1) / 2;
            let fillr = RECT {
                left: fx.min(mid) + 1,
                top: row + 1,
                right: fx.max(mid),
                bottom: row + TRACK_H - 1,
            };
            let fill_brush = brush(COL_ACCENT);
            FillRect(dc, &fillr, fill_brush);
            DeleteObject(fill_brush);
            let thr = RECT { left: fx - 3, top: row - 5, right: fx + 3, bottom: row + TRACK_H + 5 };
            let thr_brush = brush(COL_INK);
            FillRect(dc, &thr, thr_brush);
            DeleteObject(thr_brush);
            DeleteObject(line_brush);

            // dB readout box.
            let valbox = RECT { left: BOX_X0, top: row - 6, right: BOX_X1, bottom: row + 16 };
            let box_brush = brush(COL_BOX);
            FillRect(dc, &valbox, box_brush);
            DeleteObject(box_brush);
            let boxline = brush(COL_LINE);
            FrameRect(dc, &valbox, boxline);
            DeleteObject(boxline);
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_INK));
            let val: Vec<u16> = param_display(v).encode_utf16().collect();
            let mut vsz = SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &val, &mut vsz);
            let tx = BOX_X0 + ((BOX_X1 - BOX_X0) - vsz.cx) / 2;
            let _ = TextOutW(dc, tx, row - 1, &val);

            // Up/down steppers.
            let up_box = RECT { left: STEP_X0, top: row - 8, right: STEP_X1, bottom: row + 4 };
            let dn_box = RECT { left: STEP_X0, top: row + 6, right: STEP_X1, bottom: row + 18 };
            for b in [up_box, dn_box] {
                let bb = brush(COL_BOX);
                FillRect(dc, &b, bb);
                DeleteObject(bb);
                let bl = brush(COL_LINE);
                FrameRect(dc, &b, bl);
                DeleteObject(bl);
            }
            let cx = (STEP_X0 + STEP_X1) / 2;
            draw_chevron(dc, cx, row - 1, true, COL_MUTED);
            draw_chevron(dc, cx, row + 11, false, COL_MUTED);
        }

        // Footer: hairline + credit line (whole band clickable).
        let frect = RECT { left: 14, top: FOOTER_Y, right: w - 14, bottom: FOOTER_Y + 1 };
        let fb = brush(COL_LINE);
        FillRect(dc, &frect, fb);
        DeleteObject(fb);
        SelectObject(dc, font_mono);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_MUTED));
        let by: Vec<u16> = "by Yousef Mohiey - ".encode_utf16().collect();
        let _ = TextOutW(dc, 14, FOOTER_Y + 6, &by);
        let mut by_sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &by, &mut by_sz);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_ACCENT));
        let link: Vec<u16> = "github.com/YousefMohiey/VST-Applier".encode_utf16().collect();
        let _ = TextOutW(dc, 14 + by_sz.cx, FOOTER_Y + 6, &link);
    }

    SelectObject(dc, old_font);
    let _ = BitBlt(hdc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
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
