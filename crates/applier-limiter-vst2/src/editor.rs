//! Applier Limiter — editor on the shared 640x400 family kit.

use windows::Win32::Foundation::{HWND, RECT};

use crate::ui_kit::{self, EditorHost, Font, RowSpec, Ui, COL_ACCENT, COL_DIM, COL_INK, COL_LINE, COL_LINE2, COL_MUTED};
use crate::{Plugin, NUM_PARAMS, PARAMS, P_RELEASE, P_THRESHOLD};

impl EditorHost for Plugin {
    const CLASS: &'static str = "VstApplierLimiterEditor";
    const TITLE: &'static str = "APPLIER LIMITER";
    const CAPTION: &'static str = "VST2 / DYNAMICS";
    const ACCENT: u32 = COL_ACCENT;
    const ROWS: usize = NUM_PARAMS;

    fn row(i: usize) -> RowSpec {
        RowSpec {
            label: PARAMS[i].0,
            lo: PARAMS[i].1,
            hi: PARAMS[i].2,
            step: if i == P_RELEASE { 5.0 } else { 0.5 },
            bipolar: false,
        }
    }

    fn value(&self, i: usize) -> f32 {
        self.param(i)
    }

    fn set_value(&mut self, i: usize, v: f32) {
        self.set_param(i, v);
    }

    fn display(i: usize, v: f32) -> String {
        match i {
            P_THRESHOLD => format!("{v:.2} dB"),
            P_RELEASE => format!("{v:.0} ms"),
            _ => String::new(),
        }
    }

    unsafe fn card(&self, ui: &Ui, r: &RECT) {
        ui.card(r);
        ui.card_caption(r, "SIGNAL PATH");
        ui.text_at(
            r.left + ui.s(16),
            r.top + ui.s(38),
            "STEREO-LINKED PEAK",
            Font::Value,
            COL_INK,
        );
        ui.text_at(
            r.left + ui.s(16),
            r.top + ui.s(66),
            "INSTANT ATTACK  ·  EXPONENTIAL RELEASE",
            Font::Micro,
            COL_MUTED,
        );
        ui.text_at(
            r.left + ui.s(16),
            r.top + ui.s(88),
            "ZERO LATENCY  ·  NO LOOKAHEAD",
            Font::Tiny,
            COL_DIM,
        );

        // Signal-flow strip: fills the lower half of the tall card and
        // says what the two knobs actually sit between.
        let y = r.top + ui.s(104);
        let line = RECT {
            left: r.left + ui.s(16),
            top: y,
            right: r.right - ui.s(16),
            bottom: y + 1,
        };
        ui.fill(&line, COL_LINE);
        let bw = ui.s(150);
        let gap = ui.s(22);
        let top = y + ui.s(14);
        let bottom = top + ui.s(44);
        let strip = 3 * bw + 2 * gap;
        let mut x = r.left + ((r.right - r.left) - strip) / 2;
        for (label, sub) in [("INPUT", "L / R"), ("PEAK DETECT", "STEREO"), ("GAIN", "SMOOTH")] {
            let b = RECT {
                left: x,
                top,
                right: x + bw,
                bottom,
            };
            ui.fill(&b, crate::ui_kit::COL_BED);
            ui.frame(&b, crate::ui_kit::COL_LINE2);
            ui.text_at(x + ui.s(10), top + ui.s(8), label, Font::Micro, COL_MUTED);
            ui.text_at(x + ui.s(10), top + ui.s(24), sub, Font::Tiny, COL_DIM);
            // Connector + arrowhead to the next stage.
            if label != "GAIN" {
                let mid = (top + bottom) / 2;
                let a = RECT {
                    left: x + bw,
                    top: mid,
                    right: x + bw + gap,
                    bottom: mid + 1,
                };
                ui.fill(&a, COL_LINE2);
                let tip = RECT {
                    left: x + bw + gap - ui.s(5),
                    top: mid - ui.s(3),
                    right: x + bw + gap - ui.s(2),
                    bottom: mid + ui.s(4),
                };
                ui.fill(&tip, COL_LINE2);
            }
            x += bw + gap;
        }
    }
}

pub unsafe fn create(parent: HWND, plugin: *mut Plugin) -> Option<HWND> {
    ui_kit::create::<Plugin>(parent, plugin)
}

pub unsafe fn close(hwnd: HWND) {
    ui_kit::close(hwnd);
}

/// Refresh the editor rect the host asked for at its current DPI.
pub fn sync_rect(p: &mut Plugin) {
    let (w, h) = unsafe { ui_kit::size_for_dpi(ui_kit::dpi_for(p.editor)) };
    p.rect.right = w as i16;
    p.rect.bottom = h as i16;
}

#[cfg(test)]
mod dpi_tests {
    use super::*;

    /// The panel must be laid out natively at every scaling level: same
    /// kit, real pixel geometry, no fixed-size frame being stretched.
    #[test]
    fn panel_renders_native_at_96_120_144() {
        let p = Plugin::new();
        let dir = std::env::var_os("VA_UI_SHOT_DIR").map(std::path::PathBuf::from);
        let mut dims = Vec::new();
        let mut boxes = Vec::new();
        for dpi in [96u32, 120, 144] {
            let (px, w, h) = unsafe { ui_kit::render_probe(&p, dpi) };
            dims.push((w, h));
            let (ink, x0, y0, x1, y1) = ui_kit::ink_stats(&px, w, h);
            assert!(ink > 3000, "dpi {dpi}: panel looks blank ({ink} ink px)");
            assert!(x0 > 0 && y0 > 0, "dpi {dpi}: ink at the top-left edge");
            assert!(
                x1 < (w - 1) as usize && y1 < (h - 1) as usize,
                "dpi {dpi}: ink at the bottom-right edge"
            );
            if let Some(d) = dir.as_ref() {
                ui_kit::dump_bmp(d, "limiter", dpi, &px, w, h);
            }
            boxes.push((x1 - x0, y1 - y0));
        }
        assert_eq!(dims, vec![(640, 400), (800, 500), (960, 600)]);
        // More than rounding: the ink box grows with the DPI, so the
        // fonts and the layout were really scaled, not stretched.
        assert!(
            boxes[1].0 * 100 >= boxes[0].0 * 118,
            "120 dpi ink {:?} vs 96 {:?}",
            boxes[1],
            boxes[0]
        );
    }

    /// Every string in the paint path must fit the box the layout gave it,
    /// at every scaling level, and the credit must name the author.
    #[test]
    fn every_string_fits_its_box_at_96_120_144() {
        assert_eq!(ui_kit::FOOTER_TEXT, "by Yousef Mohiey");
        assert_eq!(
            format!("{}{}", ui_kit::FOOTER_BY, ui_kit::FOOTER_NAME),
            ui_kit::FOOTER_TEXT
        );
        let p = Plugin::new();
        for dpi in [96u32, 120, 144] {
            let (fails, draws, _w, _h) = unsafe { ui_kit::audit_panel(&p, dpi) };
            assert!(
                fails.is_empty(),
                "dpi {dpi}: {} text/layout fit failures:\n{}",
                fails.len(),
                fails.join("\n")
            );
            assert!(draws.len() >= 8, "dpi {dpi}: only {} strings traced", draws.len());
            assert_eq!(
                ui_kit::footer_text(&draws, dpi),
                ui_kit::FOOTER_TEXT,
                "dpi {dpi}: footer credit"
            );
        }
    }
}
