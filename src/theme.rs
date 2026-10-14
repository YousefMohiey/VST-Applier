//! Design tokens (storytold/craft-study lesson: widgets never hard-code
//! colors — everything reads these). Our palette: near-black charcoal
//! with navy undertones, teal primary, coral section marks.
#![allow(dead_code)]

use egui::Color32;

pub const CHROME: Color32 = Color32::from_rgb(12, 16, 22);
pub const CARD: Color32 = Color32::from_rgb(20, 26, 36);
pub const CARD_BORDER: Color32 = Color32::from_rgb(48, 60, 82);
pub const FIELD: Color32 = Color32::from_rgb(24, 31, 44);
pub const FIELD_HOVER: Color32 = Color32::from_rgb(33, 42, 59);
pub const METER_BED: Color32 = Color32::from_rgb(28, 36, 50);

pub const TEXT: Color32 = Color32::from_rgb(214, 228, 245);
pub const TEXT_DIM: Color32 = Color32::from_rgb(148, 163, 184);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(120, 130, 145);

pub const TEAL: Color32 = Color32::from_rgb(45, 212, 191);
pub const TEAL_DEEP: Color32 = Color32::from_rgb(13, 84, 76);
pub const CORAL: Color32 = Color32::from_rgb(248, 113, 113);
pub const GREEN: Color32 = Color32::from_rgb(52, 211, 153);
pub const AMBER: Color32 = Color32::from_rgb(250, 204, 21);

pub const RADIUS_SM: u8 = 6;
pub const RADIUS_MD: u8 = 8;
pub const RADIUS_LG: u8 = 10;

/// Launcher-style HUD micro-label (craft-launcher discipline): 11px
/// uppercase monospace +1.6 letter spacing. For eyebrows, footers,
/// badges and empty states — the single cheapest "not default egui" cue.
pub fn hud(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let job = egui::text::LayoutJob::simple(
        text.to_uppercase(),
        egui::FontId::monospace(11.0),
        TEXT_FAINT,
        f32::INFINITY,
    );
    let mut job = job;
    // LayoutJob::simple has no spacing knob; rebuild with one.
    job.sections.iter_mut().for_each(|s| {
        s.format.extra_letter_spacing = 1.6;
    });
    ui.label(job)
}

/// Thin floating scrollbars whose thumb never vanishes, fast tooltips.
pub fn tweak_style(s: &mut egui::Style) {
    s.spacing.scroll.floating = true;
    s.spacing.scroll.bar_width = 6.0;
    s.spacing.scroll.handle_min_length = 24.0;
    s.interaction.tooltip_delay = 0.35;
}
