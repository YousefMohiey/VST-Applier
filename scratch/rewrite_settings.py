p = "src/main.rs"
s = open(p, encoding="utf-8").read()

i0 = s.index('        if self.show_settings {\n            egui::Window::new("Settings")')
i1 = s.index("    }\n\n    fn on_exit(", i0)

new_block = '''        if self.show_settings {
            // Sectioned settings surface: mono-caps section heads with an
            // accent index badge, hairline-separated rows (label left,
            // control right), square toggles — the launcher-style layout.
            let frame = egui::Frame::new()
                .fill(theme::card())
                .stroke(egui::Stroke::new(1.0, theme::line()))
                .corner_radius(egui::CornerRadius::ZERO)
                .inner_margin(egui::Margin::same(18));
            egui::Window::new("Settings")
                .frame(frame)
                .resizable(false)
                .default_width(430.0)
                .show(ctx, |ui| {
                    // Every change below is saved immediately (autosave).
                    let mut settings_dirty = false;

                    theme::section_head(ui, "01", "GENERAL");
                    theme::setting_toggle_row(
                        ui,
                        "Start microphone routing on open",
                        "",
                        &mut self.settings.AutoStartMicrophone,
                        &mut settings_dirty,
                    );
                    theme::setting_toggle_row(
                        ui,
                        "Keep running in tray on close",
                        "",
                        &mut self.settings.CloseToTray,
                        &mut settings_dirty,
                    );
                    theme::setting_toggle_row(
                        ui,
                        "Mute other apps on the virtual cable",
                        "Keeps app audio from leaking into the mic",
                        &mut self.settings.KeepCableClean,
                        &mut settings_dirty,
                    );
                    ui.add_space(14.0);

                    theme::section_head(ui, "02", "SYSTEM");
                    theme::setting_toggle_row(
                        ui,
                        "Start with Windows (minimized to tray)",
                        "",
                        &mut self.settings.StartWithWindows,
                        &mut settings_dirty,
                    );
                    theme::setting_toggle_row(
                        ui,
                        "Check GitHub for updates on startup",
                        "",
                        &mut self.settings.CheckUpdates,
                        &mut settings_dirty,
                    );
                    // Theme row: three states as in the rail.
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 34.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("Theme").color(theme::ink()).size(13.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let cur = theme::pref();
                                    let k = |p: theme::Pref| {
                                        if cur == p {
                                            theme::BtnKind::Primary
                                        } else {
                                            theme::BtnKind::Secondary
                                        }
                                    };
                                    if theme::button_sized(
                                        ui,
                                        "System",
                                        k(theme::Pref::System),
                                        74.0,
                                        26.0,
                                    )
                                    .on_hover_text("Follow the Windows app theme")
                                    .clicked()
                                    {
                                        self.settings.Theme = "system".into();
                                        theme::set_pref(theme::Pref::System);
                                        settings_dirty = true;
                                    }
                                    if theme::button_sized(
                                        ui,
                                        "Light",
                                        k(theme::Pref::Light),
                                        62.0,
                                        26.0,
                                    )
                                    .clicked()
                                    {
                                        self.settings.Theme = "light".into();
                                        theme::set_pref(theme::Pref::Light);
                                        settings_dirty = true;
                                    }
                                    if theme::button_sized(
                                        ui,
                                        "Dark",
                                        k(theme::Pref::Dark),
                                        62.0,
                                        26.0,
                                    )
                                    .clicked()
                                    {
                                        self.settings.Theme = "dark".into();
                                        theme::set_pref(theme::Pref::Dark);
                                        settings_dirty = true;
                                    }
                                },
                            );
                        },
                    );
                    theme::hairline(ui);
                    ui.add_space(14.0);

                    theme::section_head(ui, "03", "UPDATES");
                    theme::setting_value_row(ui, "Version", update::APP_VERSION);
                    ui.add_space(4.0);
                    let busy = self.update_rx.is_some();
                    let mut install_err: Option<String> = None;
                    match &self.update_state {
                        UpdateUi::Idle => {
                            ui.horizontal(|ui| {
                                ui.add_enabled_ui(!busy, |ui| {
                                    if theme::button_sized(
                                        ui,
                                        "Check now",
                                        theme::BtnKind::Secondary,
                                        120.0,
                                        26.0,
                                    )
                                    .clicked()
                                    {
                                        self.spawn_update_check();
                                    }
                                });
                            });
                        }
                        UpdateUi::Checking => {
                            ui.add_enabled_ui(false, |ui| {
                                theme::button_sized(
                                    ui,
                                    "Checking...",
                                    theme::BtnKind::Secondary,
                                    120.0,
                                    26.0,
                                );
                            });
                        }
                        UpdateUi::Current => {
                            ui.horizontal(|ui| {
                                ui.add_enabled_ui(!busy, |ui| {
                                    if theme::button_sized(
                                        ui,
                                        "Check now",
                                        theme::BtnKind::Secondary,
                                        120.0,
                                        26.0,
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
                                "Download update",
                                theme::BtnKind::Primary,
                                150.0,
                                26.0,
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
                                "Install + Restart",
                                theme::BtnKind::Primary,
                                150.0,
                                26.0,
                            )
                            .clicked()
                            {
                                // The updater exits via process::exit, which
                                // skips on_exit - persist FIRST so the live
                                // chain survives the update restart.
                                self.persist_live();
                                let msg = update::install_and_restart(&stage);
                                // Success exits the process; anything else is
                                // a real failure the user must see.
                                install_err = Some(msg);
                            }
                        }
                    }
                    if let Some(e) = install_err {
                        self.update_state = UpdateUi::Failed(e);
                    }
                    ui.add_space(14.0);

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
                    );

                    if settings_dirty {
                        // Autosave: persist the moment anything changes
                        // (theme included) and apply side effects right away.
                        let _ = save_settings(&self.settings);
                        autostart::apply(self.settings.StartWithWindows);
                        self.guard.set_enabled(self.settings.KeepCableClean);
                    }
                });
        }

'''

s = s[:i0] + new_block + s[i1:]
open(p, "w", encoding="utf-8").write(s)
print("settings window rewritten")
