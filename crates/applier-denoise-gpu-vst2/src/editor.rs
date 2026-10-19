//! The editor: 640x400 logical px of dark panel, amber accent, mono
//! micro-labels — the family palette, sized to actually be readable.
//!
//! Everything is drawn by hand into an off-screen DC and blitted as one frame,
//! so dragging a slider can't flicker.

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, FrameRect, GetTextExtentPoint32W, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, TextOutW, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, FW_BOLD, FW_NORMAL, HDC, HFONT, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowLongPtrW,
    LoadCursorW, RegisterClassW, SetCursor, SetWindowLongPtrW, GWLP_USERDATA, IDC_ARROW, IDC_HAND,
    SW_SHOWNORMAL, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CHILD, WS_VISIBLE,
};

use df_ort::EpKind;

use crate::{Plugin, NUM_PARAMS, PARAMS, P_ATTEN, P_PF};

// ---------------------------------------------------------------------------
// Geometry (logical px; effEditGetRect reports exactly this)
// ---------------------------------------------------------------------------

pub const EDITOR_W: i32 = 640;
pub const EDITOR_H: i32 = 400;

const MARGIN: i32 = 24;
const CONTENT_R: i32 = EDITOR_W - MARGIN;

const HEADER_RULE_Y: i32 = 58;
const PARAM_LABEL_Y: [i32; NUM_PARAMS] = [74, 138];
const PARAM_TRACK_Y: [i32; NUM_PARAMS] = [100, 164];
const TRACK_H: i32 = 14;
/// Vertical tolerance for grabbing a track.
const HIT_H: i32 = 20;

const CARD: RECT = RECT {
    left: MARGIN,
    top: 196,
    right: CONTENT_R,
    bottom: 320,
};

const FOOTER_RULE_Y: i32 = 340;
const CREDIT_URL: &str = "https://github.com/YousefMohiey/VST-Applier";

// ---------------------------------------------------------------------------
// Palette — the family dark panel with an amber accent for the GPU line
// ---------------------------------------------------------------------------

const COL_BG: u32 = rgb(18, 19, 22);
const COL_BED: u32 = rgb(6, 6, 7);
const COL_LINE: u32 = rgb(42, 44, 49);
const COL_BOX: u32 = rgb(28, 30, 34);
const COL_INK: u32 = rgb(242, 241, 238);
const COL_MUTED: u32 = rgb(150, 150, 156);
const COL_AMBER: u32 = rgb(255, 193, 77);
const COL_BAD: u32 = rgb(255, 98, 89);

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

// ---------------------------------------------------------------------------
// GDI helpers
// ---------------------------------------------------------------------------

thread_local! {
    static MEM_DC: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static MEM_BMP: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static MEM_SIZE: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
    static FONT_TITLE: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_LABEL: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_VALUE: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_MICRO: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_FOOTER: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static DRAG: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
}

unsafe fn cached_font(
    slot: &'static std::thread::LocalKey<std::cell::Cell<isize>>,
    height: i32,
    weight: i32,
    face: PCWSTR,
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
        HFONT(c.get() as *mut c_void)
    })
}

unsafe fn fill(dc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(COLORREF(color));
    FillRect(dc, r, b);
    let _ = DeleteObject(b);
}

unsafe fn frame(dc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(COLORREF(color));
    FrameRect(dc, r, b);
    let _ = DeleteObject(b);
}

unsafe fn text(dc: HDC, x: i32, y: i32, s: &str, color: u32) {
    SetTextColor(dc, COLORREF(color));
    let w: Vec<u16> = s.encode_utf16().collect();
    let _ = TextOutW(dc, x, y, &w);
}

unsafe fn text_width(dc: HDC, s: &str) -> i32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    sz.cx
}

unsafe fn text_right(dc: HDC, right: i32, y: i32, s: &str, color: u32) {
    let w = text_width(dc, s);
    text(dc, right - w, y, s, color);
}

