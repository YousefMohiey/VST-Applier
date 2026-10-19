//! Shared editor kit for the bundled Applier VST2 plugins.
//!
//! One panel for the whole family: 640x400 *logical* pixels, a 22 px
//! product title, mono uppercase micro-labels, framed value readouts with
//! up/down steppers, an optional status/meter card and the credit footer.
//!
//! DPI: the panel is laid out in logical px and every coordinate AND font
//! height is multiplied by `GetDpiForWindow()/96` before it reaches GDI,
//! and the back buffer is allocated at the *device* size. That is the
//! whole trick behind crisp text: we never draw a 640x400 frame and let
//! the compositor stretch it — at 150% the panel is really 960x600 and the
//! fonts are really 33 px tall, created with CLEARTYPE_QUALITY.
//!
//! Text fit: every string is drawn through [`Ui::text_in`] / [`Ui::text_box`],
//! which declares the rectangle the string is allowed to occupy. In test
//! builds each draw is recorded (text, real GDI extent, declared box) and
//! [`audit_panel`] asserts the extent stays inside the box at 96/120/144
//! DPI. A caption's box ends 2 px above the control it labels, so the
//! audit also proves captions cannot be overdrawn by their value box.
//!
//! Each plugin crate carries an identical copy of this file (the bundled
//! plugins are independent cdylibs, so there is no shared runtime crate).

#![allow(dead_code)]

use std::cell::Cell;
use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, FrameRect, GetTextExtentPoint32W, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, TextOutW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, FW_BOLD, FW_NORMAL, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, GetDpiForWindow, SetThreadDpiAwarenessContext, SetThreadDpiHostingBehavior,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_HOSTING_BEHAVIOR_MIXED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowLongPtrW,
    KillTimer, LoadCursorW, RegisterClassW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos,
    GWLP_USERDATA, IDC_ARROW, IDC_HAND, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOZORDER,
    SW_SHOWNORMAL, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_DPICHANGED_AFTERPARENT,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_PRINT, WM_PRINTCLIENT,
    WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_CHILD, WS_VISIBLE,
};

// ---------------------------------------------------------------------------
// Logical layout (identical for all seven plugins)
// ---------------------------------------------------------------------------

pub const BASE_W: i32 = 640;
pub const BASE_H: i32 = 400;

pub const MARGIN: i32 = 26;
pub const CONTENT_R: i32 = BASE_W - MARGIN;

pub const TITLE_Y: i32 = 14;
pub const CAPTION_Y: i32 = 22;
pub const RULE_Y: i32 = 52;
/// Top of the column-caption band. The band ends 4 px above the first
/// row's value box (`ROW0 - VALBOX_H/2`), so the caption can never be
/// clipped by the box that follows it.
pub const COLCAP_Y: i32 = 58;

/// Centre line of the first parameter row; rows are 36 px apart. 36 keeps
/// the 28 px readout box from touching its neighbours and still leaves a
/// 40 px status strip under the six-row plugins.
pub const ROW0: i32 = 90;
pub const ROW_PITCH: i32 = 36;

pub const TRACK_X0: i32 = 190;
pub const TRACK_X1: i32 = 470;
pub const TRACK_H: i32 = 14;

pub const BOX_X0: i32 = 484;
pub const BOX_X1: i32 = 588;
pub const STEP_X0: i32 = 594;
pub const STEP_X1: i32 = 622;

/// Framed readout height; centred on the row line.
pub const VALBOX_H: i32 = 28;

/// Bottom edge of the status/meter card.
pub const CARD_BOTTOM: i32 = 344;
/// Footer hairline; the band below it is the credit link hit area.
pub const FOOTER_Y: i32 = 366;

/// The owner's credit. The brand is "Applier"; the author is a person and
/// the footer names him.
pub const FOOTER_TEXT: &str = "by Yousef Mohiey";
/// Muted lead-in, then the name in the plugin accent (the band is a link).
pub const FOOTER_BY: &str = "by ";
pub const FOOTER_NAME: &str = "Yousef Mohiey";

/// Family palette.
pub const COL_BG: u32 = rgb(18, 19, 22);
pub const COL_BED: u32 = rgb(6, 6, 7);
pub const COL_LINE: u32 = rgb(42, 44, 49);
pub const COL_BOX: u32 = rgb(28, 30, 34);
pub const COL_INK: u32 = rgb(242, 241, 238);
pub const COL_MUTED: u32 = rgb(150, 150, 156);
pub const COL_DIM: u32 = rgb(122, 122, 128);
/// Stronger hairline for boxes and steppers.
pub const COL_LINE2: u32 = rgb(60, 62, 68);
pub const COL_AMBER: u32 = rgb(232, 168, 72);
/// The family blue used by the dynamics plugins.
pub const COL_ACCENT: u32 = rgb(45, 129, 255);
pub const COL_BAD: u32 = rgb(255, 98, 89);

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

pub fn row_y(i: usize) -> i32 {
    ROW0 + i as i32 * ROW_PITCH
}

/// Logical rect of the framed readout for row `i`.
pub fn value_box(i: usize) -> (i32, i32, i32, i32) {
    let y = row_y(i);
    (BOX_X0, y - VALBOX_H / 2, BOX_X1, y + VALBOX_H / 2)
}

/// Top of the card below the last row (rows start at `ROW0`).
pub fn card_top(rows: usize) -> i32 {
    row_y(rows.saturating_sub(1)) + 34
}

/// A single parameter row, as the kit needs it.
pub struct RowSpec {
    pub label: &'static str,
    pub lo: f32,
    pub hi: f32,
    /// One click of the up/down stepper.
    pub step: f32,
    /// Fill runs from the track centre instead of the left edge.
    pub bipolar: bool,
}

