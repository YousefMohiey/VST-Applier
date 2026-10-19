//! Applier Denoise GPU — editor on the shared 640x400 family kit.
//!
//! Same panel as the rest of the family; the tall card below the two
//! sliders reports which execution provider the runtime picked and how
//! the block is being processed.

use windows::Win32::Foundation::{HWND, RECT};

use df_ort::EpKind;

use crate::ui_kit::{
    self, EditorHost, Font, RowSpec, Ui, COL_AMBER, COL_BAD, COL_INK, COL_MUTED,
};
use crate::{Plugin, NUM_PARAMS, PARAMS, P_ATTEN, P_PF};

impl EditorHost for Plugin {
    const CLASS: &'static str = "VstApplierDenoiseGpuEditor";
    const TITLE: &'static str = "APPLIER DENOISE GPU";
    const CAPTION: &'static str = "48 KHZ";
    const ACCENT: u32 = COL_AMBER;
    const ROWS: usize = NUM_PARAMS;

    fn row(i: usize) -> RowSpec {
        RowSpec {
            label: PARAMS[i].0,
            lo: PARAMS[i].1,
            hi: PARAMS[i].2,
            step: match i {
                P_ATTEN => 1.0,
                _ => 0.005,
            },
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
            P_ATTEN => format!("{v:.0} dB"),
            P_PF => {
                if v <= 0.0005 {
                    "OFF".to_string()
                } else {
                    format!("{v:.3}")
                }
            }
            _ => String::new(),
        }
    }

    unsafe fn card(&self, ui: &Ui, r: &RECT) {
        ui.card(r);
        ui.card_caption(r, "INFERENCE");

        let (ep, status, detail) = match self.denoiser.as_ref() {
            Some(d) => (d.ep(), d.status_line(), d.detail_line()),
            None => match self.init_error.as_ref() {
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
        ui.badge(r, ep.short(), color);

        // Keep vendor jargon out of the panel.
        let status = status.replace("DF_ORT_FORCE_CPU", "CPU ONLY");
        ui.text_at(r.left + ui.s(16), r.top + ui.s(42), &status, Font::Value, color);

        if !detail.is_empty() {
            let max_w = r.right - r.left - ui.s(32);
            let mut s = detail.clone();
            let mut clipped = false;
            while !s.is_empty() && ui.width(&s, Font::Micro) > max_w {
                s.pop();
                clipped = true;
            }
            if clipped {
                s.push_str("...");
            }
            ui.text_at(r.left + ui.s(16), r.top + ui.s(70), &s, Font::Micro, COL_MUTED);
        }

        let (hop, latency) = match self.denoiser.as_ref() {
            Some(d) => (d.hop(), d.latency_samples() as f32 / 48.0),
            None => (0, 0.0),
        };
        let note =
            format!("10 MS HOP = {hop} SMP  |  {latency:.0} MS LATENCY  |  APPLIER DENOISE");
        ui.text_at(r.left + ui.s(16), r.top + ui.s(94), &note, Font::Tiny, COL_MUTED);

        // Two stat tiles fill the lower half of the tall card.
        let hop_s = format!("{hop} SMP");
        let lat_s = format!("{latency:.0} MS");
        ui.stat_tiles(r, 120, ("BLOCK HOP", &hop_s), ("LATENCY", &lat_s));
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
                ui_kit::dump_bmp(d, "denoise-gpu", dpi, &px, w, h);
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