unsafe fn ensure_mem_dc(hdc: HDC, w: i32, h: i32) -> HDC {
    let reuse = MEM_DC.with(|c| c.get() != 0) && MEM_SIZE.with(|s| s.get() == (w, h));
    if !reuse {
        if MEM_DC.with(|c| c.get()) != 0 {
            let oldb = MEM_BMP.with(|b| b.replace(0));
            if oldb != 0 {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(oldb as *mut c_void));
            }
            let olddc = MEM_DC.with(|c| c.replace(0));
            let _ = DeleteDC(HDC(olddc as *mut c_void));
        }
        let dc = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let _ = SelectObject(dc, bmp);
        MEM_DC.with(|c| c.set(dc.0 as isize));
        MEM_BMP.with(|b| b.set(bmp.0 as isize));
        MEM_SIZE.with(|s| s.set((w, h)));
    }
    HDC(MEM_DC.with(|c| c.get()) as *mut c_void)
}

/// Draw the two sliders; `fraction` is the hit-testable x→value mapping.
unsafe fn draw_param(dc: HDC, i: usize, value: f32, lo: f32, hi: f32) {
    let label_font = cached_font(&FONT_LABEL, -13, FW_NORMAL.0 as i32, windows::core::w!("Consolas"));
    let value_font = cached_font(&FONT_VALUE, -16, FW_NORMAL.0 as i32, windows::core::w!("Segoe UI"));

    let y_label = PARAM_LABEL_Y[i];
    let y_track = PARAM_TRACK_Y[i];
    SelectObject(dc, label_font);
    SetBkMode(dc, TRANSPARENT);
    text(dc, MARGIN, y_label, PARAMS[i].0, COL_MUTED);

    // Value readout, right aligned with the track end.
    SelectObject(dc, value_font);
    text_right(dc, CONTENT_R, y_label - 6, &param_display(i, value), COL_INK);

    // Track: recessed bed, hairline frame, amber fill, ink thumb.
    let bed = RECT {
        left: MARGIN,
        top: y_track,
        right: CONTENT_R,
        bottom: y_track + TRACK_H,
    };
    fill(dc, &bed, COL_BED);
    frame(dc, &bed, COL_LINE);
    let frac = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
    let fx = MARGIN + ((CONTENT_R - MARGIN) as f32 * frac) as i32;
    if fx > MARGIN + 1 {
        let f = RECT {
            left: MARGIN + 1,
            top: y_track + 1,
            right: fx,
            bottom: y_track + TRACK_H - 1,
        };
        fill(dc, &f, COL_AMBER);
    }
    let thumb = RECT {
        left: fx - 3,
        top: y_track - 5,
        right: fx + 3,
        bottom: y_track + TRACK_H + 5,
    };
    fill(dc, &thumb, COL_INK);
}

fn param_display(i: usize, v: f32) -> String {
    match i {
        P_ATTEN => format!("{:.0} dB", v),
        P_PF => {
            if v <= 0.0005 {
                "OFF".to_string()
            } else {
                format!("{:.3}", v)
            }
        }
        _ => String::new(),
    }
}

