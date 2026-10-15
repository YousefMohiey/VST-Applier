//! Design tokens — transplant of craft-launcher's theme (`storytold/
//! craft-launcher` crates/ui-egui/src/theme.rs): hard edges, hairline
//! rules, mono upper-case micro-labels, one accent, one shadow.
//! Widgets never hard-code colors — everything reads these.
//!
//! Palette mapping: their near-black neutral surfaces + warm-white ink,
//! with our audio semantics on top (teal = running/good, red = danger,
//! red checkbox kept from the approved reference mockup).
#![allow(dead_code)]

use egui::Color32;

/// Page, panels and cards (their `bg`).
pub const BG: Color32 = Color32::from_rgb(0x12, 0x13, 0x16);
/// Wells: text edits, meter beds, badges (their `sunken`).
pub const SUNKEN: Color32 = Color32::from_rgb(0x06, 0x06, 0x07);
/// Body text (warm white).
pub const INK: Color32 = Color32::from_rgb(0xf2, 0xf1, 0xee);
pub const INK_STRONG: Color32 = Color32::WHITE;
/// Secondary text (ink @ 60%, premultiplied).
pub const MUTED: Color32 = Color32::from_rgba_premultiplied(0x91, 0x91, 0x8f, 0x99);
/// Labels and tertiary text (ink @ 38%, premultiplied).
pub const FAINT: Color32 = Color32::from_rgba_premultiplied(0x5c, 0x5b, 0x5a, 0x61);
/// Hairlines (ink @ 15%, premultiplied).
pub const LINE: Color32 = Color32::from_rgba_premultiplied(0x24, 0x24, 0x23, 0x26);
/// Outlined buttons, window stroke (ink @ 40%, premultiplied).
pub const LINE_STRONG: Color32 = Color32::from_rgba_premultiplied(0x61, 0x60, 0x5f, 0x66);

/// Selection/focus accent (their brand blue; links + selection).
pub const ACCENT: Color32 = Color32::from_rgb(0x2d, 0x81, 0xff);
/// The same blue for text and icons.
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x74, 0xaa, 0xff);

/// Primary-button inversion.
pub const INVERT_BG: Color32 = Color32::WHITE;
pub const INVERT_FG: Color32 = Color32::from_rgb(0x0b, 0x0b, 0x0c);

/// Status palette (their good/warn/bad).
pub const GOOD: Color32 = Color32::from_rgb(0x3f, 0xd6, 0xb8);
pub const WARN: Color32 = Color32::from_rgb(0xff, 0xc1, 0x4d);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x62, 0x59);

// ---- legacy names kept so call sites stay put ----------------------
pub const CHROME: Color32 = BG;
pub const CARD: Color32 = BG;
pub const CARD_BORDER: Color32 = LINE;
pub const FIELD: Color32 = SUNKEN;
pub const FIELD_HOVER: Color32 = Color32::from_rgb(0x1b, 0x1c, 0x1f);
pub const METER_BED: Color32 = SUNKEN;
pub const TEXT: Color32 = INK;
pub const TEXT_DIM: Color32 = MUTED;
pub const TEXT_FAINT: Color32 = FAINT;
pub const TEAL: Color32 = GOOD;
pub const TEAL_DEEP: Color32 = Color32::from_rgb(0x11, 0x4a, 0x3f);
pub const CORAL: Color32 = BAD;
pub const GREEN: Color32 = GOOD;
pub const AMBER: Color32 = WARN;
/// The approved red checkbox red.
pub const CHECK_RED: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

/// Hard edges everywhere (their signature).
pub const RADIUS_SM: u8 = 0;
pub const RADIUS_MD: u8 = 0;
pub const RADIUS_LG: u8 = 0;

/// Height of buttons, inputs, combos (their `CONTROL_H`).
pub const CONTROL_H: f32 = 34.0;
/// `.hud-label`: 11 px mono, upper case, 0.15 em apart.
pub const HUD_SIZE: f32 = 11.0;
pub const HUD_SPACING: f32 = 1.65;
/// Buttons: 11 px mono, upper case, 0.12 em apart.
pub const BUTTON_SPACING: f32 = 1.32;

/// `text` laid out upper case in mono, `spacing` between letters.
fn caps_galley(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    spacing: f32,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    let format = egui::TextFormat {
        font_id: egui::FontId::new(size, egui::FontFamily::Monospace),
        color,
        extra_letter_spacing: spacing,
        ..Default::default()
    };
    ui.painter()
        .layout_job(egui::text::LayoutJob::single_section(
            text.to_uppercase(),
            format,
        ))
}

/// A `.hud-label`: 11 px mono, upper case, widely spaced.
pub fn hud(ui: &mut egui::Ui, text: &str) -> egui::Response {
    hud_colored(ui, text, FAINT)
}

/// Same, in a specific color.
pub fn hud_colored(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley = caps_galley(ui, text, HUD_SIZE, HUD_SPACING, color);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    resp
}

/// Measured width of a hud-style label (same layout the label will use).
pub fn hud_width(ui: &egui::Ui, text: &str) -> f32 {
    caps_galley(ui, text, HUD_SIZE, HUD_SPACING, FAINT).size().x
}

/// Which of the button variants (their `widgets::Kind`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BtnKind {
    /// Solid inverted block; fades a little on hover.
    Primary,
    /// Outlined hairline; inverts on hover.
    Secondary,
    /// Solid red; fades a little on hover.
    Danger,
}

