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

/// Colour mode: the app runs dark by default (its signature look) and
/// offers a light mode from Settings. Every token below resolves through
/// this flag at draw time, so switching modes repaints everything.
static DARK_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn dark() -> bool {
    DARK_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_dark(d: bool) {
    DARK_MODE.store(d, std::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn pick(d: Color32, l: Color32) -> Color32 {
    if dark() {
        d
    } else {
        l
    }
}

/// Page, panels and cards — near-black in dark mode, warm paper in light.
pub fn bg() -> Color32 {
    pick(
        Color32::from_rgb(0x12, 0x13, 0x16),
        Color32::from_rgb(0xf4, 0xf3, 0xf1),
    )
}
/// Wells: text edits, meter beds, badges (`sunken`).
pub fn sunken() -> Color32 {
    pick(
        Color32::from_rgb(0x06, 0x06, 0x07),
        Color32::from_rgb(0xe7, 0xe6, 0xe3),
    )
}
/// Body text.
pub fn ink() -> Color32 {
    pick(
        Color32::from_rgb(0xf2, 0xf1, 0xee),
        Color32::from_rgb(0x1a, 0x1b, 0x1e),
    )
}
pub fn ink_strong() -> Color32 {
    pick(Color32::WHITE, Color32::BLACK)
}
/// Secondary text (ink @ 60%, premultiplied).
pub fn muted() -> Color32 {
    pick(
        Color32::from_rgba_premultiplied(0x91, 0x91, 0x8f, 0x99),
        Color32::from_rgba_premultiplied(0x10, 0x10, 0x10, 0x99),
    )
}
/// Labels and tertiary text (ink @ 38%, premultiplied).
pub fn faint() -> Color32 {
    pick(
        Color32::from_rgba_premultiplied(0x5c, 0x5b, 0x5a, 0x61),
        Color32::from_rgba_premultiplied(0x0a, 0x0a, 0x0a, 0x61),
    )
}
/// Hairlines (ink @ 15%, premultiplied).
pub fn line() -> Color32 {
    pick(
        Color32::from_rgba_premultiplied(0x24, 0x24, 0x23, 0x26),
        Color32::from_rgba_premultiplied(0x06, 0x06, 0x06, 0x26),
    )
}
/// Outlined buttons, window stroke (ink @ 40%, premultiplied).
pub fn line_strong() -> Color32 {
    pick(
        Color32::from_rgba_premultiplied(0x61, 0x60, 0x5f, 0x66),
        Color32::from_rgba_premultiplied(0x0c, 0x0c, 0x0c, 0x66),
    )
}

/// Selection/focus accent.
pub fn accent() -> Color32 {
    pick(
        Color32::from_rgb(0x2d, 0x81, 0xff),
        Color32::from_rgb(0x1f, 0x6f, 0xe5),
    )
}
/// The same blue for text and icons (darkened for light mode contrast).
pub fn accent_ink() -> Color32 {
    pick(
        Color32::from_rgb(0x74, 0xaa, 0xff),
        Color32::from_rgb(0x16, 0x5f, 0xc9),
    )
}

/// Primary-button inversion (flips with the mode).
pub fn invert_bg() -> Color32 {
    pick(
        Color32::WHITE,
        Color32::from_rgb(0x0b, 0x0b, 0x0c),
    )
}
pub fn invert_fg() -> Color32 {
    pick(
        Color32::from_rgb(0x0b, 0x0b, 0x0c),
        Color32::WHITE,
    )
}

/// Status palette (darkened in light mode for contrast on paper).
pub fn good() -> Color32 {
    pick(
        Color32::from_rgb(0x3f, 0xd6, 0xb8),
        Color32::from_rgb(0x0e, 0x9f, 0x82),
    )
}
pub fn warn() -> Color32 {
    pick(
        Color32::from_rgb(0xff, 0xc1, 0x4d),
        Color32::from_rgb(0xb2, 0x7b, 0x1c),
    )
}
pub fn bad() -> Color32 {
    pick(
        Color32::from_rgb(0xff, 0x62, 0x59),
        Color32::from_rgb(0xd6, 0x4a, 0x41),
    )
}

// ---- legacy names kept so call sites stay put ----------------------
pub fn chrome() -> Color32 {
    bg()
}
pub fn card() -> Color32 {
    bg()
}
pub fn card_border() -> Color32 {
    line()
}
pub fn field() -> Color32 {
    sunken()
}
pub fn field_hover() -> Color32 {
    pick(
        Color32::from_rgb(0x1b, 0x1c, 0x1f),
        Color32::from_rgb(0xde, 0xdd, 0xda),
    )
}
pub fn meter_bed() -> Color32 {
    sunken()
}
pub fn text() -> Color32 {
    ink()
}
pub fn text_dim() -> Color32 {
    muted()
}
pub fn text_faint() -> Color32 {
    faint()
}
pub fn teal() -> Color32 {
    good()
}
pub fn teal_deep() -> Color32 {
    pick(
        Color32::from_rgb(0x11, 0x4a, 0x3f),
        Color32::from_rgb(0xcf, 0xed, 0xe6),
    )
}
pub fn coral() -> Color32 {
    bad()
}
pub fn green() -> Color32 {
    good()
}
pub fn amber() -> Color32 {
    warn()
}
/// Soft glyph tone for icons drawn on the app surface (light slate in
/// dark mode, dark slate in light mode).
pub fn mist() -> Color32 {
    pick(
        Color32::from_rgb(203, 213, 225),
        Color32::from_rgb(0x3c, 0x3e, 0x44),
    )
}

/// The approved red checkbox red (same in both modes).
pub fn check_red() -> Color32 {
    Color32::from_rgb(0xe5, 0x48, 0x4d)
}

/// Hard edges everywhere (their signature).
pub const RADIUS_SM: u8 = 0;
pub const RADIUS_MD: u8 = 0;
pub const RADIUS_LG: u8 = 0;

/// Height of buttons, inputs, combos (their `CONTROL_H`).
pub const CONTROL_H: f32 = 34.0;
/// `.hud-label`: 12 px mono, upper case, 0.15 em apart — 11 px read too
/// small next to the 20 px control glyphs.
pub const HUD_SIZE: f32 = 12.0;
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
    hud_colored(ui, text, faint())
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
    caps_galley(ui, text, HUD_SIZE, HUD_SPACING, faint()).size().x
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
        if enabled { ink() } else { ink().gamma_multiply(0.5) },
    );
    let size = egui::vec2((galley.size().x + 32.0).max(width), height);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = resp.hovered() && enabled;
    let (fill, stroke) = match kind {
        BtnKind::Primary => (
            invert_bg().gamma_multiply(if hovered { 0.8 } else { 1.0 }),
            egui::Stroke::NONE,
        ),
        BtnKind::Danger => (
            bad().gamma_multiply(if hovered { 0.8 } else { 1.0 }),
            egui::Stroke::NONE,
        ),
        BtnKind::Secondary if hovered => (invert_bg(), egui::Stroke::NONE),
        BtnKind::Secondary => (bg(), egui::Stroke::new(1.0_f32, line_strong())),
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
        BtnKind::Primary => fade(invert_fg()),
        BtnKind::Danger => fade(Color32::WHITE),
        BtnKind::Secondary if hovered => fade(invert_fg()),
        BtnKind::Secondary => {
            if enabled {
                ink()
            } else {
                ink().gamma_multiply(0.5)
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

/// A small square status badge: 11 px mono on the sunken surface (their
/// `widgets::badge`). Hairline-outlined, hard edges.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley = caps_galley(ui, text, 11.0, 1.0, color);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(galley.size().x + 12.0, 21.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        egui::CornerRadius::ZERO,
        sunken(),
        egui::Stroke::new(1.0_f32, line()),
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
/// Built from the current mode so egui-internal widgets (text edits,
/// combos, scrollbars, tooltips, menus) match our tokens in both modes.
pub fn visuals() -> egui::Visuals {
    let mut v = if dark() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = bg();
    v.window_fill = bg();
    v.extreme_bg_color = sunken();
    v.faint_bg_color = sunken();
    v.code_bg_color = sunken();
    v.window_stroke = egui::Stroke::new(1.0_f32, line_strong());
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = shadow();
    v.popup_shadow = shadow();
    v.override_text_color = Some(ink());
    v.hyperlink_color = accent_ink();
    v.selection.bg_fill = accent();
    v.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    v.warn_fg_color = warn();
    v.error_fg_color = bad();
    for (w, fill, stroke, fg) in [
        (&mut v.widgets.noninteractive, bg(), line(), muted()),
        (&mut v.widgets.inactive, bg(), line(), ink()),
        (&mut v.widgets.hovered, sunken(), line_strong(), ink_strong()),
        (&mut v.widgets.active, sunken(), line_strong(), ink_strong()),
        (&mut v.widgets.open, sunken(), line_strong(), ink_strong()),
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
