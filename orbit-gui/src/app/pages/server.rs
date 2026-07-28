use super::super::*;

impl OrbitApp {
    pub(crate) fn show_server(&mut self, ui: &mut egui::Ui) {
        theme::section_title(
            ui,
            "Server",
            "Managed foreground runtime and crash restart supervisor",
        );
        let Some(instance) = self.selected_instance().cloned() else {
            empty_state(
                ui,
                "No server selected",
                "Select a server runtime from the top bar.",
            );
            return;
        };
        let running = self
            .server_status
            .as_ref()
            .is_some_and(|status| status.running);
        theme::elevated_card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(if running {
                            tr!("RUNNING")
                        } else {
                            tr!("STOPPED")
                        })
                        .color(if running {
                            theme::success()
                        } else {
                            theme::muted()
                        }),
                    );
                    ui.heading(&instance.name);
                    ui.label(
                        RichText::new(instance.root.display().to_string()).color(theme::muted()),
                    );
                    if let Some(state) = self
                        .server_status
                        .as_ref()
                        .and_then(|status| status.state.as_ref())
                    {
                        let generation = state.get("generation").and_then(Value::as_u64);
                        let pid = state.get("pid").and_then(Value::as_u64);
                        ui.label(
                            RichText::new(tr!(
                                "PID %{pid} · generation %{generation}",
                                pid = pid.map_or_else(|| "—".into(), |value| value.to_string()),
                                generation = generation
                                    .map_or_else(|| "—".into(), |value| value.to_string())
                            ))
                            .size(11.0)
                            .color(theme::muted()),
                        );
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if running {
                        if ui.add(theme::danger_button("Stop server")).clicked() {
                            self.launcher_task(
                                "Stopping server",
                                Intent::ServerMutated,
                                Some(instance.id.clone()),
                                ["server", "stop"],
                                None,
                            );
                        }
                    } else if ui.add(theme::primary_button("Start server")).clicked() {
                        self.launcher_task(
                            "Starting server",
                            Intent::ServerMutated,
                            Some(instance.id.clone()),
                            ["server", "start"],
                            None,
                        );
                    }
                });
            });
        });
        ui.add_space(14.0);
        ui.columns(2, |columns| {
            theme::card().show(&mut columns[0], |ui| {
                ui.heading(tr!("Console command"));
                ui.horizontal(|ui| {
                    let response = theme::text_field(
                        ui,
                        &mut self.server_command,
                        "say Hello",
                        theme::InputWidth::Compact,
                    );
                    let submit = ui.button(tr!("Send")).clicked()
                        || response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if submit && running && !self.server_command.trim().is_empty() {
                        let command = std::mem::take(&mut self.server_command);
                        let mut args = vec!["server".into(), "command".into()];
                        args.extend(command.split_whitespace().map(str::to_string));
                        self.launcher_task_args(
                            "Sending server command",
                            Intent::Generic,
                            Some(instance.id.clone()),
                            args,
                            None,
                        );
                    }
                });
            });
            theme::card().show(&mut columns[1], |ui| {
                ui.heading(tr!("Minecraft EULA"));
                ui.label(
                    RichText::new(tr!("View the complete current document before accepting."))
                        .color(theme::muted()),
                );
                if ui.button(tr!("Show EULA")).clicked() {
                    self.launcher_task(
                        "Loading Minecraft EULA",
                        Intent::EulaShow,
                        Some(instance.id),
                        ["server", "eula", "show"],
                        None,
                    );
                }
            });
        });
    }
}