/// What a plugin's `Plugin` type must expose to reuse the window.
pub trait EditorHost: Sized + 'static {
    /// Registered window class (one per plugin).
    const CLASS: &'static str;
    /// Big product title, e.g. "APPLIER COMPRESSOR".
    const TITLE: &'static str;
    /// Top-right section caption, e.g. "VST2 / DYNAMICS".
    const CAPTION: &'static str;
    /// Slider/meter accent.
    const ACCENT: u32;
    const ROWS: usize;

    fn row(i: usize) -> RowSpec;
    fn value(&self, i: usize) -> f32;
    fn set_value(&mut self, i: usize, v: f32);
    /// Formatted readout for the framed value box.
    fn display(i: usize, v: f32) -> String;
    /// Status / meter card below the rows (kit has already laid it out and
    /// sets the card rect as the drawing bounds while this runs).
    unsafe fn card(&self, ui: &Ui, r: &RECT);
}

// ---------------------------------------------------------------------------
// DPI
// ---------------------------------------------------------------------------

/// Real DPI of `hwnd`, clamped to something sane.
pub unsafe fn dpi_of(hwnd: HWND) -> u32 {
    let d = GetDpiForWindow(hwnd);
    let d = if d == 0 { GetDpiForSystem() } else { d };
    d.clamp(96, 384)
}

pub fn scale_of(dpi: u32) -> f32 {
    dpi as f32 / 96.0
}

/// Device size of the whole panel at `dpi`.
pub fn size_for_dpi(dpi: u32) -> (i32, i32) {
    let s = scale_of(dpi);
    (
        (BASE_W as f32 * s).round() as i32,
        (BASE_H as f32 * s).round() as i32,
    )
}

/// `Some(hwnd)` -> that window's DPI, `None` -> system DPI.
pub unsafe fn dpi_for(editor: Option<HWND>) -> u32 {
    match editor {
        Some(h) => dpi_of(h),
        None => GetDpiForSystem().clamp(96, 384),
    }
}

// ---------------------------------------------------------------------------
// Fonts — one cache per (thread, dpi); recreated when the DPI changes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Title,
    Label,
    Value,
    Micro,
    Tiny,
}

impl Font {
    fn spec(self) -> (i32, i32, PCWSTR) {
        match self {
            Font::Title => (22, FW_BOLD.0 as i32, windows::core::w!("Segoe UI")),
            Font::Label => (13, FW_NORMAL.0 as i32, windows::core::w!("Consolas")),
            Font::Value => (16, FW_NORMAL.0 as i32, windows::core::w!("Segoe UI")),
            Font::Micro => (12, FW_NORMAL.0 as i32, windows::core::w!("Consolas")),
            Font::Tiny => (11, FW_NORMAL.0 as i32, windows::core::w!("Consolas")),
        }
    }

    #[cfg(test)]
    fn name(self) -> &'static str {
        match self {
            Font::Title => "title",
            Font::Label => "label",
            Font::Value => "value",
            Font::Micro => "micro",
            Font::Tiny => "tiny",
        }
    }
}

thread_local! {
    static FONT_DPI: Cell<u32> = const { Cell::new(0) };
    static FONT_TITLE: Cell<isize> = const { Cell::new(0) };
    static FONT_LABEL: Cell<isize> = const { Cell::new(0) };
    static FONT_VALUE: Cell<isize> = const { Cell::new(0) };
    static FONT_MICRO: Cell<isize> = const { Cell::new(0) };
    static FONT_TINY: Cell<isize> = const { Cell::new(0) };
    static MEM_DC: Cell<isize> = const { Cell::new(0) };
    static MEM_BMP: Cell<isize> = const { Cell::new(0) };
    static MEM_SIZE: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    static DRAG: Cell<i32> = const { Cell::new(-1) };
}

unsafe fn delete_fonts() {
    for slot in [
        &FONT_TITLE,
        &FONT_LABEL,
        &FONT_VALUE,
        &FONT_MICRO,
        &FONT_TINY,
    ] {
        let h = slot.with(|c| c.replace(0));
        if h != 0 {
            let _ = DeleteObject(HGDIOBJ(h as *mut c_void));
        }
    }
}

unsafe fn ensure_fonts(dpi: u32) {
    if FONT_DPI.with(|c| c.get()) == dpi {
        return;
    }
    delete_fonts();
    let s = scale_of(dpi);
    let mk = |f: Font| -> isize {
        let (base, weight, face) = f.spec();
        let height = -((base as f32 * s).round() as i32).max(8);
        CreateFontW(
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
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            face,
        )
        .0 as isize
    };
    FONT_TITLE.with(|c| c.set(mk(Font::Title)));
    FONT_LABEL.with(|c| c.set(mk(Font::Label)));
    FONT_VALUE.with(|c| c.set(mk(Font::Value)));
    FONT_MICRO.with(|c| c.set(mk(Font::Micro)));
    FONT_TINY.with(|c| c.set(mk(Font::Tiny)));
    FONT_DPI.with(|c| c.set(dpi));
}

// ---------------------------------------------------------------------------
// Test-only draw trace
// ---------------------------------------------------------------------------

/// One recorded `TextOutW`: what was drawn, its real GDI extent, and the
/// rectangle it was allowed to occupy.
#[cfg(test)]
#[derive(Clone, Debug)]
pub struct TextDraw {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub bounds: RECT,
    pub font: &'static str,
}

