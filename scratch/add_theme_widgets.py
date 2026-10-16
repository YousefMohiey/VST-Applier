import sys

p = "src/theme.rs"
s = open(p, encoding="utf-8").read()

addition = '''

// ---------------------------------------------------------------------------
// Settings surface widgets (ArtCraft-style: zero radius, hairlines, square
// toggles, mono caps section heads). Used by the Settings window.
// ---------------------------------------------------------------------------

/// 1px hairline across the available width (settings row separators).
pub fn hairline(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, line());
}

/// Settings section header: accent index badge + mono-caps title + hairline.
/// Returns the response of the title for callers that want a hover text.
pub fn section_head(ui: &mut egui::Ui, idx: &str, title: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 14.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, egui::CornerRadius::ZERO, accent());
        let galley = p.layout_no_wrap(
            idx.to_string(),
            egui::FontId::monospace(10.0),
            invert_fg(),
        );
        let pos = rect.center() - galley.size() / 2.0;
        p.galley(pos, galley, invert_fg());
        ui.add_space(8.0);
        hud_colored(ui, title, muted());
    });
    ui.add_space(6.0);
    hairline(ui);
    ui.add_space(2.0);
}

/// Square toggle switch (34x18, zero radius): ON = accent fill + light
/// knob right; OFF = sunken fill, hairline border, muted knob left.
/// Click flips the value; callers read `.clicked()` to persist.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(egui::vec2(34.0, 18.0), egui::Sense::click());
    resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
    }
    let p = ui.painter();
    if *on {
        p.rect_filled(rect, egui::CornerRadius::ZERO, accent());
        let knob = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 18.0, rect.top() + 2.0),
            egui::vec2(14.0, 14.0),
        );
        p.rect_filled(knob, egui::CornerRadius::ZERO, invert_fg());
    } else {
        p.rect_filled(rect, egui::CornerRadius::ZERO, sunken());
        p.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(
                1.0,
                if resp.hovered() { line_strong() } else { line() },
            ),
            egui::StrokeKind::Inside,
        );
        let knob = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 2.0, rect.top() + 2.0),
            egui::vec2(14.0, 14.0),
        );
        p.rect_filled(knob, egui::CornerRadius::ZERO, muted());
    }
    resp
}

/// One settings row: label left, square toggle right, hairline underneath.
/// Clicking the toggle sets `*dirty` (the caller persists immediately).
pub fn setting_toggle_row(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    value: &mut bool,
    dirty: &mut bool,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 34.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let resp = ui.label(
                egui::RichText::new(label)
                    .color(if *value { ink() } else { ink() })
                    .size(13.0),
            );
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if toggle(ui, value).clicked() {
                        *dirty = true;
                    }
                },
            );
            if !hint.is_empty() {
                resp.on_hover_text(hint);
            }
        },
    );
    hairline(ui);
}

/// One settings row with a plain right-aligned value (mono, muted).
pub fn setting_value_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 34.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(egui::RichText::new(label).color(ink()).size(13.0));
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let g = ui.painter().layout_no_wrap(
                        value.to_string(),
                        egui::FontId::monospace(12.0),
                        muted(),
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(g.size(), egui::Sense::hover());
                    ui.painter().galley(rect.min, g, muted());
                },
            );
        },
    );
    hairline(ui);
}
'''

s = s.rstrip() + addition
open(p, "w", encoding="utf-8").write(s)
print("theme widgets added")
