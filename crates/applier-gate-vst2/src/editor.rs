//! Applier Gate — editor on the shared 640x400 family kit.

use windows::Win32::Foundation::{HWND, RECT};

use crate::ui_kit::{self, EditorHost, RowSpec, Ui, COL_ACCENT, COL_AMBER};
use crate::{
    Plugin, NUM_PARAMS, PARAM_STEP, PARAMS, P_ATTACK, P_HOLD, P_RANGE, P_RELEASE, P_THRESHOLD,
};

/// The attenuation meter spans the gate's full range.
const METER_RANGE_DB: f32 = 80.0;

impl EditorHost for Plugin {
    const CLASS: &'static str = "VstApplierGateEditor";
    const TITLE: &'static str = "APPLIER GATE";
    const CAPTION: &'static str = "VST2 / DYNAMICS";
    const ACCENT: u32 = COL_ACCENT;
    const ROWS: usize = NUM_PARAMS;

    fn row(i: usize) -> RowSpec {
        RowSpec {
            label: PARAMS[i].0,
            lo: PARAMS[i].1,
            hi: PARAMS[i].2,
            step: PARAM_STEP[i],
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
            P_THRESHOLD => format!("{v:.1} dB"),
            P_RANGE => format!("{v:.0} dB"),
            P_ATTACK => format!("{v:.1} ms"),
            P_HOLD => format!("{v:.0} ms"),
            P_RELEASE => format!("{v:.0} ms"),
            _ => String::new(),
        }
    }

    unsafe fn card(&self, ui: &Ui, r: &RECT) {
        let att = (-self.meter_db()).clamp(0.0, METER_RANGE_DB);
        let readout = if self.is_open() {
            "OPEN".to_string()
        } else if att < 0.05 {
            "0.0 dB".to_string()
        } else {
            format!("-{att:.1} dB")
        };
        ui.meter_card(r, "ATTENUATION", att / METER_RANGE_DB, &readout, COL_AMBER);
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
                ui_kit::dump_bmp(d, "gate", dpi, &px, w, h);
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
