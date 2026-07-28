use super::super::*;

impl OrbitApp {
    pub(crate) fn show_settings(&mut self, ui: &mut egui::Ui) {
        theme::section_title(
            ui,
            &tr!("Settings"),
            &tr!("Appearance, language, and desktop integration"),
        );

        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(tr!("Appearance")).size(18.0).strong());
            ui.label(
                RichText::new(
                    tr!("Language, theme, and accent are presentation-only; language is passed explicitly to each CLI."),
                )
                .size(12.0)
                .color(theme::muted()),
            );
            ui.add_space(10.0);
            ui.label(RichText::new(tr!("Language")).strong());
            let previous_language = self.preferences.language;
            ui.horizontal(|ui| {
                for language in orbit_i18n::LanguageMode::ALL {
                    ui.selectable_value(
                        &mut self.preferences.language,
                        language,
                        language.label(),
                    );
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new(tr!("Color mode")).strong());
            let previous_mode = self.preferences.theme_mode;
            ui.horizontal(|ui| {
                for mode in theme::ThemeMode::ALL {
                    ui.selectable_value(&mut self.preferences.theme_mode, mode, mode.label());
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new(tr!("Accent")).strong());
            let previous_accent = self.preferences.accent_theme;
            ui.horizontal(|ui| {
                for accent in theme::AccentTheme::ALL {
                    ui.selectable_value(&mut self.preferences.accent_theme, accent, accent.label());
                }
            });
            if previous_language != self.preferences.language {
                orbit_i18n::install(self.preferences.language);
                if let Err(message) =
                    theme::install_language_fonts(ui.ctx(), self.preferences.language)
                {
                    self.toast = Some((message, theme::warning()));
                }
                ui.ctx().request_repaint();
            }
            if previous_mode != self.preferences.theme_mode
                || previous_accent != self.preferences.accent_theme
            {
                theme::install(
                    ui.ctx(),
                    self.preferences.theme_mode,
                    self.preferences.accent_theme,
                );
            }
        });

        ui.add_space(14.0);
        egui::CollapsingHeader::new(tr!("Advanced integration"))
            .default_open(false)
            .show(ui, |ui| {
                theme::card().show(ui, |ui| {
                    ui.label(
                        RichText::new(tr!("CLI executables")).size(17.0).strong(),
                    );
                    ui.label(
                        RichText::new(
                            tr!("Normally both binaries are installed beside orbit-gui. Override them only for development."),
                        )
                        .size(12.0)
                        .color(theme::muted()),
                    );
                    egui::Grid::new("binary-settings")
                        .num_columns(3)
                        .spacing([12.0, 10.0])
                        .show(ui, |ui| {
                            ui.label("Orbit");
                            ui.label(self.preferences.orbit_binary.display().to_string());
                            if ui.button(tr!("Choose")).clicked()
                                && let Some(path) = rfd::FileDialog::new().pick_file()
                            {
                                self.preferences.orbit_binary = path;
                            }
                            ui.end_row();
                            ui.label(tr!("Orbit Launcher"));
                            ui.label(self.preferences.launcher_binary.display().to_string());
                            if ui.button(tr!("Choose")).clicked()
                                && let Some(path) = rfd::FileDialog::new().pick_file()
                            {
                                self.preferences.launcher_binary = path;
                            }
                            ui.end_row();
                        });
                    if !self.preferences.orbit_binary.is_file() {
                        ui.label(
                            RichText::new(tr!("Orbit executable not found."))
                                .color(theme::danger()),
                        );
                    }
                    if !self.preferences.launcher_binary.is_file() {
                        ui.label(
                            RichText::new(tr!("Orbit Launcher executable not found."))
                                .color(theme::danger()),
                        );
                    }
                    if ui.add(theme::secondary_button(tr!("Reload integration state"))).clicked() {
                        self.refresh_registries();
                    }
                });
                ui.add_space(8.0);
                theme::card().show(ui, |ui| {
                    ui.label(RichText::new(tr!("Orbit registrations")).size(17.0).strong());
                    for instance in &self.orbit_instances {
                        ui.label(
                            RichText::new(format!(
                                "{}{}{} · Minecraft {} · {}",
                                instance.name,
                                if instance.is_default { tr!(" · default") } else { tr!("") },
                                if instance.is_current { tr!(" · current") } else { tr!("") },
                                instance.mc_version,
                                instance.modloader
                            ))
                            .color(theme::muted()),
                        )
                        .on_hover_text(&instance.path);
                    }
                });
            });
    }
}
