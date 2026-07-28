use super::super::*;

impl OrbitApp {
    pub(crate) fn show_library(&mut self, ui: &mut egui::Ui) {
        let orbit_initialized = self
            .selected_instance()
            .is_some_and(|instance| instance.root.join("orbit.toml").is_file());

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(tr!("Mods")).size(25.0).strong());
                ui.label(
                    RichText::new(tr!(
                        "One resolved package set for the selected installation"
                    ))
                    .color(theme::muted()),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if orbit_initialized {
                    if ui
                        .add(theme::primary_button("Browse compatible mods"))
                        .clicked()
                    {
                        self.preferences.page = Page::Discover;
                    }
                    ui.menu_button(tr!("Workspace actions"), |ui| {
                        if ui.button(tr!("Reload package state")).clicked() {
                            self.reload_packages();
                            ui.close();
                        }
                        if ui.button(tr!("Verify and repair mods")).clicked() {
                            self.install_mods();
                            ui.close();
                        }
                        if ui.button(tr!("Rescan local files")).clicked() {
                            self.sync_instance();
                            ui.close();
                        }
                    });
                }
            });
        });
        ui.add_space(18.0);

        if self.selected_instance().is_none() {
            theme::elevated_card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    theme::orbit_mark(ui, 44.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(tr!("Select a game installation first"))
                                .size(19.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(tr!(
                                "Every mod workspace belongs to one exact Minecraft runtime."
                            ))
                            .color(theme::muted()),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(theme::secondary_button("Open installations"))
                            .clicked()
                        {
                            self.preferences.page = Page::Runtime;
                        }
                    });
                });
            });
            return;
        }

        if !orbit_initialized {
            theme::elevated_card().show(ui, |ui| {
                ui.horizontal(|ui| {
                    version_badge(ui, "01", 54.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(tr!("Connect this installation to Orbit")).size(21.0).strong());
                        ui.label(
                            RichText::new(tr!("The Minecraft and loader versions come from the exact launcher lock. Orbit then scans the current mods and creates its own lock."))
                            .color(theme::muted()),
                        );
                    });
                });
                ui.add_space(18.0);
                ui.columns(3, |columns| {
                    onboarding_step(
                        &mut columns[0],
                        "1",
                        "Read runtime lock",
                        "No loader guessing or filename fallback",
                    );
                    onboarding_step(
                        &mut columns[1],
                        "2",
                        "Scan package JARs",
                        "Package identity comes from JAR metadata",
                    );
                    onboarding_step(
                        &mut columns[2],
                        "3",
                        "Create exact lock",
                        "Ready for install, updates, and audit",
                    );
                });
                ui.add_space(18.0);
                if ui.add(theme::primary_button("Initialize mod workspace")).clicked() {
                    self.initialize_orbit();
                }
            });
            return;
        }

        let root_packages = self.packages.iter().filter(|package| package.root).count();
        ui.columns(3, |columns| {
            metric_card(
                &mut columns[0],
                "Packages",
                self.packages.len().to_string(),
                &tr!("%{count} explicitly installed", count = root_packages),
            );
            metric_card(
                &mut columns[1],
                "Available updates",
                if self.outdated_checked {
                    self.outdated.len().to_string()
                } else {
                    "—".to_string()
                },
                if self.outdated_checked {
                    "Latest solver result"
                } else {
                    "Not checked yet"
                },
            );
            metric_card(
                &mut columns[2],
                "Environment",
                self.selected_instance()
                    .map(|instance| title_case(&instance.kind))
                    .unwrap_or_else(|| tr!("Unknown").into_owned()),
                "JAR declarations are applied",
            );
        });
        ui.add_space(18.0);

        ui.horizontal(|ui| {
            if ui
                .selectable_label(
                    self.mod_view == 0,
                    tr!("Installed  %{count}", count = self.packages.len()),
                )
                .clicked()
            {
                self.mod_view = 0;
            }
            if ui
                .selectable_label(
                    self.mod_view == 1,
                    if self.outdated_checked {
                        tr!("Updates  %{count}", count = self.outdated.len())
                    } else {
                        tr!("Updates").into_owned()
                    },
                )
                .clicked()
            {
                self.mod_view = 1;
            }
        });
        ui.separator();
        ui.add_space(8.0);

        if self.mod_view == 0 {
            theme::text_field(
                ui,
                &mut self.package_filter,
                "Search installed packages",
                theme::InputWidth::Fill,
            );
            ui.add_space(10.0);
            let filter = self.package_filter.trim().to_ascii_lowercase();
            ScrollArea::vertical()
                .id_salt("installed-mods")
                .show(ui, |ui| {
                    let packages: Vec<_> = self
                        .packages
                        .clone()
                        .into_iter()
                        .filter(|package| {
                            filter.is_empty()
                                || package.mod_id.to_ascii_lowercase().contains(&filter)
                                || package.version.to_ascii_lowercase().contains(&filter)
                        })
                        .collect();
                    if packages.is_empty() {
                        empty_state(
                            ui,
                            if self.packages.is_empty() {
                                "No mods installed"
                            } else {
                                "No matching packages"
                            },
                            if self.packages.is_empty() {
                                "Browse compatible projects to build this installation."
                            } else {
                                "Try a different package name or version."
                            },
                        );
                    }
                    for package in packages {
                        let update = self
                            .outdated
                            .iter()
                            .find(|update| update.mod_id == package.mod_id)
                            .cloned();
                        theme::card().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                version_badge(ui, &package_initials(&package.mod_id), 48.0);
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(&package.mod_id).size(17.0).strong(),
                                        );
                                        if package.root {
                                            info_chip(ui, "DIRECT", theme::accent());
                                        } else {
                                            info_chip(ui, "DEPENDENCY", theme::muted());
                                        }
                                        if package.optional {
                                            info_chip(ui, "OPTIONAL", theme::warning());
                                        }
                                    });
                                    ui.label(
                                        RichText::new(tr!(
                                            "%{version} · %{environment} · %{count} dependency/dependencies",
                                            version = package.version,
                                            environment = title_case(&package.environment),
                                            count = package.dependencies.len()
                                        ))
                                        .color(theme::muted()),
                                    );
                                    let source_count = package.remotes.len();
                                    let bundled_count = package.bundled.len();
                                    if source_count > 0 || bundled_count > 0 {
                                        let source_summary = ui.label(
                                            RichText::new(if bundled_count == 0 {
                                                tr!("%{count} source(s)", count = source_count)
                                            } else {
                                                tr!("%{sources} source(s) · contains %{bundled} bundled package(s)", sources = source_count, bundled = bundled_count)
                                            })
                                            .size(11.0)
                                            .color(theme::muted()),
                                        );
                                        if bundled_count > 0 {
                                            source_summary.on_hover_text(
                                                package
                                                    .bundled
                                                    .iter()
                                                    .map(|item| {
                                                        format!("{} {}", item.mod_id, item.version)
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join("\n"),
                                            );
                                        }
                                    }
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.menu_button(tr!("More"), |ui| {
                                        if ui.button(tr!("Manage environment and sources")).clicked() {
                                            self.package_editor =
                                                Some(PackageEditor::new(package.clone()));
                                            ui.close();
                                        }
                                        if ui.button(tr!("Remove from installation")).clicked() {
                                            self.remove_package(&package.mod_id);
                                            ui.close();
                                        }
                                    });
                                    if let Some(update) = update
                                        && ui
                                            .add(theme::secondary_button(tr!("Update to %{version}", version = update.new_version)))
                                            .clicked()
                                    {
                                        self.upgrade_package(&package.mod_id);
                                    }
                                });
                            });
                        });
                        ui.add_space(8.0);
                    }
                });
        } else {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr!("Package updates")).size(19.0).strong());
                    ui.label(
                        RichText::new(tr!("Orbit evaluates complete Pareto-maximal solutions, including required downgrades."))
                        .size(12.0)
                        .color(theme::muted()),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.outdated_checked && !self.outdated.is_empty()
                        && ui.add(theme::primary_button("Update all")).clicked()
                    {
                        self.upgrade_all_packages();
                    }
                    if ui
                        .add(theme::secondary_button(if self.outdated_checked {
                            "Check again"
                        } else {
                            "Check for updates"
                        }))
                        .clicked()
                    {
                        self.run_outdated();
                    }
                });
            });
            ui.add_space(12.0);

            if !self.outdated_checked {
                theme::elevated_card().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(24.0);
                        version_badge(ui, "UP", 52.0);
                        ui.add_space(10.0);
                        ui.label(RichText::new(tr!("No update plan has been calculated")).size(20.0).strong());
                        ui.label(
                            RichText::new(tr!("A check downloads and analyzes candidate JAR metadata before asking you to choose a solution."))
                            .color(theme::muted()),
                        );
                        ui.add_space(24.0);
                    });
                });
            } else if self.outdated.is_empty() {
                theme::elevated_card().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(24.0);
                        version_badge(ui, "OK", 52.0);
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(tr!("Everything is up to date"))
                                .size(20.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(tr!(
                                "No package can be upgraded in the current resolved environment."
                            ))
                            .color(theme::muted()),
                        );
                        ui.add_space(24.0);
                    });
                });
            } else {
                for update in self.outdated.clone() {
                    theme::card().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            version_badge(ui, &package_initials(&update.mod_id), 46.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&update.mod_id).size(17.0).strong());
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&update.current_version)
                                            .color(theme::muted())
                                            .strikethrough(),
                                    );
                                    ui.label(
                                        RichText::new(tr!("to")).size(11.0).color(theme::muted()),
                                    );
                                    ui.label(
                                        RichText::new(&update.new_version)
                                            .strong()
                                            .color(theme::success()),
                                    );
                                });
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.add(theme::secondary_button("Update")).clicked() {
                                    self.upgrade_package(&update.mod_id);
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }
            }

            if !self.outdated_diagnostics.is_empty() {
                ui.add_space(16.0);
                ui.label(
                    RichText::new(tr!("Why some packages stay unchanged"))
                        .size(18.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr!(
                        "These explanations come from the same solver run as the update plan."
                    ))
                    .size(12.0)
                    .color(theme::muted()),
                );
                ui.add_space(6.0);
                for diagnostic in &self.outdated_diagnostics {
                    egui::CollapsingHeader::new(tr!(
                        "%{package} remains at %{version}",
                        package = diagnostic.package,
                        version = diagnostic.selected_version
                    ))
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(tr!(
                                "Candidate %{version} · %{kind}",
                                version = diagnostic.candidate_version,
                                kind = diagnostic.kind
                            ))
                            .color(theme::warning()),
                        );
                        for fact in &diagnostic.facts {
                            ui.label(RichText::new(fact).color(theme::muted()));
                        }
                    });
                }
            }
            for warning in &self.outdated_warnings {
                ui.add_space(6.0);
                ui.label(RichText::new(warning).color(theme::warning()));
            }
        }
    }
}