unsafe fn draw_status_card(dc: HDC, plugin: Option<&Plugin>) {
    fill(dc, &CARD, COL_BOX);
    frame(dc, &CARD, COL_LINE);

    let micro = cached_font(&FONT_MICRO, -12, FW_NORMAL.0 as i32, windows::core::w!("Consolas"));
    let value = cached_font(&FONT_VALUE, -16, FW_NORMAL.0 as i32, windows::core::w!("Segoe UI"));

    SelectObject(dc, micro);
    SetBkMode(dc, TRANSPARENT);
    text(dc, CARD.left + 16, CARD.top + 12, "INFERENCE", COL_MUTED);

    let (ep, status, detail) = match plugin.and_then(|p| p.denoiser.as_ref()) {
        Some(d) => (d.ep(), d.status_line(), d.detail_line()),
        None => match plugin.and_then(|p| p.init_error.as_ref()) {
            Some(e) => (
                EpKind::Missing,
                e.clone(),
                "audio passes through untouched".to_string(),
            ),
            None => (
                EpKind::Missing,
                "not initialised".to_string(),
                String::new(),
            ),
        },
    };
    let color = match ep {
        EpKind::DirectMl => COL_AMBER,
        EpKind::Cpu => COL_INK,
        EpKind::Missing => COL_BAD,
    };

    // Badge, right side of the card.
    let label = ep.short();
    SelectObject(dc, micro);
    let bw = text_width(dc, label) + 24;
    let badge = RECT {
        left: CARD.right - 16 - bw,
        top: CARD.top + 34,
        right: CARD.right - 16,
        bottom: CARD.top + 58,
    };
    fill(dc, &badge, COL_BED);
    frame(dc, &badge, color);
    let tw = text_width(dc, label);
    text(
        dc,
        badge.left + (bw - tw) / 2,
        badge.top + 6,
        label,
        color,
    );

    SelectObject(dc, value);
    text(dc, CARD.left + 16, CARD.top + 34, &status, color);

    if !detail.is_empty() {
        SelectObject(dc, micro);
        // Trim to what fits; GDI has no ellipsis at this size without DrawText.
        let max_cx = CARD.right - CARD.left - 32;
        let mut s = detail.as_str().to_string();
        let mut clipped = false;
        while !s.is_empty() && text_width(dc, &s) > max_cx {
            s.pop();
            clipped = true;
        }
        if clipped {
            s.push_str("...");
        }
        text(dc, CARD.left + 16, CARD.top + 66, &s, COL_MUTED);
    }

    let hop_note = match plugin.and_then(|p| p.denoiser.as_ref()) {
        Some(d) => format!(
            "10 MS HOP = {} SMP  |  {:.0} MS LATENCY  |  DEEPFILTERNET3-LL",
            d.hop(),
            d.latency_samples() as f32 / 48.0
        ),
        None => "DEEPFILTERNET3-LL / ONNX RUNTIME".to_string(),
    };
    SelectObject(dc, micro);
    text(dc, CARD.left + 16, CARD.top + 88, &hop_note, COL_MUTED);
}

unsafe fn paint(hwnd: HWND, plugin_ptr: *mut Plugin) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);
    if w <= 0 || h <= 0 {
        let _ = EndPaint(hwnd, &ps);
        return;
    }
    let dc = ensure_mem_dc(hdc, w, h);

    fill(dc, &rc, COL_BG);

    let p = if plugin_ptr.is_null() {
        None
    } else {
        Some(&*plugin_ptr)
    };

    // Title block.
    let title = cached_font(&FONT_TITLE, -22, FW_BOLD.0 as i32, windows::core::w!("Segoe UI"));
    let micro = cached_font(&FONT_MICRO, -12, FW_NORMAL.0 as i32, windows::core::w!("Consolas"));
    SelectObject(dc, title);
    SetBkMode(dc, TRANSPARENT);
    text(dc, MARGIN, 16, "YOUF DENOISE GPU", COL_INK);

    SelectObject(dc, micro);
    text_right(dc, CONTENT_R, 26, "DEEPFILTERNET3-LL  /  48 KHZ", COL_MUTED);

    let rule = RECT {
        left: MARGIN,
        top: HEADER_RULE_Y,
        right: CONTENT_R,
        bottom: HEADER_RULE_Y + 1,
    };
    fill(dc, &rule, COL_LINE);

    for i in 0..NUM_PARAMS {
        let (_, lo, hi) = PARAMS[i];
        let v = p.map(|p| p.param(i)).unwrap_or(lo);
        draw_param(dc, i, v, lo, hi);
    }

    draw_status_card(dc, p);

    // Footer: hairline, credit (whole band is the link), model attribution.
    let frule = RECT {
        left: MARGIN,
        top: FOOTER_RULE_Y,
        right: CONTENT_R,
        bottom: FOOTER_RULE_Y + 1,
    };
    fill(dc, &frule, COL_LINE);

    let footer = cached_font(&FONT_FOOTER, -11, FW_NORMAL.0 as i32, windows::core::w!("Consolas"));
    SelectObject(dc, footer);
    let by = "by Yousef Mohiey - ";
    text(dc, MARGIN, FOOTER_RULE_Y + 12, by, COL_MUTED);
    let bx = text_width(dc, by);
    text(dc, MARGIN + bx, FOOTER_RULE_Y + 12, "github.com/YousefMohiey/VST-Applier", COL_AMBER);
    text(
        dc,
        MARGIN,
        FOOTER_RULE_Y + 30,
        "DEEPFILTERNET3-LL BY HENDRIK SCHROTER (MIT / APACHE-2.0)",
        COL_MUTED,
    );

    let _ = BitBlt(hdc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
    let _ = EndPaint(hwnd, &ps);
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn track_at(y: i32) -> Option<usize> {
    for i in 0..NUM_PARAMS {
        let cy = PARAM_TRACK_Y[i] + TRACK_H / 2;
        if (y - cy).abs() <= HIT_H {
            return Some(i);
        }
    }
    None
}

unsafe fn apply_x(plugin: &mut Plugin, i: usize, x: i32) {
    let (_, lo, hi) = PARAMS[i];
    let f = ((x - MARGIN) as f32 / (CONTENT_R - MARGIN) as f32).clamp(0.0, 1.0);
    plugin.set_param(i, lo + f * (hi - lo));
}

// ---------------------------------------------------------------------------
// Window plumbing
// ---------------------------------------------------------------------------

pub fn ensure_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: cursor,
            lpszClassName: windows::core::w!("VstApplierDenoiseGpuEditor"),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
}

