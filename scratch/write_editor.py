
p = "crates/applier-denoise-vst2/src/lib.rs"
s = open(p, encoding="utf-8").read()

start = 'unsafe extern "system" fn editor_wndproc('
marker = '// ---------------------------------------------------------------------------\n// DLL entry points'
i0 = s.index(start)
i1 = s.index(marker)

new_block = '''/// Editor paint state. Thread-local: the editor lives on the app UI
/// thread only. The memory-DC back buffer and the fonts are created
/// once and reused — painting only ever BLTs one finished frame, so
/// dragging a slider cannot flicker.
thread_local! {
    static MEM_DC: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static MEM_BMP: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static MEM_SIZE: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
    static FONT_MAIN: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_BOLD: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static FONT_MONO: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static DRAG: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
}

const CREDIT_URL: &str = "https://github.com/YousefMohiey";
/// Footer band top; the whole band is the credit link hit area.
const FOOTER_Y: i32 = 160;

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
        let dc = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let _ = SelectObject(dc, bmp);
        MEM_DC.with(|c| c.set(dc.0 as isize));
        MEM_BMP.with(|b| b.set(bmp.0 as isize));
        MEM_SIZE.with(|s| s.set((w, h)));
    }
    HDC(MEM_DC.with(|c| c.get()) as *mut core::ffi::c_void)
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
                for (i, &row) in ROW_Y.iter().enumerate() {
                    if (y - (row + TRACK_H / 2)).abs() <= 14 {
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
                if pt.y >= FOOTER_Y {
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

    let p = if plugin_ptr.is_null() {
        None
    } else {
        Some(&*plugin_ptr)
    };
    if let Some(p) = p {
        // Header: product name + build tag, hairline underneath.
        SelectObject(dc, font_bold);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_INK));
        let title: Vec<u16> = "YOUSEF NOISE SUPPRESSION".encode_utf16().collect();
        let _ = TextOutW(dc, 14, 10, &title);
        SelectObject(dc, font_mono);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_MUTED));
        let tag: Vec<u16> = "DFN3-ll".encode_utf16().collect();
        let mut tag_sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &tag, &mut tag_sz);
        let _ = TextOutW(dc, (w - 14 - tag_sz.cx).max(0), 13, &tag);
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

            let bed = RECT {
                left: TRACK_X0,
                top: row,
                right: TRACK_X1,
                bottom: row + TRACK_H,
            };
            let bed_brush = brush(COL_BED);
            FillRect(dc, &bed, bed_brush);
            DeleteObject(bed_brush);
            let line_brush = brush(COL_LINE);
            FrameRect(dc, &bed, line_brush);

            let frac = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            let fx = TRACK_X0 + ((TRACK_X1 - TRACK_X0) as f32 * frac) as i32;
            let fillr = RECT {
                left: TRACK_X0 + 1,
                top: row + 1,
                right: fx.max(TRACK_X0 + 1),
                bottom: row + TRACK_H - 1,
            };
            let fill_brush = brush(COL_ACCENT);
            FillRect(dc, &fillr, fill_brush);
            DeleteObject(fill_brush);
            let thr = RECT {
                left: fx - 3,
                top: row - 5,
                right: fx + 3,
                bottom: row + TRACK_H + 5,
            };
            let thr_brush = brush(COL_INK);
            FillRect(dc, &thr, thr_brush);
            DeleteObject(thr_brush);
            DeleteObject(line_brush);

            SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_INK));
            let val: Vec<u16> = param_display(i, v).encode_utf16().collect();
            let _ = TextOutW(dc, TRACK_X1 + 10, ty, &val);
        }

        // Footer: hairline + credit line (whole band clickable).
        let frect = RECT { left: 14, top: FOOTER_Y, right: w - 14, bottom: FOOTER_Y + 1 };
        let fb = brush(COL_LINE);
        FillRect(dc, &frect, fb);
        DeleteObject(fb);
        SelectObject(dc, font_mono);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_MUTED));
        let by: Vec<u16> = "by Yousef Mohiey   -   ".encode_utf16().collect();
        let _ = TextOutW(dc, 14, FOOTER_Y + 6, &by);
        let mut by_sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &by, &mut by_sz);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(COL_ACCENT));
        let link: Vec<u16> = "github.com/YousefMohiey".encode_utf16().collect();
        let _ = TextOutW(dc, 14 + by_sz.cx, FOOTER_Y + 6, &link);
    }

    SelectObject(dc, old_font);
    let _ = BitBlt(hdc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
    let _ = EndPaint(hwnd, &ps);
}

'''

s = s[:i0] + new_block + s[i1:]
open(p, "w", encoding="utf-8").write(s)
print("editor rewritten")
