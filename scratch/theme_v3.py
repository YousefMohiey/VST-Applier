p = "src/theme.rs"
s = open(p, encoding="utf-8").read()

# Replace the numbered-badge section head with a plain launcher-style label,
# swap the toggle for a white checkbox, and add a compact checkbox row.
old_head = '''/// Settings section header: accent index badge + mono-caps title + hairline.
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
}'''

new_head = '''/// Settings subsection label: mono-caps muted, spaced above its rows —
/// the launcher-settings pattern (no badges, no rules).
pub fn settings_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(16.0);
    hud_colored(ui, text, muted());
    ui.add_space(6.0);
}

/// Launcher-style checkbox: 16x16 hairline box; checked = ink square
/// with a dark check. Returns true on toggle.
pub fn checkbox_white(ui: &mut egui::Ui, checked: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    let inner = rect.shrink(1.0);
    if *checked {
        p.rect_filled(inner, egui::CornerRadius::ZERO, ink_strong());
        let s = egui::Stroke::new(1.7_f32, bg());
        let at = |x: f32, y: f32| egui::pos2(rect.min.x + x, rect.min.y + y);
        p.line_segment([at(4.4, 8.6), at(6.9, 11.2)], s);
        p.line_segment([at(6.9, 11.2), at(11.6, 5.4)], s);
    } else {
        p.rect_stroke(
            inner,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(
                1.2_f32,
                if resp.hovered() { line_strong() } else { line() },
            ),
            egui::StrokeKind::Inside,
        );
    }
    if resp.clicked() {
        *checked = !*checked;
        true
    } else {
        false
    }
}

/// Checkbox row: box then label, left-aligned, launcher-settings density.
pub fn checkbox_row(ui: &mut egui::Ui, label: &str, value: &mut bool, dirty: &mut bool) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 28.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            if checkbox_white(ui, value) {
                *dirty = true;
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new(label).color(ink()).size(13.0));
        },
    );
}'''

assert old_head in s
s = s.replace(old_head, new_head)

# Remove the now-unused toggle + setting_toggle_row
old_toggle = s[s.index("/// Square toggle switch (34x18, zero radius)"):s.index("/// One settings row with a plain right-aligned value")]
s = s.replace(old_toggle, "")

open(p, "w", encoding="utf-8").write(s)
print("theme v3 widgets done")