pub unsafe fn create(parent: HWND, plugin: *mut Plugin) -> Option<HWND> {
    ensure_class();
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        windows::core::w!("VstApplierDenoiseGpuEditor"),
        windows::core::w!("Yousef Noise Suppression GPU"),
        WS_CHILD | WS_VISIBLE,
        0,
        0,
        EDITOR_W,
        EDITOR_H,
        parent,
        None,
        instance,
        Some(plugin as *const c_void),
    );
    match hwnd {
        Ok(hwnd) => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, plugin as isize);
            Some(hwnd)
        }
        Err(_) => None,
    }
}

pub unsafe fn close(hwnd: HWND) {
    let _ = DestroyWindow(hwnd);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let plugin_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Plugin;
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd, plugin_ptr);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lp.0 & 0xFFFF) as i16 as i32;
            let y = (lp.0 >> 16) as i16 as i32;
            if y >= FOOTER_RULE_Y + 4 {
                // Credit band: open the author's GitHub page.
                let url = windows::core::HSTRING::from(CREDIT_URL);
                let _ = ShellExecuteW(
                    None,
                    windows::core::w!("open"),
                    PCWSTR(url.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                );
            } else if !plugin_ptr.is_null() {
                if let Some(i) = track_at(y) {
                    if x >= MARGIN - 20 && x <= CONTENT_R + 20 {
                        let p = &mut *plugin_ptr;
                        apply_x(p, i, x);
                        DRAG.with(|d| d.set(i as i32));
                        let _ = SetCapture(hwnd);
                        let _ = InvalidateRect(hwnd, None, false);
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
                    apply_x(&mut *plugin_ptr, active as usize, x);
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
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                let over_slider = track_at(pt.y).is_some();
                let over_credit = pt.y >= FOOTER_RULE_Y + 4;
                if (over_slider || over_credit) && !plugin_ptr.is_null() {
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
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(oldb as *mut c_void));
            }
            let olddc = MEM_DC.with(|c| c.replace(0));
            if olddc != 0 {
                let _ = DeleteDC(HDC(olddc as *mut c_void));
            }
            MEM_SIZE.with(|s| s.set((0, 0)));
            LRESULT(0)
        }
        WM_CLOSE => LRESULT(0), // host owns the close flow
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
