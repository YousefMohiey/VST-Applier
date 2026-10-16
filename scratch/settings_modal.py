import re

p = "src/main.rs"
s = open(p, encoding="utf-8").read()

i0 = s.index("        if self.show_settings {")
i1 = s.index("    }\n\n    fn on_exit(", i0)

new_block = '''        if self.show_settings {
            // egui::Modal: top-most layer (Foreground), so the backdrop
            // dims ONLY the app behind it and never the panel itself;
            // centered with a slight upward nudge; backdrop click, Esc,
            // the header x and DONE all close it.
            let frame = egui::Frame::new()
                .fill(theme::bg())
                .stroke(egui::Stroke::new(1.0, theme::line_strong()))
                .corner_radius(egui::CornerRadius::ZERO)
                .inner_margin(egui::Margin::same(20));
            let area = egui::Modal::default_area(egui::Id::new("settings_modal"))
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -28.0));
            let modal = egui::Modal::new(egui::Id::new("settings_modal"))
                .area(area)
                .frame(frame)
                .backdrop_color(egui::Color32::from_black_alpha(150));
            let resp = modal.show(ctx, |ui| {
                ui.set_width(560.0);
                // Every change below is saved immediately (autosave).
                let mut settings_dirty = false;

                // Header: title + subtitle + close (no collapse arrow).
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Settings").color(theme::ink()).size(20.0),
                        );
                        theme::hud_colored(
                            ui,
                            &format!("VST-APPLIER {}", update::APP_VERSION),
                            theme::muted(),
                        );
                    });
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::TOP),
                        |ui| {
                            if theme::button_sized(
                                ui,
                                "\\u{00D7}",
                                theme::BtnKind::Secondary,
                                34.0,
                                26.0,
                            )
                            .on_hover_text("Close")
                            .clicked()
                            {
                                self.show_settings = false;
                            }
                        },
                    );
                });
                theme::hairline(ui);

                theme::settings_label(ui, "GENERAL");
                theme::checkbox_row(
                    ui,
                    "Start microphone routing on open",
                    &mut self.settings.AutoStartMicrophone,
                    &mut settings_dirty,
                );
                theme::checkbox_row(
                    ui,
                    "Keep running in tray on close",
                    &mut self.settings.CloseToTray,
                    &mut settings_dirty,
                );
                theme::checkbox_row(
                    ui,
                    "Mute other apps on the virtual cable",
                    &mut self.settings.KeepCableClean,
                    &mut settings_dirty,
                );

                theme::settings_label(ui, "BACKGROUND");
                theme::checkbox_row(
                    ui,
                    "Start with Windows (minimized to tray)",
                    &mut self.settings.StartWithWindows,
                    &mut settings_dirty,
                );

                theme::settings_label(ui, "APPEARANCE");
                ui.horizontal(|ui| {
                    let cur = theme::pref();
                    let k = |p: theme::Pref| {
                        if cur == p {
                            theme::BtnKind::Primary
                        } else {
                            theme::BtnKind::Secondary
                        }
                    };
                    if theme::button_sized(ui, "MATCH SYSTEM", k(theme::Pref::System), 132.0, 28.0)
                        .on_hover_text("Follow the Windows app theme")
                        .clicked()
                    {
                        self.settings.Theme = "system".into();
                        theme::set_pref(theme::Pref::System);
                        settings_dirty = true;
                    }
                    if theme::button_sized(ui, "DARK", k(theme::Pref::Dark), 76.0, 28.0).clicked() {
                        self.settings.Theme = "dark".into();
                        theme::set_pref(theme::Pref::Dark);
                        settings_dirty = true;
                    }
                    if theme::button_sized(ui, "LIGHT", k(theme::Pref::Light), 76.0, 28.0)
                        .clicked()
                    {
                        self.settings.Theme = "light".into();
                        theme::set_pref(theme::Pref::Light);
                        settings_dirty = true;
                    }
                });

                theme::settings_label(ui, "UPDATES");
                theme::checkbox_row(
                    ui,
                    "Check GitHub for updates on startup",
                    &mut self.settings.CheckUpdates,
                    &mut settings_dirty,
                );
                ui.add_space(6.0);
                let busy = self.update_rx.is_some();
                let mut install_err: Option<String> = None;
                match &self.update_state {
                    UpdateUi::Idle => {
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(!busy, |ui| {
                                if theme::button_sized(
                                    ui,
                                    "CHECK NOW",
                                    theme::BtnKind::Secondary,
                                    130.0,
                                    28.0,
                                )
                                .clicked()
                                {
                                    self.spawn_update_check();
                                }
                            });
                        });
                    }
                    UpdateUi::Checking => {
                        ui.label(
                            egui::RichText::new("Checking GitHub...")
                                .color(theme::muted())
                                .size(12.5),
                        );
                    }
                    UpdateUi::Current => {
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(!busy, |ui| {
                                if theme::button_sized(
                                    ui,
                                    "CHECK NOW",
                                    theme::BtnKind::Secondary,
                                    130.0,
                                    28.0,
                                )
                                .clicked()
                                {
                                    self.spawn_update_check();
                                }
                            });
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("On the latest version")
                                    .color(theme::muted())
                                    .size(12.5),
                            );
                        });
                    }
                    UpdateUi::Failed(e) => {
                        ui.label(
                            egui::RichText::new(format!("Update check failed: {e}"))
                                .color(theme::bad())
                                .size(12.5),
                        );
                    }
                    UpdateUi::Available { tag, name, .. } => {
                        ui.label(
                            egui::RichText::new(format!("{name} ({tag}) is available"))
                                .color(theme::ink())
                                .size(13.0),
                        );
                        ui.add_space(4.0);
                        let pending = self.update_pending.clone();
                        if theme::button_sized(
                            ui,
                            "DOWNLOAD UPDATE",
                            theme::BtnKind::Primary,
                            170.0,
                            28.0,
                        )
                        .clicked()
                        {
                            if let Some(info) = pending {
                                self.spawn_update_download(info);
                            }
                        }
                    }
                    UpdateUi::Downloading => {
                        ui.label(
                            egui::RichText::new("Downloading + extracting...")
                                .color(theme::muted())
                                .size(12.5),
                        );
                    }
                    UpdateUi::Ready(stage) => {
                        ui.label(
                            egui::RichText::new("Update staged - the app restarts to finish.")
                                .color(theme::ink())
                                .size(12.5),
                        );
                        ui.label(
                            egui::RichText::new(
                                "If a Windows admin prompt appears, approve it to finish the update.",
                            )
                            .color(theme::muted())
                            .size(12.0),
                        );
                        ui.add_space(4.0);
                        let stage = stage.clone();
                        if theme::button_sized(
                            ui,
                            "INSTALL + RESTART",
                            theme::BtnKind::Primary,
                            170.0,
                            28.0,
                        )
                        .clicked()
                        {
                            // Exits via process::exit (skips on_exit):
                            // persist FIRST so the live chain survives.
                            self.persist_live();
                            let msg = update::install_and_restart(&stage);
                            install_err = Some(msg);
                        }
                    }
                }
                if let Some(e) = install_err {
                    self.update_state = UpdateUi::Failed(e);
                }

                theme::settings_label(ui, "CREDITS");
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 24.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            egui::RichText::new("made by Yousef Mohiey")
                                .color(theme::muted())
                                .size(12.5),
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

                ui.add_space(18.0);
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if theme::button(ui, "DONE", theme::BtnKind::Primary, 110.0).clicked() {
                            let _ = save_settings(&self.settings);
                            autostart::apply(self.settings.StartWithWindows);
                            self.guard.set_enabled(self.settings.KeepCableClean);
                            self.show_settings = false;
                        }
                    },
                );

                if settings_dirty {
                    // Autosave: persist the moment anything changes
                    // (theme included) and apply side effects right away.
                    let _ = save_settings(&self.settings);
                    autostart::apply(self.settings.StartWithWindows);
                    self.guard.set_enabled(self.settings.KeepCableClean);
                }
            });
            if resp.should_close() {
                // Backdrop click or Esc: persisting + closing is enough.
                let _ = save_settings(&self.settings);
                self.show_settings = false;
            }
        }

'''

s = s[:i0] + new_block + s[i1:]
open(p, "w", encoding="utf-8").write(s)
print("settings migrated to egui::Modal")
