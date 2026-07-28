use super::super::*;

impl OrbitApp {
    pub(crate) fn show_home(&mut self, ui: &mut egui::Ui) {
        let instance = self.selected_instance().cloned();
        theme::elevated_card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(if instance.is_some() {
                            tr!("READY TO PLAY")
                        } else {
                            tr!("WELCOME TO ORBIT")
                        })
                        .size(11.0)
                        .color(theme::success()),
                    );
                    ui.add_space(4.0);
                    if let Some(instance) = &instance {
                        ui.heading(&instance.name);
                    } else {
                        ui.heading(tr!("No runtime instance selected"));
                    }
                    let subtitle = self.instance_detail.as_ref().map_or_else(
                        || tr!("Create or import a runtime to begin").into_owned(),
                        |detail| {
                            let loader_version = detail
                                .desired
                                .loader_version
                                .clone()
                                .unwrap_or_else(|| tr!("managed").into_owned());
                            tr!(
                                "Minecraft %{minecraft} · %{loader} %{loader_version} · Java %{java}",
                                minecraft = detail.desired.minecraft,
                                loader = detail.desired.loader,
                                loader_version = loader_version,
                                java = detail.desired.java_policy
                            )
                        },
                    );
                    ui.label(RichText::new(subtitle).color(theme::muted()));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if instance.is_none() {
                        if ui.add(theme::primary_button("New installation")).clicked() {
                            self.preferences.page = Page::Runtime;
                            self.begin_runtime_flow(RuntimeFlowMode::Create);
                        }
                        if ui.add(theme::secondary_button("Import folder")).clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            self.runtime_edit.import_root = path.display().to_string();
                            self.import_runtime();
                        }
                        return;
                    }
                    let label = if self.is_server() {
                        "Start server"
                    } else {
                        "Launch game"
                    };
                    if ui
                        .add_enabled(instance.is_some(), theme::primary_button(label))
                        .clicked()
                        && let Some(instance) = instance.clone()
                    {
                        let command = if instance.kind == "server" {
                            vec!["server".into(), "start".into()]
                        } else {
                            vec!["launch".into()]
                        };
                        let intent = if instance.kind == "server" {
                            Intent::ServerMutated
                        } else {
                            Intent::Generic
                        };
                        self.launcher_task_args(label, intent, Some(instance.id), command, None);
                    }
                });
            });
        });
        if instance.is_none() {
            ui.add_space(18.0);
            theme::section_title(
                ui,
                "One workspace, clear responsibilities",
                "Runtime and mod management stay separate behind one native interface",
            );
            ui.columns(3, |columns| {
                capability_card(
                    &mut columns[0],
                    "01",
                    "Game runtime",
                    "Minecraft, mod loaders, and managed Java are installed as one verified runtime.",
                );
                capability_card(
                    &mut columns[1],
                    "02",
                    "Mod workspace",
                    "Logical packages, dependency solutions, updates, and audits come from Orbit.",
                );
                capability_card(
                    &mut columns[2],
                    "03",
                    "Play and serve",
                    "Accounts, client sessions, and crash-restarting servers remain Launcher tasks.",
                );
            });
            return;
        }
        ui.add_space(16.0);
        ui.columns(3, |columns| {
            metric_card(
                &mut columns[0],
                "Installed mods",
                self.packages.len().to_string(),
                "Exact lock state",
            );
            metric_card(
                &mut columns[1],
                "Updates",
                self.outdated.len().to_string(),
                if self.outdated.is_empty() {
                    "Run a fresh check"
                } else {
                    "Feasible upgrades"
                },
            );
            metric_card(
                &mut columns[2],
                "Audit findings",
                self.audit
                    .as_ref()
                    .map_or(0, |audit| audit.findings.len())
                    .to_string(),
                "Bytecode compatibility",
            );
        });
        ui.add_space(16.0);
        theme::section_title(ui, "Quick actions", "The most common instance workflows");
        ui.columns(2, |columns| {
            if quick_action(
                &mut columns[0],
                "01",
                "Find mods",
                "Browse compatible projects",
            ) {
                self.preferences.page = Page::Discover;
            }
            if quick_action(
                &mut columns[1],
                "02",
                "Check updates",
                "Compare the complete mod set",
            ) {
                self.run_outdated();
            }
        });
        ui.add_space(10.0);
        ui.columns(2, |columns| {
            if quick_action(
                &mut columns[0],
                "03",
                "Compatibility",
                "Review bytecode risks",
            ) {
                self.preferences.page = Page::Audit;
                self.run_audit();
            }
            if quick_action(
                &mut columns[1],
                "04",
                "Installation",
                "Versions, loader, and Java",
            ) {
                self.preferences.page = Page::Runtime;
            }
        });
        if !self.outdated.is_empty() {
            ui.add_space(18.0);
            theme::section_title(
                ui,
                "Feasible updates",
                "Each item comes from Orbit's solver portfolio",
            );
            for update in self.outdated.clone().into_iter().take(4) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&update.mod_id).strong());
                    ui.label(
                        RichText::new(format!(
                            "{}  →  {}",
                            update.current_version, update.new_version
                        ))
                        .color(theme::success()),
                    );
                    if ui.small_button(tr!("Upgrade")).clicked() {
                        self.upgrade_package(&update.mod_id);
                    }
                });
            }
        }
    }
}
