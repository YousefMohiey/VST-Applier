p = "src/main.rs"
s = open(p, encoding="utf-8").read()

# Extract the CREDITS block from inside the scroll area
credits_block = """                    theme::section_head(ui, "04", "CREDITS");
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 26.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("VST-Applier").color(theme::ink()).size(13.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new("made by Yousef Mohiey")
                                            .color(theme::muted())
                                            .size(12.5),
                                    );
                                },
                            );
                        },
                    );
                    theme::hairline(ui);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("GitHub").color(theme::muted()).size(12.5),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.hyperlink_to(
                                        egui::RichText::new("github.com/YousefMohiey")
                                            .color(theme::accent())
                                            .size(12.5),
                                        "https://github.com/YousefMohiey",
                                    )
                                    .on_hover_text("Opens in your browser");
                                },
                            );
                        },
                    );
                    theme::hairline(ui);
                        }); // end settings scroll area
"""

assert credits_block in s, "credits block not found (indentation drift?)"
s = s.replace(credits_block, "                        }); // end settings scroll area\n")

# Re-insert the credits block AFTER the scroll area close, before the footer
anchor = """                        }); // end settings scroll area

                    ui.add_space(12.0);

                    // Footer: primary Save + secondary Close, always visible."""
new = """                        }); // end settings scroll area

                    ui.add_space(10.0);

                    // Credits stay OUTSIDE the scroll: always on screen.
                    theme::section_head(ui, "04", "CREDITS");
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 26.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("VST-Applier").color(theme::ink()).size(13.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new("made by Yousef Mohiey")
                                            .color(theme::muted())
                                            .size(12.5),
                                    );
                                },
                            );
                        },
                    );
                    theme::hairline(ui);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("GitHub").color(theme::muted()).size(12.5),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.hyperlink_to(
                                        egui::RichText::new("github.com/YousefMohiey")
                                            .color(theme::accent())
                                            .size(12.5),
                                        "https://github.com/YousefMohiey",
                                    )
                                    .on_hover_text("Opens in your browser");
                                },
                            );
                        },
                    );
                    theme::hairline(ui);
                    ui.add_space(10.0);

                    // Footer: primary Save + secondary Close, always visible."""
assert anchor in s, "anchor not found"
s = s.replace(anchor, new)

open(p, "w", encoding="utf-8").write(s)
print("credits moved out of scroll")
