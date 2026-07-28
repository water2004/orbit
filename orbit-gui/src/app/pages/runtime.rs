use super::super::*;

impl OrbitApp {
    pub(crate) fn show_runtime(&mut self, ui: &mut egui::Ui) {
        if let Some(flow) = self.runtime_flow {
            self.show_runtime_flow(ui, flow);
        } else {
            self.show_runtime_dashboard(ui);
        }
    }

    fn show_runtime_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(tr!("Game installations")).size(25.0).strong());
                ui.label(
                    RichText::new(tr!(
                        "Minecraft, mod loaders, and Java managed as one runtime"
                    ))
                    .color(theme::muted()),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.runtime_instances.is_empty()
                    && ui.add(theme::primary_button("New installation")).clicked()
                {
                    self.begin_runtime_flow(RuntimeFlowMode::Create);
                }
                if ui.add(theme::secondary_button("Import folder")).clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_folder()
                {
                    self.runtime_edit.import_root = path.display().to_string();
                    self.import_runtime();
                }
            });
        });
        ui.add_space(18.0);

        let selected = self.selected_instance().cloned();
        if let Some(instance) = selected.clone() {
            let detail = self.instance_detail.clone();
            theme::elevated_card().show(ui, |ui| {
                ui.horizontal(|ui| {
                    version_badge(
                        ui,
                        detail
                            .as_ref()
                            .and_then(|item| item.installed.as_ref())
                            .map(|item| item.minecraft.as_str())
                            .unwrap_or("MC"),
                        64.0,
                    );
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&instance.name).size(23.0).strong());
                            info_chip(
                                ui,
                                if instance.kind == "server" {
                                    "SERVER"
                                } else {
                                    "CLIENT"
                                },
                                theme::accent(),
                            );
                            if instance.is_default {
                                info_chip(ui, "DEFAULT", theme::success());
                            }
                        });
                        if let Some(installed) =
                            detail.as_ref().and_then(|item| item.installed.as_ref())
                        {
                            let loader = match &installed.loader_version {
                                Some(version) => {
                                    format!("{} {}", title_case(&installed.loader), version)
                                }
                                None => title_case(&installed.loader),
                            };
                            ui.label(
                                RichText::new(format!(
                                    "Minecraft {}   ·   {}   ·   {}",
                                    installed.minecraft,
                                    loader,
                                    installed
                                        .java
                                        .as_ref()
                                        .map(|java| {
                                            format!(
                                                "Java {} · {} {} ({})",
                                                java.major,
                                                java.provider,
                                                java.version,
                                                java.platform
                                            )
                                        })
                                        .unwrap_or_else(|| tr!("Java pending").into_owned())
                                ))
                                .color(theme::muted()),
                            );
                        } else {
                            ui.label(
                                RichText::new(tr!("Runtime files have not been installed yet"))
                                    .color(theme::warning()),
                            );
                        }
                        let path = ui.label(
                            RichText::new(instance.root.display().to_string())
                                .size(11.0)
                                .color(theme::muted()),
                        );
                        if let Some(detail) = &detail {
                            path.on_hover_text(tr!(
                                "Discovered from %{context} context",
                                context = detail.context
                            ));
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let launch_label = if instance.kind == "server" {
                            "Start server"
                        } else {
                            "Launch"
                        };
                        if ui.add(theme::primary_button(launch_label)).clicked() {
                            let (label, command, intent) = if instance.kind == "server" {
                                (
                                    "Starting server",
                                    vec!["server".into(), "start".into()],
                                    Intent::ServerMutated,
                                )
                            } else {
                                ("Launching game", vec!["launch".into()], Intent::Generic)
                            };
                            self.launcher_task_args(
                                label,
                                intent,
                                Some(instance.id.clone()),
                                command,
                                None,
                            );
                        }
                    });
                });
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add(theme::secondary_button("Change version")).clicked() {
                        self.begin_runtime_flow(RuntimeFlowMode::Update);
                    }
                    if ui.add(theme::secondary_button("Verify and repair")).clicked() {
                        self.install_runtime();
                    }
                    if !instance.is_default
                        && ui.add(theme::ghost_button("Make default")).clicked()
                    {
                        self.set_default_runtime();
                    }
                    if instance.kind == "server"
                        && ui.add(theme::ghost_button("Server controls")).clicked()
                    {
                        self.preferences.page = Page::Server;
                    }
                    if ui.add(theme::ghost_button("Unregister")).clicked() {
                        self.confirmation = Some(Confirmation {
                            title: tr!("Unregister %{name}?", name = instance.name),
                            body: tr!("This removes the installation from Orbit Launcher without deleting its game directory.").into_owned(),
                            action: ConfirmationAction::UnregisterInstance(instance.id.clone()),
                        });
                    }
                });
            });
        } else {
            theme::elevated_card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    theme::orbit_mark(ui, 48.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(tr!("Build your first Minecraft installation"))
                                .size(19.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(tr!(
                                "Choose Minecraft and a loader; Java is resolved automatically."
                            ))
                            .color(theme::muted()),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::primary_button("Choose version")).clicked() {
                            self.begin_runtime_flow(RuntimeFlowMode::Create);
                        }
                    });
                });
            });
        }

        if !self.runtime_instances.is_empty() {
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr!("Your installations")).size(19.0).strong());
                ui.label(
                    RichText::new(tr!(
                        "%{count} registered",
                        count = self.runtime_instances.len()
                    ))
                    .size(12.0)
                    .color(theme::muted()),
                );
            });
            ui.add_space(8.0);
            for pair in self.runtime_instances.clone().chunks(2) {
                ui.columns(2, |columns| {
                    for (column, instance) in pair.iter().enumerate() {
                        let selected = self.preferences.selected_instance.as_deref()
                            == Some(instance.id.as_str());
                        let response = egui::Frame::new()
                            .fill(if selected {
                                theme::accent_soft()
                            } else {
                                theme::surface()
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if selected {
                                    theme::accent()
                                } else {
                                    theme::border()
                                },
                            ))
                            .corner_radius(14)
                            .inner_margin(egui::Margin::same(18))
                            .show(&mut columns[column], |ui| {
                                ui.set_min_height(58.0);
                                ui.horizontal(|ui| {
                                    version_badge(
                                        ui,
                                        if instance.kind == "server" {
                                            "SV"
                                        } else {
                                            "MC"
                                        },
                                        42.0,
                                    );
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&instance.name).strong());
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {}",
                                                title_case(&instance.kind),
                                                instance.root.display()
                                            ))
                                            .size(11.0)
                                            .color(theme::muted()),
                                        );
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if selected {
                                            info_chip(ui, "OPEN", theme::accent());
                                        }
                                    });
                                });
                            });
                        if response.response.interact(Sense::click()).clicked() && !selected {
                            self.preferences.selected_instance = Some(instance.id.clone());
                            self.load_selected();
                        }
                    }
                });
                ui.add_space(10.0);
            }
        }

        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("Managed Java")).size(19.0).strong());
            ui.label(
                RichText::new(tr!("Shared, verified runtimes"))
                    .size(12.0)
                    .color(theme::muted()),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::ghost_button("Verify all")).clicked() {
                    self.refresh_java_runtimes(true);
                }
            });
        });
        ui.add_space(8.0);
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if self.java_runtimes.is_empty() {
                ui.horizontal(|ui| {
                    version_badge(ui, "J", 42.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(tr!("Installed automatically when needed")).strong());
                        ui.label(
                            RichText::new(tr!("No managed Java runtime is currently stored"))
                                .color(theme::muted()),
                        );
                    });
                });
            }
            for runtime in self.java_runtimes.clone() {
                ui.horizontal(|ui| {
                    version_badge(ui, &runtime.major.to_string(), 42.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("Java {} · {}", runtime.major, runtime.version))
                                .strong(),
                        );
                        ui.label(
                            RichText::new(tr!(
                                "%{component} · %{platform} · %{files} files · %{bytes}",
                                component = runtime.component,
                                platform = runtime.platform,
                                files = runtime.files,
                                bytes = human_bytes(runtime.bytes)
                            ))
                            .size(11.0)
                            .color(theme::muted()),
                        )
                        .on_hover_text(tr!(
                            "Provider: %{provider}\nRuntime: %{runtime}\nExecutable: %{executable}",
                            provider = runtime.provider,
                            runtime = runtime.root.display(),
                            executable = runtime.executable.display()
                        ));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::ghost_button("Remove")).clicked() {
                            self.confirmation = Some(Confirmation {
                                title: tr!("Remove Java %{major}?", major = runtime.major),
                                body: tr!("The launcher first checks every registered installation and refuses to remove a runtime that is still in use.").into_owned(),
                                action: ConfirmationAction::RemoveJavaRuntime(
                                    runtime.runtime_id.clone(),
                                ),
                            });
                        }
                        if runtime.verified == Some(true) {
                            info_chip(ui, "VERIFIED", theme::success());
                        } else if ui.add(theme::ghost_button("Verify")).clicked() {
                            self.verify_java_runtime(&runtime.runtime_id);
                        }
                    });
                });
                ui.separator();
            }
        });
    }

    fn show_runtime_flow(&mut self, ui: &mut egui::Ui, flow: RuntimeFlow) {
        ui.horizontal(|ui| {
            if ui.add(theme::ghost_button("Back")).clicked() {
                match flow.step {
                    RuntimeFlowStep::Minecraft => self.runtime_flow = None,
                    RuntimeFlowStep::Components => {
                        self.runtime_flow = Some(RuntimeFlow {
                            step: RuntimeFlowStep::Minecraft,
                            ..flow
                        });
                    }
                    RuntimeFlowStep::Review => {
                        self.runtime_flow = Some(RuntimeFlow {
                            step: RuntimeFlowStep::Components,
                            ..flow
                        });
                    }
                }
            }
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(match flow.mode {
                        RuntimeFlowMode::Create => tr!("New installation"),
                        RuntimeFlowMode::Update => tr!("Change game version"),
                    })
                    .size(25.0)
                    .strong(),
                );
                ui.label(
                    RichText::new(tr!("Minecraft first, then loader, then one final review"))
                        .color(theme::muted()),
                );
            });
        });
        ui.add_space(16.0);
        runtime_steps(ui, flow.step);
        ui.add_space(18.0);

        match flow.step {
            RuntimeFlowStep::Minecraft => self.show_minecraft_picker(ui, flow),
            RuntimeFlowStep::Components => self.show_component_picker(ui, flow),
            RuntimeFlowStep::Review => self.show_runtime_review(ui, flow),
        }
    }

    fn show_minecraft_picker(&mut self, ui: &mut egui::Ui, flow: RuntimeFlow) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("Choose Minecraft")).size(20.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for (index, label) in ["Release", "Snapshot", "Historical", "All"]
                    .iter()
                    .enumerate()
                    .rev()
                {
                    if ui
                        .selectable_label(
                            self.minecraft_version_type == index,
                            orbit_i18n::text(label),
                        )
                        .clicked()
                    {
                        self.minecraft_version_type = index;
                    }
                }
            });
        });
        theme::text_field(
            ui,
            &mut self.minecraft_version_filter,
            "Search versions",
            theme::InputWidth::Fill,
        );
        ui.add_space(8.0);

        let needle = self.minecraft_version_filter.trim().to_ascii_lowercase();
        let versions: Vec<_> = self
            .minecraft_versions
            .iter()
            .filter(|version| {
                minecraft_type_matches(version, self.minecraft_version_type)
                    && (needle.is_empty() || version.id.to_ascii_lowercase().contains(&needle))
            })
            .take(160)
            .cloned()
            .collect();

        if versions.is_empty() {
            empty_state(
                ui,
                "No matching Minecraft versions",
                "Change the search or version category.",
            );
            return;
        }

        for version in versions {
            let selected = match flow.mode {
                RuntimeFlowMode::Create => self.new_instance.minecraft == version.id,
                RuntimeFlowMode::Update => self.runtime_edit.minecraft == version.id,
            };
            let response = egui::Frame::new()
                .fill(if selected {
                    theme::accent_soft()
                } else {
                    theme::surface()
                })
                .stroke(Stroke::new(
                    1.0,
                    if selected {
                        theme::accent()
                    } else {
                        theme::border()
                    },
                ))
                .corner_radius(12)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        version_badge(
                            ui,
                            match version.version_type.as_str() {
                                "release" => "R",
                                "snapshot" => "S",
                                _ => "H",
                            },
                            42.0,
                        );
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&version.id).size(17.0).strong());
                                if version.latest_release {
                                    info_chip(ui, "LATEST", theme::success());
                                } else if version.latest_snapshot {
                                    info_chip(ui, "LATEST SNAPSHOT", theme::warning());
                                }
                            });
                            ui.label(
                                RichText::new(format!(
                                    "{} · released {}",
                                    title_case(&version.version_type),
                                    version
                                        .release_time
                                        .get(..10)
                                        .unwrap_or(&version.release_time)
                                ))
                                .size(12.0)
                                .color(theme::muted()),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(if selected {
                                    tr!("Selected")
                                } else {
                                    tr!("Choose")
                                })
                                .color(if selected {
                                    theme::accent_hover()
                                } else {
                                    theme::muted()
                                }),
                            );
                        });
                    });
                });
            if response.response.interact(Sense::click()).clicked() {
                match flow.mode {
                    RuntimeFlowMode::Create => {
                        if self.new_instance.minecraft != version.id {
                            self.new_instance.minecraft = version.id.clone();
                            self.new_instance.loader_version.clear();
                        }
                        let loader = self.new_instance.loader;
                        self.request_runtime_metadata(&version.id, loader);
                    }
                    RuntimeFlowMode::Update => {
                        if self.runtime_edit.minecraft != version.id {
                            self.runtime_edit.minecraft = version.id.clone();
                            self.runtime_edit.loader_version.clear();
                        }
                        let loader = self.runtime_edit.loader;
                        self.request_runtime_metadata(&version.id, loader);
                    }
                }
                self.runtime_flow = Some(RuntimeFlow {
                    step: RuntimeFlowStep::Components,
                    ..flow
                });
            }
            ui.add_space(8.0);
        }
    }

    fn show_component_picker(&mut self, ui: &mut egui::Ui, flow: RuntimeFlow) {
        let (minecraft, selected_loader, selected_loader_version) = match flow.mode {
            RuntimeFlowMode::Create => (
                self.new_instance.minecraft.clone(),
                self.new_instance.loader,
                self.new_instance.loader_version.clone(),
            ),
            RuntimeFlowMode::Update => (
                self.runtime_edit.minecraft.clone(),
                self.runtime_edit.loader,
                self.runtime_edit.loader_version.clone(),
            ),
        };
        let loaders = ["vanilla", "fabric", "forge", "neoforge", "quilt"];

        ui.horizontal(|ui| {
            version_badge(ui, &minecraft, 52.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!("Minecraft {minecraft}"))
                        .size(20.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr!(
                        "Choose one loader. Compatible versions come from its official catalog."
                    ))
                    .color(theme::muted()),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::ghost_button("Change Minecraft")).clicked() {
                    self.runtime_flow = Some(RuntimeFlow {
                        step: RuntimeFlowStep::Minecraft,
                        ..flow
                    });
                }
            });
        });
        ui.add_space(18.0);

        ui.label(RichText::new(tr!("Mod loader")).size(18.0).strong());
        ui.add_space(6.0);
        for (index, loader) in loaders.iter().enumerate() {
            let selected = selected_loader == index;
            let response = selectable_runtime_row(
                ui,
                &title_case(loader),
                match *loader {
                    "vanilla" => "The official game without a mod loader",
                    "fabric" => "Lightweight loader with a broad modern mod ecosystem",
                    "forge" => "Established loader for the Forge ecosystem",
                    "neoforge" => "Modern continuation of the Forge ecosystem",
                    "quilt" => "Fabric-compatible loader with Quilt extensions",
                    _ => "",
                },
                selected,
                None,
            );
            if response.clicked() && !selected {
                match flow.mode {
                    RuntimeFlowMode::Create => {
                        self.new_instance.loader = index;
                        self.new_instance.loader_version.clear();
                    }
                    RuntimeFlowMode::Update => {
                        self.runtime_edit.loader = index;
                        self.runtime_edit.loader_version.clear();
                    }
                }
                self.request_runtime_metadata(&minecraft, index);
            }
            ui.add_space(7.0);
        }

        if selected_loader != 0 {
            ui.add_space(12.0);
            ui.label(
                RichText::new(tr!(
                    "%{loader} version",
                    loader = title_case(loaders[selected_loader])
                ))
                .size(18.0)
                .strong(),
            );
            ui.label(
                RichText::new(tr!("Recommended and stable releases are marked; the exact version is kept in the runtime lock."))
                    .size(12.0)
                    .color(theme::muted()),
            );
            ui.add_space(6.0);
            let key = (loaders[selected_loader].to_string(), minecraft.clone());
            if let Some(versions) = self.loader_version_catalogs.get(&key).cloned() {
                if versions.is_empty() {
                    ui.label(
                        RichText::new(tr!(
                            "No compatible loader release was reported for this Minecraft version."
                        ))
                        .color(theme::warning()),
                    );
                }
                for version in versions.into_iter().take(24) {
                    let tags = loader_version_tags(&version);
                    let selected = selected_loader_version == version.version;
                    let requirement = version.minimum_java_major.map_or_else(
                        || tr!("Compatible release").into_owned(),
                        |major| tr!("Requires at least Java %{major}", major = major),
                    );
                    let response = selectable_runtime_row(
                        ui,
                        &version.version,
                        &requirement,
                        selected,
                        (!tags.is_empty()).then_some(tags.as_str()),
                    );
                    if response.clicked() {
                        match flow.mode {
                            RuntimeFlowMode::Create => {
                                self.new_instance.loader_version = version.version;
                            }
                            RuntimeFlowMode::Update => {
                                self.runtime_edit.loader_version = version.version;
                            }
                        }
                    }
                    ui.add_space(7.0);
                }
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(tr!("Loading compatible loader versions"))
                            .color(theme::muted()),
                    );
                });
            }
        }

        ui.add_space(16.0);
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                version_badge(ui, "J", 40.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr!("Java is resolved automatically")).strong());
                    ui.label(java_requirement_label(
                        self.java_requirements.get(&minecraft),
                    ));
                });
            });
        });

        let loader_ready = selected_loader == 0 || !selected_loader_version.is_empty();
        let java_ready = self
            .java_requirements
            .get(&minecraft)
            .is_some_and(|requirement| requirement.required);
        ui.add_space(16.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(
                    loader_ready && java_ready,
                    theme::primary_button("Review installation"),
                )
                .clicked()
            {
                self.runtime_flow = Some(RuntimeFlow {
                    step: RuntimeFlowStep::Review,
                    ..flow
                });
            }
            if !java_ready {
                ui.label(
                    RichText::new(tr!("Waiting for official Java metadata"))
                        .size(12.0)
                        .color(theme::muted()),
                );
            }
        });
    }

    fn show_runtime_review(&mut self, ui: &mut egui::Ui, flow: RuntimeFlow) {
        let loaders = ["vanilla", "fabric", "forge", "neoforge", "quilt"];
        let (minecraft, loader, loader_version) = match flow.mode {
            RuntimeFlowMode::Create => (
                self.new_instance.minecraft.clone(),
                loaders[self.new_instance.loader].to_string(),
                self.new_instance.loader_version.clone(),
            ),
            RuntimeFlowMode::Update => (
                self.runtime_edit.minecraft.clone(),
                loaders[self.runtime_edit.loader].to_string(),
                self.runtime_edit.loader_version.clone(),
            ),
        };

        ui.label(RichText::new(tr!("Review")).size(20.0).strong());
        ui.label(
            RichText::new(tr!("Only the target state is shown here; the launcher performs one verified transaction."))
                .color(theme::muted()),
        );
        ui.add_space(12.0);

        theme::elevated_card().show(ui, |ui| {
            ui.horizontal(|ui| {
                summary_value(ui, "MINECRAFT", &minecraft);
                ui.separator();
                summary_value(
                    ui,
                    "LOADER",
                    &if loader == "vanilla" {
                        "Vanilla".to_string()
                    } else {
                        format!("{} {}", title_case(&loader), loader_version)
                    },
                );
                ui.separator();
                summary_value(
                    ui,
                    "JAVA",
                    &self
                        .java_requirements
                        .get(&minecraft)
                        .and_then(|requirement| requirement.major)
                        .map(|major| tr!("Java %{major} · managed", major = major))
                        .unwrap_or_else(|| tr!("Resolving").into_owned()),
                );
            });
        });

        ui.add_space(14.0);
        match flow.mode {
            RuntimeFlowMode::Create => {
                theme::card().show(ui, |ui| {
                    ui.label(RichText::new(tr!("Installation details")).size(18.0).strong());
                    ui.label(
                        RichText::new(tr!("These identify the installation; runtime versions remain locked separately."))
                            .size(12.0)
                            .color(theme::muted()),
                    );
                    ui.add_space(10.0);
                    egui::Grid::new("runtime-create-review")
                        .num_columns(2)
                        .spacing([16.0, 12.0])
                        .show(ui, |ui| {
                            ui.label(tr!("Name"));
                            theme::text_field(
                                ui,
                                &mut self.new_instance.name,
                                "My Minecraft",
                                theme::InputWidth::Form,
                            );
                            ui.end_row();

                            ui.label(tr!("Folder"));
                            ui.horizontal(|ui| {
                                theme::text_field(
                                    ui,
                                    &mut self.new_instance.root,
                                    "Choose an empty installation folder",
                                    theme::InputWidth::Form,
                                );
                                if ui.add(theme::secondary_button("Browse")).clicked()
                                    && let Some(path) = rfd::FileDialog::new().pick_folder()
                                {
                                    self.new_instance.root = path.display().to_string();
                                }
                            });
                            ui.end_row();

                            ui.label(tr!("Usage"));
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.new_instance.kind, 0, tr!("Client"));
                                ui.selectable_value(&mut self.new_instance.kind, 1, tr!("Server"));
                            });
                            ui.end_row();
                        });
                });
            }
            RuntimeFlowMode::Update => {
                if let Some(detail) = self.instance_detail.clone() {
                    theme::card().show(ui, |ui| {
                        ui.label(RichText::new(&detail.instance.name).size(18.0).strong());
                        ui.label(
                            RichText::new(tr!(
                                "The installed runtime will be replaced by the selected target."
                            ))
                            .color(theme::muted()),
                        );
                        ui.add_space(10.0);
                        if let Some(installed) = detail.installed {
                            change_row(ui, "Minecraft", &installed.minecraft, &minecraft);
                            change_row(
                                ui,
                                "Loader",
                                &format!(
                                    "{} {}",
                                    title_case(&installed.loader),
                                    installed.loader_version.unwrap_or_default()
                                ),
                                &format!("{} {}", title_case(&loader), loader_version),
                            );
                        } else {
                            ui.label(
                                RichText::new(tr!(
                                    "This installation has no completed runtime yet."
                                ))
                                .color(theme::warning()),
                            );
                        }
                    });
                }
            }
        }

        ui.add_space(18.0);
        let valid = match flow.mode {
            RuntimeFlowMode::Create => {
                !self.new_instance.name.trim().is_empty()
                    && !self.new_instance.root.trim().is_empty()
            }
            RuntimeFlowMode::Update => self.selected_instance().is_some(),
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = match flow.mode {
                RuntimeFlowMode::Create => "Create and install",
                RuntimeFlowMode::Update => "Apply and install",
            };
            if ui
                .add_enabled(valid, theme::primary_button(label))
                .clicked()
            {
                self.runtime_flow = None;
                match flow.mode {
                    RuntimeFlowMode::Create => self.create_runtime(),
                    RuntimeFlowMode::Update => self.configure_runtime_and_install(),
                }
            }
            ui.label(
                RichText::new(tr!("Downloads are verified before activation"))
                    .size(12.0)
                    .color(theme::muted()),
            );
        });
    }

    pub(in crate::app) fn begin_runtime_flow(&mut self, mode: RuntimeFlowMode) {
        if mode == RuntimeFlowMode::Create {
            self.new_instance = NewInstanceForm {
                minecraft: self.latest_minecraft_release.clone().unwrap_or_default(),
                ..NewInstanceForm::default()
            };
            if !self.new_instance.minecraft.is_empty() {
                let minecraft = self.new_instance.minecraft.clone();
                self.request_runtime_metadata(&minecraft, 0);
            }
        } else {
            let Some(detail) = self.instance_detail.clone() else {
                return;
            };
            let target = detail.installed.as_ref();
            self.runtime_edit.name = detail.instance.name;
            self.runtime_edit.minecraft = target
                .map(|installed| installed.minecraft.clone())
                .unwrap_or(detail.desired.minecraft);
            self.runtime_edit.loader = loader_index(
                target
                    .map(|installed| installed.loader.as_str())
                    .unwrap_or(&detail.desired.loader),
            );
            self.runtime_edit.loader_version = target
                .and_then(|installed| installed.loader_version.clone())
                .or(detail.desired.loader_version)
                .unwrap_or_default();
            self.runtime_edit.java_policy = java_policy_index(&detail.desired.java_policy);
        }
        self.runtime_flow = Some(RuntimeFlow {
            mode,
            step: RuntimeFlowStep::Minecraft,
        });
    }
}
