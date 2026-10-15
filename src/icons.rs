//! Icons via the Phosphor icon font (egui-phosphor, Regular variant),
//! registered as a fallback on the Proportional family at startup.
//!
//! Same public API as the old hand-painted set (`Icon`, `Glyph`,
//! `icon`, `icon_button`, `glyph_button`, `logo`, `checkbox_red`) so call
//! sites are untouched — but every glyph is now a crisp vector icon that
//! scales cleanly to any size.

use egui::{Color32, RichText, Ui, Vec2};
use egui_phosphor::variants::regular as P;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Icon {
    /// App logo / section mark.
    Mic,
    /// Audio Route section.
    Wave,
    /// Input device row.
    Headset,
    /// Buffer row.
    Chip,
    /// Virtual cable section.
    Link,
    /// Levels section.
    Bars,
    /// Settings row.
    Gear,
    /// Plugin chain section.
    Layers,
    /// Available plugins section.
    Puzzle,
    /// Clear / Delete.
    Trash,
    /// Search field.
    Search,
    /// Folder row.
    Folder,
    /// Right chevron (Settings row).
    ChevR,
    /// Header New button.
    Plus,
    /// Header Save button.
    Disk,
    /// Edit buttons.
    Sliders,
    /// Load order button.
    SwapV,
    /// Engine telemetry section.
    Gauge,
}

pub const CORAL: Color32 = Color32::from_rgb(240, 76, 86);
pub const MIST: Color32 = Color32::from_rgb(203, 213, 225);

fn glyph(kind: Icon) -> &'static str {
    match kind {
        Icon::Mic => P::MICROPHONE,
        Icon::Wave => P::WAVEFORM,
        Icon::Headset => P::HEADPHONES,
        Icon::Chip => P::CPU,
        Icon::Link => P::PLUGS_CONNECTED,
        Icon::Bars => P::CHART_BAR,
        Icon::Gear => P::GEAR,
        Icon::Layers => P::STACK,
        Icon::Puzzle => P::PUZZLE_PIECE,
        Icon::Trash => P::TRASH,
        Icon::Search => P::MAGNIFYING_GLASS,
        Icon::Folder => P::FOLDER_OPEN,
        Icon::ChevR => P::CARET_RIGHT,
        Icon::Plus => P::PLUS,
        Icon::Disk => P::FLOPPY_DISK,
        Icon::Sliders => P::SLIDERS_HORIZONTAL,
        Icon::SwapV => P::ARROWS_DOWN_UP,
        Icon::Gauge => P::GAUGE,
    }
}

/// Section-mark icon: 21px glyph, vertically centered in its row.
pub fn icon(ui: &mut Ui, kind: Icon, color: Color32) {
    ui.label(RichText::new(glyph(kind)).size(21.0).color(color));
}

/// Clickable glyph for icon buttons (real font arrows now).
#[derive(Clone, Copy)]
pub enum Glyph {
    Up,
    Down,
    X,
    Undo,
    Redo,
    /// The same wheel arrow as the launcher's refresh.
    Refresh,
}

fn glyph_str(kind: Glyph) -> &'static str {
    match kind {
        Glyph::Up => P::ARROW_UP,
        Glyph::Down => P::ARROW_DOWN,
        Glyph::X => P::X,
        Glyph::Undo => P::ARROW_COUNTER_CLOCKWISE,
        Glyph::Redo => P::ARROW_CLOCKWISE,
        Glyph::Refresh => P::ARROWS_CLOCKWISE,
    }
}

/// Ghost button with a font glyph. Hover inverts to white/black like
/// the launcher's header cells, so row actions read instantly.
pub fn glyph_button(ui: &mut Ui, kind: Glyph, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(36.0, 28.0), egui::Sense::click());
    let p = ui.painter();
    let invert = resp.hovered() && resp.enabled();
    if resp.is_pointer_button_down_on() {
        p.rect_filled(
            rect,
            egui::CornerRadius::ZERO,
            Color32::from_white_alpha(120),
        );
    } else if invert {
        p.rect_filled(rect, egui::CornerRadius::ZERO, Color32::WHITE);
    }
    let color = if !resp.enabled() {
        Color32::from_gray(110)
    } else if invert {
        Color32::from_rgb(0x0b, 0x0b, 0x0c)
    } else {
        MIST
    };
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph_str(kind),
        egui::FontId::proportional(16.0),
        color,
    );
    resp.on_hover_text(tip)
}

/// Red enable/disable checkbox (reference style). Returns true on toggle.
pub fn checkbox_red(ui: &mut Ui, checked: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::click());
    let p = ui.painter();
    let inner = rect.shrink(1.0);
    if *checked {
        p.rect_filled(inner, egui::CornerRadius::ZERO, crate::theme::CHECK_RED);
        let s = egui::Stroke::new(2.0_f32, egui::Color32::WHITE);
        let at = |x: f32, y: f32| egui::pos2(rect.min.x + x, rect.min.y + y);
        p.line_segment([at(5.5, 10.0), at(8.3, 12.8)], s);
        p.line_segment([at(8.3, 12.8), at(13.0, 6.8)], s);
    } else {
        p.rect_stroke(
            inner,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.2_f32, if resp.hovered() {
                egui::Color32::from_gray(170)
            } else {
                egui::Color32::from_gray(110)
            }),
            egui::StrokeKind::Inside,
        );
    }
    let resp = resp.on_hover_text("Enable/disable plugin");
    if resp.clicked() {
        *checked = !*checked;
        true
    } else {
        false
    }
}

/// Button with an icon glyph left and text: native hover/active styling.
/// Fixed size — pass constants, never `available_width()` derivations that
/// could feed panel-width feedback.
#[allow(dead_code)]
pub fn icon_button(ui: &mut Ui, ic: Icon, text: &str, size: Vec2) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(format!("{}  {text}", glyph(ic))).size(14.5),
    );
    ui.add_sized(size, btn)
}

pub fn logo(ui: &mut Ui) {
    ui.label(RichText::new(P::MICROPHONE_STAGE).size(24.0).color(CORAL));
}