#[cfg(test)]
thread_local! {
    static TRACE_ON: Cell<bool> = const { Cell::new(false) };
    static TRACE: std::cell::RefCell<Vec<TextDraw>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn trace_push(text: &str, x: i32, y: i32, w: i32, h: i32, bounds: RECT, font: &'static str) {
    TRACE_ON.with(|on| {
        if on.get() {
            TRACE.with(|t| {
                t.borrow_mut().push(TextDraw {
                    text: text.to_string(),
                    x,
                    y,
                    w,
                    h,
                    bounds,
                    font,
                })
            });
        }
    });
}

#[cfg(test)]
pub fn trace_begin() {
    TRACE.with(|t| t.borrow_mut().clear());
    TRACE_ON.with(|o| o.set(true));
}

#[cfg(test)]
pub fn trace_take() -> Vec<TextDraw> {
    TRACE_ON.with(|o| o.set(false));
    TRACE.with(|t| t.borrow_mut().split_off(0))
}

// ---------------------------------------------------------------------------
// Ui — draws in device pixels from logical coordinates
// ---------------------------------------------------------------------------

pub struct Ui {
    pub dc: HDC,
    pub dpi: u32,
    /// Rect (device px) every undecorated draw must stay inside. The paint
    /// path narrows it for the header, each row cell, the card and the
    /// footer band; per-string boxes are passed to `text_in`/`text_box`.
    bounds: Cell<RECT>,
}

impl Ui {
    pub unsafe fn new(dc: HDC, dpi: u32) -> Self {
        ensure_fonts(dpi);
        let _ = SetBkMode(dc, TRANSPARENT);
        Self {
            dc,
            dpi,
            bounds: Cell::new(RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            }),
        }
    }

    /// Logical px -> device px.
    pub fn s(&self, v: i32) -> i32 {
        (v as f32 * scale_of(self.dpi)).round() as i32
    }

    pub fn rect(&self, l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT {
            left: self.s(l),
            top: self.s(t),
            right: self.s(r),
            bottom: self.s(b),
        }
    }

    /// Replaces the inherited drawing bounds; returns the previous one.
    pub fn set_bounds(&self, r: RECT) -> RECT {
        self.bounds.replace(r)
    }

    pub fn get_bounds(&self) -> RECT {
        self.bounds.get()
    }

    pub unsafe fn font(&self, f: Font) -> HFONT {
        let h = match f {
            Font::Title => FONT_TITLE.with(|c| c.get()),
            Font::Label => FONT_LABEL.with(|c| c.get()),
            Font::Value => FONT_VALUE.with(|c| c.get()),
            Font::Micro => FONT_MICRO.with(|c| c.get()),
            Font::Tiny => FONT_TINY.with(|c| c.get()),
        };
        HFONT(h as *mut c_void)
    }

    pub unsafe fn fill(&self, r: &RECT, color: u32) {
        let b = CreateSolidBrush(COLORREF(color));
        FillRect(self.dc, r, b);
        let _ = DeleteObject(b);
    }

    pub unsafe fn frame(&self, r: &RECT, color: u32) {
        let b = CreateSolidBrush(COLORREF(color));
        FrameRect(self.dc, r, b);
        let _ = DeleteObject(b);
    }

    pub unsafe fn hline(&self, x0: i32, x1: i32, y: i32, color: u32) {
        let r = self.rect(x0, y, x1, y + 1);
        self.fill(&r, color);
    }

    /// Draws text inside `bounds` (device coords) and returns its width.
    pub unsafe fn text_in(
        &self,
        bounds: RECT,
        x: i32,
        y: i32,
        s: &str,
        f: Font,
        color: u32,
    ) -> i32 {
        let prev = self.bounds.replace(bounds);
        let w = self.draw(x, y, s, f, color, bounds);
        self.bounds.set(prev);
        w
    }

    /// Draws text inside a logical-coordinate box.
    pub unsafe fn text_box(
        &self,
        b: (i32, i32, i32, i32),
        x: i32,
        y: i32,
        s: &str,
        f: Font,
        color: u32,
    ) -> i32 {
        let r = self.rect(b.0, b.1, b.2, b.3);
        self.text_in(r, self.s(x), self.s(y), s, f, color)
    }

    /// Draws text (device coords already) inside the inherited bounds and
    /// returns its width in px.
    pub unsafe fn text_at(&self, x: i32, y: i32, s: &str, f: Font, color: u32) -> i32 {
        self.draw(x, y, s, f, color, self.bounds.get())
    }

    unsafe fn draw(&self, x: i32, y: i32, s: &str, f: Font, color: u32, _bounds: RECT) -> i32 {
        SelectObject(self.dc, self.font(f));
        SetTextColor(self.dc, COLORREF(color));
        let u: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(self.dc, &u, &mut sz);
        let _ = TextOutW(self.dc, x, y, &u);
        #[cfg(test)]
        trace_push(s, x, y, sz.cx, sz.cy, _bounds, f.name());
        sz.cx
    }

    /// Logical-coordinate text helper (inherited bounds).
    pub unsafe fn text(&self, x: i32, y: i32, s: &str, f: Font, color: u32) -> i32 {
        self.text_at(self.s(x), self.s(y), s, f, color)
    }

    pub unsafe fn width(&self, s: &str, f: Font) -> i32 {
        SelectObject(self.dc, self.font(f));
        let u: Vec<u16> = s.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(self.dc, &u, &mut sz);
        sz.cx
    }

    /// Height of one line in the given font (device px).
    pub unsafe fn line_h(&self, f: Font) -> i32 {
        SelectObject(self.dc, self.font(f));
        let u: Vec<u16> = "H".encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(self.dc, &u, &mut sz);
        sz.cy
    }

    pub unsafe fn text_right(&self, right: i32, y: i32, s: &str, f: Font, color: u32) -> i32 {
        let w = self.width(s, f);
        self.text_at(right - w, self.s(y), s, f, color);
        w
    }

    /// Up/down chevron made of stacked fills (no glyph font involved).
    pub unsafe fn chevron(&self, cx: i32, cy: i32, up: bool, color: u32) {
        let brush = CreateSolidBrush(COLORREF(color));
        let s = scale_of(self.dpi);
        let rows = (7.0 * s).round().max(3.0) as i32;
        let step = (1.0 * s).round().max(1.0) as i32;
        for k in 0..rows {
            let half = (1.0 * s).round() as i32 + k;
            let y = if up {
                self.s(cy - 3) + k * step
            } else {
                self.s(cy + 3) - k * step
            };
            let r = RECT {
                left: self.s(cx) - half,
                top: y,
                right: self.s(cx) + half + 1,
                bottom: y + step,
            };
            FillRect(self.dc, &r, brush);
        }
        let _ = DeleteObject(brush);
    }

    // -- card primitives ----------------------------------------------------

    /// Framed panel used by every status / meter card.
    pub unsafe fn card(&self, r: &RECT) {
        self.fill(r, COL_BOX);
        self.frame(r, COL_LINE);
    }

    pub unsafe fn card_caption(&self, r: &RECT, s: &str) {
        let b = RECT {
            left: r.left + self.s(16),
            top: r.top + self.s(8),
            right: r.right - self.s(120),
            bottom: r.top + self.s(30),
        };
        self.text_in(
            b,
            r.left + self.s(16),
            r.top + self.s(12),
            s,
            Font::Micro,
            COL_MUTED,
        );
    }

    /// Coloured border + label, anchored to the card's top-right corner.
    pub unsafe fn badge(&self, r: &RECT, label: &str, color: u32) {
        let tw = self.width(label, Font::Micro);
        let bw = tw + self.s(20);
        let h = self.s(20);
        let b = RECT {
            left: r.right - self.s(16) - bw,
            top: r.top + self.s(12),
            right: r.right - self.s(16),
            bottom: r.top + self.s(12) + h,
        };
        self.fill(&b, COL_BED);
        self.frame(&b, color);
        let inner = RECT {
            left: b.left + self.s(2),
            top: b.top + self.s(2),
            right: b.right - self.s(2),
            bottom: b.bottom - self.s(1),
        };
        self.text_in(
            inner,
            b.left + (bw - tw) / 2,
            b.top + self.s(3),
            label,
            Font::Micro,
            color,
        );
    }

    /// Recessed meter bed; `frac` in 0..1 fills it from the left.
    pub unsafe fn bar(&self, x0: i32, x1: i32, y: i32, h: i32, frac: f32, color: u32) {
        let bed = self.rect(x0, y, x1, y + h);
        self.fill(&bed, COL_BED);
        self.frame(&bed, COL_LINE2);
        let inner = (x1 - x0 - 2) as f32;
        let fw = (inner * frac.clamp(0.0, 1.0)) as i32;
        if fw > 1 {
            let f = self.rect(x0 + 1, y + 1, x0 + 1 + fw, y + h - 1);
            self.fill(&f, color);
        }
    }

    /// Divider plus two caption/value stat tiles across the card width.
    /// `dy` is the divider's offset from the card top; the tiles need
    /// `dy + 8 + line(Tiny) + 4 + line(Value) + 4` px of card height.
    pub unsafe fn stat_tiles(&self, r: &RECT, dy: i32, a: (&str, &str), b: (&str, &str)) {
        let y = r.top + self.s(dy);
        let line = RECT {
            left: r.left + self.s(16),
            top: y,
            right: r.right - self.s(16),
            bottom: y + 1,
        };
        self.fill(&line, COL_LINE);
        let mid = (r.left + r.right) / 2;
        let cap_y = y + self.s(8);
        let ch = self.line_h(Font::Tiny);
        let val_y = cap_y + ch + self.s(4);
        let vh = self.line_h(Font::Value);
        let v = RECT {
            left: mid,
            top: cap_y - self.s(2),
            right: mid + 1,
            bottom: (val_y + vh + self.s(4)).min(r.bottom - self.s(4)),
        };
        self.fill(&v, COL_LINE);
        for (x0, (cap, val)) in [(r.left + self.s(16), a), (mid + self.s(16), b)] {
            let w = (mid - r.left) - self.s(32);
            let cap_box = RECT {
                left: x0,
                top: cap_y - self.s(2),
                right: x0 + w,
                bottom: val_y - self.s(2),
            };
            let val_box = RECT {
                left: x0,
                top: val_y,
                right: x0 + w,
                bottom: (val_y + vh + self.s(3)).min(r.bottom - self.s(2)),
            };
            self.text_in(cap_box, x0, cap_y, cap, Font::Tiny, COL_DIM);
            self.text_in(val_box, x0, val_y, val, Font::Value, COL_INK);
        }
    }

    /// The standard family meter card. Three densities by card height; the
    /// readout band is sized from the real GDI line height (≈1.4× the font
    /// pixel size), which is why the strips never clip their value text.
    pub unsafe fn meter_card(&self, r: &RECT, label: &str, frac: f32, readout: &str, color: u32) {
        self.card(r);
        let h = r.bottom - r.top;
        let lh = self.line_h(Font::Value);
        let rw = self.width(readout, Font::Value);
        if h >= self.s(120) {
            self.card_caption(r, label);
            let top = r.top + self.s(34);
            let band = RECT {
                left: r.left + self.s(16),
                top,
                right: r.right - self.s(16),
                bottom: top + lh + self.s(4),
            };
            self.text_in(
                band,
                r.right - self.s(16) - rw,
                top + self.s(2),
                readout,
                Font::Value,
                COL_INK,
            );
            self.bar(
                r.left + self.s(16),
                r.right - self.s(16),
                r.bottom - self.s(30),
                self.s(10),
                frac,
                color,
            );
        } else if h >= self.s(54) {
            self.card_caption(r, label);
            let top = r.top + self.s(4);
            let band = RECT {
                left: r.right - self.s(180),
                top,
                right: r.right - self.s(16),
                bottom: top + lh + self.s(4),
            };
            self.text_in(
                band,
                r.right - self.s(16) - rw,
                top + self.s(2),
                readout,
                Font::Value,
                COL_INK,
            );
            self.bar(
                r.left + self.s(16),
                r.right - self.s(16),
                r.bottom - self.s(18),
                self.s(8),
                frac,
                color,
            );
        } else {
            // Compact strip: caption left, readout right, bar underneath.
            let bar_top = r.bottom - self.s(11);
            let top = r.top + self.s(2);
            let band = RECT {
                left: r.left + self.s(14),
                top,
                right: r.right - self.s(14),
                bottom: bar_top - self.s(2),
            };
            let cap_box = RECT {
                right: band.left + self.s(144),
                ..band
            };
            self.text_in(
                cap_box,
                band.left,
                top + (lh - self.line_h(Font::Micro)) / 2,
                label,
                Font::Micro,
                COL_MUTED,
            );
            let val_box = RECT {
                left: band.right - self.s(150),
                ..band
            };
            self.text_in(
                val_box,
                band.right - rw,
                top + self.s(2),
                readout,
                Font::Value,
                COL_INK,
            );
            self.bar(
                r.left + self.s(14),
                r.right - self.s(14),
                bar_top,
                self.s(8),
                frac,
                color,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Back buffer
// ---------------------------------------------------------------------------

unsafe fn ensure_mem_dc(hdc: HDC, w: i32, h: i32) -> HDC {
    let reuse = MEM_DC.with(|c| c.get() != 0) && MEM_SIZE.with(|s| s.get() == (w, h));
    if !reuse {
        if MEM_DC.with(|c| c.get()) != 0 {
            let oldb = MEM_BMP.with(|b| b.replace(0));
            if oldb != 0 {
                let _ = DeleteObject(HGDIOBJ(oldb as *mut c_void));
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

unsafe fn free_mem_dc() {
    let b = MEM_BMP.with(|x| x.replace(0));
    if b != 0 {
        let _ = DeleteObject(HGDIOBJ(b as *mut c_void));
    }
    let d = MEM_DC.with(|x| x.replace(0));
    if d != 0 {
        let _ = DeleteDC(HDC(d as *mut c_void));
    }
    MEM_SIZE.with(|s| s.set((0, 0)));
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

/// Renders the whole panel into `dc` at `dpi`. Used by WM_PAINT's back
/// buffer and by WM_PRINT/WM_PRINTCLIENT (so PrintWindow shows the panel).
pub unsafe fn paint_into<T: EditorHost>(dc: HDC, w: i32, h: i32, dpi: u32, plugin: *const T) {
    let ui = Ui::new(dc, dpi);
    let full = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    ui.set_bounds(full);
    ui.fill(&full, COL_BG);

    // Header: the title owns everything left of the section caption.
    let cap_w = ui.width(T::CAPTION, Font::Micro);
    let cap_left = ui.s(CONTENT_R) - cap_w;
    let title_box = RECT {
        left: ui.s(MARGIN),
        top: ui.s(TITLE_Y - 2),
        right: cap_left - ui.s(16),
        bottom: ui.s(RULE_Y - 6),
    };
    ui.text_in(
        title_box,
        ui.s(MARGIN),
        ui.s(TITLE_Y),
        T::TITLE,
        Font::Title,
        COL_INK,
    );
    let cap_box = RECT {
        left: cap_left - ui.s(4),
        top: ui.s(CAPTION_Y - 2),
        right: ui.s(CONTENT_R),
        bottom: ui.s(CAPTION_Y + 16),
    };
    ui.text_in(
        cap_box,
        cap_left,
        ui.s(CAPTION_Y),
        T::CAPTION,
        Font::Micro,
        COL_DIM,
    );
    ui.hline(MARGIN, CONTENT_R, RULE_Y, COL_LINE);

    // Column captions. Their band stops 2 px above the first value box, so
    // a caption can never be covered by the box border/fill below it.
    let cap_band_bottom = row_y(0) - VALBOX_H / 2 - 2;
    ui.text_box(
        (MARGIN, RULE_Y + 4, BOX_X0 - 12, cap_band_bottom),
        MARGIN,
        COLCAP_Y,
        "PARAMETER",
        Font::Tiny,
        COL_DIM,
    );
    ui.text_box(
        (BOX_X0, RULE_Y + 4, STEP_X1, cap_band_bottom),
        BOX_X0,
        COLCAP_Y,
        "VALUE",
        Font::Tiny,
        COL_DIM,
    );

    let p = if plugin.is_null() {
        None
    } else {
        Some(&*plugin)
    };

    if let Some(p) = p {
        for i in 0..T::ROWS {
            let spec = T::row(i);
            let v = p.value(i);
            let y = row_y(i);

            let label_box = RECT {
                left: ui.s(MARGIN),
                top: ui.s(y - 13),
                right: ui.s(TRACK_X0 - 10),
                bottom: ui.s(y + 6),
            };
            ui.text_in(
                label_box,
                ui.s(MARGIN),
                ui.s(y - 9),
                &spec.label.to_uppercase(),
                Font::Label,
                COL_MUTED,
            );

            let frac = ((v - spec.lo) / (spec.hi - spec.lo)).clamp(0.0, 1.0);

            // Track: recessed bed, hairline frame, accent fill, ink thumb.
            let bed = ui.rect(TRACK_X0, y - TRACK_H / 2, TRACK_X1, y + TRACK_H / 2);
            ui.fill(&bed, COL_BED);
            ui.frame(&bed, COL_LINE);
            let span = ui.s(TRACK_X1) - ui.s(TRACK_X0);
            let fx = ui.s(TRACK_X0) + (span as f32 * frac) as i32;
            let (f0, f1) = if spec.bipolar {
                let mid = ui.s(TRACK_X0) + (ui.s(TRACK_X1) - ui.s(TRACK_X0)) / 2;
                (fx.min(mid), fx.max(mid))
            } else {
                (ui.s(TRACK_X0), fx)
            };
            if f1 > f0 + 1 {
                let f = RECT {
                    left: f0 + 1,
                    top: bed.top + 1,
                    right: f1,
                    bottom: bed.bottom - 1,
                };
                ui.fill(&f, T::ACCENT);
            }
            let thumb = RECT {
                left: (fx - ui.s(4)).max(bed.left),
                top: bed.top - ui.s(3),
                right: (fx + ui.s(5)).min(bed.right),
                bottom: bed.bottom + ui.s(3),
            };
            ui.fill(&thumb, COL_INK);

            // Framed readout; the text box is the box interior.
            let (bl, bt, br, bb) = value_box(i);
            let valbox = ui.rect(bl, bt, br, bb);
            ui.fill(&valbox, COL_BOX);
            ui.frame(&valbox, COL_LINE2);
            let txt = T::display(i, v);
            let tw = ui.width(&txt, Font::Value);
            let lh = ui.line_h(Font::Value);
            let inner = RECT {
                left: valbox.left + ui.s(2),
                top: valbox.top + ui.s(2),
                right: valbox.right - ui.s(2),
                bottom: valbox.bottom - ui.s(2),
            };
            ui.text_in(
                inner,
                valbox.left + ((valbox.right - valbox.left) - tw) / 2,
                valbox.top + ((valbox.bottom - valbox.top) - lh) / 2,
                &txt,
                Font::Value,
                COL_INK,
            );

            // Steppers; the whole 28 px box is the hit area.
            let up = ui.rect(STEP_X0, y - 16, STEP_X1, y - 2);
            let dn = ui.rect(STEP_X0, y + 2, STEP_X1, y + 16);
            for b in [up, dn] {
                ui.fill(&b, COL_BOX);
                ui.frame(&b, COL_LINE2);
            }
            let cx = (STEP_X0 + STEP_X1) / 2;
            ui.chevron(cx, y - 9, true, COL_MUTED);
            ui.chevron(cx, y + 9, false, COL_MUTED);
        }

        // Status / meter card. Everything the plugin draws here is bounded
        // by the card rect.
        let card = ui.rect(MARGIN, card_top(T::ROWS), CONTENT_R, CARD_BOTTOM);
        let prev = ui.set_bounds(card);
        p.card(&ui, &card);
        ui.set_bounds(prev);
    }

    // Footer: hairline + credit band (the whole band is the link).
    ui.hline(MARGIN, CONTENT_R, FOOTER_Y, COL_LINE);
    let band = RECT {
        left: ui.s(MARGIN),
        top: ui.s(FOOTER_Y + 3),
        right: ui.s(CONTENT_R),
        bottom: ui.s(BASE_H - 2),
    };
    let mut x = ui.s(MARGIN);
    x += ui.text_in(
        band,
        x,
        ui.s(FOOTER_Y + 8),
        FOOTER_BY,
        Font::Tiny,
        COL_MUTED,
    );
    ui.text_in(
        band,
        x,
        ui.s(FOOTER_Y + 8),
        FOOTER_NAME,
        Font::Tiny,
        T::ACCENT,
    );
}

unsafe fn paint<T: EditorHost>(hwnd: HWND, plugin: *const T) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);
    if w > 0 && h > 0 {
        let dpi = dpi_of(hwnd);
        let dc = ensure_mem_dc(hdc, w, h);
        paint_into::<T>(dc, w, h, dpi, plugin);
        let _ = BitBlt(hdc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
    }
    let _ = EndPaint(hwnd, &ps);
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn stepper_at(dpi: u32, x: i32, y: i32, rows: usize) -> Option<(usize, f32)> {
    let s = scale_of(dpi);
    let lx = (x as f32 / s).round() as i32;
    let ly = (y as f32 / s).round() as i32;
    if !(STEP_X0..STEP_X1).contains(&lx) {
        return None;
    }
    for i in 0..rows {
        let row = row_y(i);
        if (row - 16..=row - 2).contains(&ly) {
            return Some((i, 1.0));
        }
        if (row + 2..=row + 16).contains(&ly) {
            return Some((i, -1.0));
        }
    }
    None
}

fn track_at(dpi: u32, x: i32, y: i32, rows: usize) -> Option<usize> {
    let s = scale_of(dpi);
    let lx = (x as f32 / s).round() as i32;
    let ly = (y as f32 / s).round() as i32;
    if !(TRACK_X0 - 8..BOX_X0).contains(&lx) {
        return None;
    }
    (0..rows).find(|&i| (ly - row_y(i)).abs() <= 16)
}

unsafe fn apply_x<T: EditorHost>(p: &mut T, i: usize, x: i32, dpi: u32) {
    let s = scale_of(dpi);
    let lx = (x as f32 / s).round() as i32;
    let spec = T::row(i);
    let f = ((lx - TRACK_X0) as f32 / (TRACK_X1 - TRACK_X0) as f32).clamp(0.0, 1.0);
    p.set_value(i, spec.lo + f * (spec.hi - spec.lo));
}

// ---------------------------------------------------------------------------
// Window plumbing
// ---------------------------------------------------------------------------

fn ensure_class<T: EditorHost>() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let name: Vec<u16> = T::CLASS.encode_utf16().chain(std::iter::once(0)).collect();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc::<T>),
            hInstance: instance.into(),
            hCursor: cursor,
            lpszClassName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
}

pub unsafe fn create<T: EditorHost>(parent: HWND, plugin: *mut T) -> Option<HWND> {
    ensure_class::<T>();
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let class: Vec<u16> = T::CLASS.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = T::TITLE.encode_utf16().chain(std::iter::once(0)).collect();

    // Native DPI even inside a DPI-unaware host. The thread context is
    // what GetDpiForWindow and CreateWindowExW consult; mixed hosting is
    // what lets a native-DPI child live inside an unaware parent window.
    // Without this pair Windows virtualises the DPI to 96 and the whole
    // panel is bitmap-stretched -> the blurry/small text users report.
    let prev_ctx = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let prev_host = SetThreadDpiHostingBehavior(DPI_HOSTING_BEHAVIOR_MIXED);

    let created = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR(class.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_CHILD | WS_VISIBLE,
        0,
        0,
        BASE_W,
        BASE_H,
        parent,
        None,
        instance,
        Some(plugin as *const c_void),
    );

    let hwnd = match created {
        Ok(hwnd) => hwnd,
        Err(_) => {
            let _ = SetThreadDpiHostingBehavior(prev_host);
            let _ = SetThreadDpiAwarenessContext(prev_ctx);
            return None;
        }
    };

    // Now that the window exists in an aware context, ask it for the real
    // DPI and resize to the native size (640x400 at 96, 800x500 at 120).
    let (w, h) = size_for_dpi(dpi_of(hwnd));
    if let (Ok(w), Ok(h)) = (u32::try_from(w), u32::try_from(h)) {
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            w as i32,
            h as i32,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        );
    }

    let _ = SetThreadDpiHostingBehavior(prev_host);
    let _ = SetThreadDpiAwarenessContext(prev_ctx);

    SetWindowLongPtrW(hwnd, GWLP_USERDATA, plugin as isize);
    SetTimer(hwnd, METER_TIMER_ID, 33, None);
    Some(hwnd)
}

pub unsafe fn close(hwnd: HWND) {
    let _ = DestroyWindow(hwnd);
}

const METER_TIMER_ID: usize = 1;

unsafe fn open_credit() {
    let url = windows::core::HSTRING::from("https://github.com/YousefMohiey/VST-Applier");
    let _ = ShellExecuteW(
        None,
        windows::core::w!("open"),
        PCWSTR(url.as_ptr()),
        PCWSTR::null(),
        PCWSTR::null(),
        SW_SHOWNORMAL,
    );
}

unsafe extern "system" fn wndproc<T: EditorHost>(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    let plugin = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T;
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint::<T>(hwnd, plugin as *const T);
            LRESULT(0)
        }
        // PrintWindow / some hosts ask through WM_PRINT.
        WM_PRINT | WM_PRINTCLIENT => {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            if rc.right > 0 && rc.bottom > 0 {
                paint_into::<T>(
                    HDC(wp.0 as *mut c_void),
                    rc.right,
                    rc.bottom,
                    dpi_of(hwnd),
                    plugin as *const T,
                );
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wp.0 == METER_TIMER_ID {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        // A child window does not get WM_DPICHANGED when its host moves
        // monitors; it gets WM_DPICHANGED_AFTERPARENT instead. Handle
        // both so the panel re-lays-out at the new native size.
        WM_DPICHANGED | WM_DPICHANGED_AFTERPARENT => {
            let dpi = if msg == WM_DPICHANGED {
                ((wp.0 >> 16) & 0xFFFF) as u32
            } else {
                dpi_of(hwnd)
            };
            let (w, h) = size_for_dpi(if dpi == 0 { 96 } else { dpi });
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            if rc.right != w || rc.bottom != h {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
                );
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lp.0 & 0xFFFF) as i16 as i32;
            let y = (lp.0 >> 16) as i16 as i32;
            let dpi = dpi_of(hwnd);
            if y >= window_s(FOOTER_Y, dpi) {
                open_credit();
            } else if !plugin.is_null() {
                if let Some((i, dir)) = stepper_at(dpi, x, y, T::ROWS) {
                    let p = &mut *plugin;
                    p.set_value(i, p.value(i) + dir * T::row(i).step);
                    let _ = InvalidateRect(hwnd, None, false);
                } else if let Some(i) = track_at(dpi, x, y, T::ROWS) {
                    apply_x(&mut *plugin, i, x, dpi);
                    DRAG.with(|d| d.set(i as i32));
                    let _ = SetCapture(hwnd);
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !plugin.is_null() && wp.0 & 1 != 0 {
                let active = DRAG.with(|d| d.get());
                if active >= 0 {
                    let x = (lp.0 & 0xFFFF) as i16 as i32;
                    apply_x(&mut *plugin, active as usize, x, dpi_of(hwnd));
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
                let dpi = dpi_of(hwnd);
                let hand = pt.y >= window_s(FOOTER_Y, dpi)
                    || stepper_at(dpi, pt.x, pt.y, T::ROWS).is_some()
                    || track_at(dpi, pt.x, pt.y, T::ROWS).is_some();
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
            DRAG.with(|d| d.set(-1));
            free_mem_dc();
            LRESULT(0)
        }
        WM_CLOSE => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn window_s(v: i32, dpi: u32) -> i32 {
    (v as f32 * scale_of(dpi)).round() as i32
}

// ---------------------------------------------------------------------------
// Test-only: render a panel at an arbitrary DPI into a DIB, no window needed
// ---------------------------------------------------------------------------

/// Renders `plugin`'s panel at `dpi` and returns (top-down BGRA, w, h).
/// This is the exact path WM_PAINT uses, so a 120-DPI render here is the
/// same pixel work a 120-DPI window would do.
#[cfg(test)]
pub unsafe fn render_probe<T: EditorHost>(plugin: &T, dpi: u32) -> (Vec<u8>, i32, i32) {
    use windows::Win32::Graphics::Gdi::{
        CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    let (w, h) = size_for_dpi(dpi);
    let screen = CreateCompatibleDC(None);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // top-down rows
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;
    let mut bits: *mut c_void = std::ptr::null_mut();
    let dib = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        .expect("CreateDIBSection");
    let old = SelectObject(screen, dib);
    paint_into::<T>(screen, w, h, dpi, plugin as *const T);
    let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
    SelectObject(screen, old);
    let _ = DeleteObject(dib);
    let _ = DeleteDC(screen);
    (px, w, h)
}

/// Renders at `dpi`, then returns every text-fit failure for that render.
/// A failure is a string whose real GDI extent leaves its declared box
/// (2 px tolerance for antialias overhang), a string outside the panel, two
/// strings overlapping, or the VALUE caption intruding into row 0's box.
#[cfg(test)]
pub unsafe fn audit_panel<T: EditorHost>(
    plugin: &T,
    dpi: u32,
) -> (Vec<String>, Vec<TextDraw>, i32, i32) {
    trace_begin();
    let (_px, w, h) = render_probe(plugin, dpi);
    let draws = trace_take();
    let tol = 2;
    let mut fails: Vec<String> = Vec::new();

    for d in &draws {
        if d.x < 0 || d.y < 0 || d.x + d.w > w + tol || d.y + d.h > h + tol {
            fails.push(format!(
                "dpi {dpi}: '{}' [{}] leaves the panel: {}x{} at {},{} (panel {w}x{h})",
                d.text, d.font, d.w, d.h, d.x, d.y
            ));
        }
        if d.x < d.bounds.left - tol || d.x + d.w > d.bounds.right + tol {
            fails.push(format!(
                "dpi {dpi}: '{}' [{}] horizontally {}..{} outside its box {}..{}",
                d.text,
                d.font,
                d.x,
                d.x + d.w,
                d.bounds.left,
                d.bounds.right
            ));
        }
        if d.y < d.bounds.top - tol || d.y + d.h > d.bounds.bottom + tol {
            fails.push(format!(
                "dpi {dpi}: '{}' [{}] vertically {}..{} outside its box {}..{}",
                d.text,
                d.font,
                d.y,
                d.y + d.h,
                d.bounds.top,
                d.bounds.bottom
            ));
        }
    }

    for i in 0..draws.len() {
        for j in (i + 1)..draws.len() {
            let (a, b) = (&draws[i], &draws[j]);
            let ox = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
            let oy = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
            if ox > 1 && oy > 1 {
                fails.push(format!(
                    "dpi {dpi}: '{}' overlaps '{}' by {ox}x{oy} px",
                    a.text, b.text
                ));
            }
        }
    }

    let s = scale_of(dpi);
    let vb_top = (value_box(0).1 as f32 * s).round() as i32;
    for d in draws.iter().filter(|d| d.text == "VALUE") {
        if d.y + d.h > vb_top {
            fails.push(format!(
                "dpi {dpi}: VALUE caption bottom {} is inside row 0's value box (top {vb_top})",
                d.y + d.h
            ));
        }
    }

    (fails, draws, w, h)
}

/// The footer credit as actually painted, in draw order.
#[cfg(test)]
pub fn footer_text(draws: &[TextDraw], dpi: u32) -> String {
    let s = scale_of(dpi);
    let top = (FOOTER_Y as f32 * s).round() as i32;
    let mut v: Vec<&TextDraw> = draws.iter().filter(|d| d.y >= top).collect();
    v.sort_by_key(|d| d.x);
    v.into_iter()
        .map(|d| d.text.as_str())
        .collect::<String>()
        .trim()
        .to_string()
}

/// (ink pixels, x0, y0, x1, y1) over pixels brighter than the panel lines.
#[cfg(test)]
pub fn ink_stats(px: &[u8], _w: i32, _h: i32) -> (usize, usize, usize, usize, usize) {
    let (mut n, mut x0, mut y0, mut x1, mut y1) = (0usize, usize::MAX, usize::MAX, 0usize, 0usize);
    // We do not know the row pitch here, so callers pass a tightly packed
    // buffer produced by `render_probe`.
    for (i, p) in px.chunks_exact(4).enumerate() {
        if p[0].max(p[1]).max(p[2]) > 60 {
            n += 1;
            let (x, y) = (i % _w as usize, i / _w as usize);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    (n, x0, y0, x1, y1)
}

/// Writes a probe render as a 32-bit top-down BMP (Pillow converts it).
#[cfg(test)]
pub fn dump_bmp(dir: &std::path::Path, name: &str, dpi: u32, px: &[u8], w: i32, h: i32) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{name}-{dpi}dpi.bmp"));
    let image_size = (w * h * 4) as u32;
    let mut out = Vec::with_capacity(54 + image_size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + image_size).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&(-h).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(px);
    if let Ok(mut f) = std::fs::File::create(path) {
        let _ = f.write_all(&out);
    }
}