/// One button in craft-launcher style: hard square, mono upper-case
/// label, at least `width` wide and `CONTROL_H` tall.
pub fn button(ui: &mut egui::Ui, text: &str, kind: BtnKind, width: f32) -> egui::Response {
    button_sized(ui, text, kind, width, CONTROL_H)
}

/// Same button with an explicit height (compact menus).
pub fn button_sized(
    ui: &mut egui::Ui,
    text: &str,
    kind: BtnKind,
    width: f32,
    height: f32,
) -> egui::Response {
    let enabled = ui.is_enabled();
    let galley = caps_galley(
        ui,
        text,
        HUD_SIZE,
        BUTTON_SPACING,
        if enabled { INK } else { INK.gamma_multiply(0.5) },
    );
    let size = egui::vec2((galley.size().x + 32.0).max(width), height);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = resp.hovered() && enabled;
    let (fill, stroke) = match kind {
        BtnKind::Primary => (
            INVERT_BG.gamma_multiply(if hovered { 0.8 } else { 1.0 }),
            egui::Stroke::NONE,
        ),
        BtnKind::Danger => (
            BAD.gamma_multiply(if hovered { 0.8 } else { 1.0 }),
            egui::Stroke::NONE,
        ),
        BtnKind::Secondary if hovered => (INVERT_BG, egui::Stroke::NONE),
        BtnKind::Secondary => (BG, egui::Stroke::new(1.0_f32, LINE_STRONG)),
    };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.5) };
    ui.painter().rect(
        rect,
        egui::CornerRadius::ZERO,
        fade(fill),
        egui::Stroke::new(stroke.width, fade(stroke.color)),
        egui::StrokeKind::Inside,
    );
    let ink = match kind {
        BtnKind::Primary => fade(INVERT_FG),
        BtnKind::Danger => fade(Color32::WHITE),
        BtnKind::Secondary if hovered => fade(INVERT_FG),
        BtnKind::Secondary => {
            if enabled {
                INK
            } else {
                INK.gamma_multiply(0.5)
            }
        }
    };
    let galley = caps_galley(ui, text, HUD_SIZE, BUTTON_SPACING, ink);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// A solid chip with a mono label (their `tag`): number tabs, badges.
pub fn tag(ui: &mut egui::Ui, text: &str, fill: Color32, ink: Color32) -> egui::Response {
    let galley = caps_galley(ui, text, HUD_SIZE, HUD_SPACING, ink);
    let (rect, resp) =
        ui.allocate_exact_size(galley.size() + egui::vec2(16.0, 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    resp
}

/// A small square status badge: 10 px mono on the sunken surface (their
/// `widgets::badge`). Hairline-outlined, hard edges.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley = caps_galley(ui, text, 10.0, 1.0, color);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(galley.size().x + 12.0, 20.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        egui::CornerRadius::ZERO,
        SUNKEN,
        egui::Stroke::new(1.0_f32, LINE),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// The single shadow in the app: menus, dialogs and toasts only.
pub fn shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(140),
    }
}

/// Their `visuals()`: square everything, hairline strokes, one shadow.
pub fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = SUNKEN;
    v.faint_bg_color = SUNKEN;
    v.code_bg_color = SUNKEN;
    v.window_stroke = egui::Stroke::new(1.0_f32, LINE_STRONG);
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = shadow();
    v.popup_shadow = shadow();
    v.override_text_color = Some(INK);
    v.hyperlink_color = ACCENT_INK;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    v.warn_fg_color = WARN;
    v.error_fg_color = BAD;
    for (w, fill, stroke, fg) in [
        (&mut v.widgets.noninteractive, BG, LINE, MUTED),
        (&mut v.widgets.inactive, BG, LINE, INK),
        (&mut v.widgets.hovered, SUNKEN, LINE_STRONG, INK_STRONG),
        (&mut v.widgets.active, SUNKEN, LINE_STRONG, INK_STRONG),
        (&mut v.widgets.open, SUNKEN, LINE_STRONG, INK_STRONG),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = egui::Stroke::new(1.0_f32, stroke);
        w.corner_radius = egui::CornerRadius::ZERO;
        w.fg_stroke = egui::Stroke::new(1.0_f32, fg);
        w.expansion = 0.0;
    }
    v
}

/// Their `all_styles_mut` spacing/text values plus our scrollbar tweak.
pub fn style() -> egui::Style {
    use egui::{FontFamily, FontId, TextStyle};
    let mut s = egui::Style {
        visuals: visuals(),
        ..Default::default()
    };
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.icon_spacing = 10.0;
    s.spacing.button_padding = egui::vec2(12.0, 6.0);
    s.spacing.interact_size.y = CONTROL_H;
    s.spacing.menu_margin = egui::Margin::same(8);
    s.spacing.window_margin = egui::Margin::same(4);
    s.spacing.combo_height = 300.0;
    s.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(24.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
    .into();
    tweak_style(&mut s);
    s
}

/// Thin floating scrollbars whose thumb never vanishes, fast tooltips.
pub fn tweak_style(s: &mut egui::Style) {
    s.spacing.scroll.floating = true;
    s.spacing.scroll.bar_width = 6.0;
    s.spacing.scroll.handle_min_length = 24.0;
    s.interaction.tooltip_delay = 0.35;
}
