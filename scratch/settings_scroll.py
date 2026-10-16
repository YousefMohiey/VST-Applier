p = "src/main.rs"
s = open(p, encoding="utf-8").read()

# 1) open the scroll area right after the dirty flag
old1 = """                .show(ctx, |ui| {
                    // Every change below is saved immediately (autosave).
                    let mut settings_dirty = false;

                    theme::section_head(ui, "01", "GENERAL");"""
new1 = """                .show(ctx, |ui| {
                    // Every change below is saved immediately (autosave).
                    let mut settings_dirty = false;

                    // The sections live in a capped scroll area so the
                    // Save/Close footer below stays visible no matter how
                    // tall the content gets or where the window lands.
                    egui::ScrollArea::vertical()
                        .max_height(520.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                    theme::section_head(ui, "01", "GENERAL");"""
assert old1 in s
s = s.replace(old1, new1)

# 2) close the scroll area before the footer buttons
old2 = """                    theme::hairline(ui);
                    ui.add_space(16.0);

                    // Footer: primary Save + secondary Close.
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if theme::button(ui, "Save", theme::BtnKind::Primary, 96.0).clicked() {
                                let _ = save_settings(&self.settings);
                                autostart::apply(self.settings.StartWithWindows);
                                self.guard.set_enabled(self.settings.KeepCableClean);
                                self.show_settings = false;
                            }
                            ui.add_space(8.0);
                            if theme::button(ui, "Close", theme::BtnKind::Secondary, 96.0).clicked()
                            {
                                self.show_settings = false;
                            }
                        },
                    );"""
new2 = """                    theme::hairline(ui);
                        }); // end settings scroll area

                    ui.add_space(12.0);

                    // Footer: primary Save + secondary Close, always visible.
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if theme::button(ui, "Save", theme::BtnKind::Primary, 96.0).clicked() {
                                let _ = save_settings(&self.settings);
                                autostart::apply(self.settings.StartWithWindows);
                                self.guard.set_enabled(self.settings.KeepCableClean);
                                self.show_settings = false;
                            }
                            ui.add_space(8.0);
                            if theme::button(ui, "Close", theme::BtnKind::Secondary, 96.0).clicked()
                            {
                                self.show_settings = false;
                            }
                        },
                    );"""
assert old2 in s
s = s.replace(old2, new2)

open(p, "w", encoding="utf-8").write(s)
print("scrollarea + pinned footer applied")
