use super::super::*;

impl OrbitApp {
    pub(crate) fn show_activity(&mut self, ctx: &egui::Context) {
        let latest = self.tasks.values().next_back().cloned();
        if let Some(task) = latest {
            if !self.activity_open && !matches!(task.state, TaskState::Running | TaskState::Failed)
            {
                return;
            }
            egui::TopBottomPanel::bottom("activity-strip")
                .exact_height(if self.activity_open { 220.0 } else { 66.0 })
                .frame(
                    egui::Frame::new()
                        .fill(theme::sidebar())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .inner_margin(egui::Margin::symmetric(22, 10)),
                )
                .show(ctx, |ui| {
                    theme::apply_ui(ui);
                    ui.horizontal(|ui| {
                        status_dot(ui, task.state);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&task.label).strong());
                            if self.activity_open {
                                ui.label(
                                    RichText::new(&task.command)
                                        .size(10.0)
                                        .color(theme::muted()),
                                );
                            }
                            ui.label(
                                RichText::new(&task.status_line)
                                    .size(12.0)
                                    .color(theme::muted()),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .button(if self.activity_open {
                                    tr!("Hide")
                                } else {
                                    tr!("Activity")
                                })
                                .clicked()
                            {
                                self.activity_open = !self.activity_open;
                            }
                            if task.state == TaskState::Running
                                && ui.button(tr!("Cancel")).clicked()
                            {
                                self.bridge.cancel(task.id);
                            }
                        });
                    });
                    if let (Some(completed), Some(total)) = (task.completed, task.total) {
                        let fraction = if total == 0 {
                            0.0
                        } else {
                            (completed as f32 / total as f32).clamp(0.0, 1.0)
                        };
                        ui.add(egui::ProgressBar::new(fraction).show_percentage());
                    }
                    if self.activity_open {
                        ui.separator();
                        ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                            for task in self.tasks.values().rev().take(20) {
                                ui.horizontal(|ui| {
                                    status_dot(ui, task.state);
                                    ui.label(RichText::new(&task.label).strong());
                                    ui.label(
                                        RichText::new(&task.command)
                                            .size(10.0)
                                            .color(theme::muted()),
                                    );
                                    ui.label(
                                        RichText::new(&task.status_line).color(theme::muted()),
                                    );
                                });
                                if let Some(message) = &task.error_message {
                                    ui.label(
                                        RichText::new(message).size(12.0).color(theme::danger()),
                                    );
                                }
                            }
                        });
                    }
                });
        }
    }

    pub(crate) fn show_overlays(&mut self, ctx: &egui::Context) {
        if let Some(mut editor) = self.package_editor.clone() {
            let mut keep_open = true;
            egui::Window::new(tr!("Manage %{package}", package = editor.package.mod_id))
                .collapsible(false)
                .resizable(true)
                .default_width(620.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                theme::apply_ui(ui);
                    ui.label(
                        RichText::new(format!(
                            "{} · effective environment {}",
                            editor.package.version, editor.package.environment
                        ))
                        .color(theme::muted()),
                    );
                    if !editor.package.root {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(tr!("This is a transitive package. Its source and environment are controlled by the root dependency plan."))
                            .color(theme::warning()),
                        );
                    } else {
                        ui.separator();
                        ui.heading(tr!("Environment filter"));
                        ui.horizontal(|ui| {
                            ComboBox::from_id_salt("package-environment")
                                .selected_text(match editor.environment.as_str() {
                                    "auto" => tr!("Auto (JAR declaration)").into_owned(),
                                    "client" => tr!("Client").into_owned(),
                                    "server" => tr!("Server").into_owned(),
                                    "both" => tr!("Both").into_owned(),
                                    other => other.to_string(),
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut editor.environment,
                                        "auto".into(),
                                        tr!("Auto (JAR declaration)"),
                                    );
                                    ui.selectable_value(
                                        &mut editor.environment,
                                        "client".into(),
                                        tr!("Client"),
                                    );
                                    ui.selectable_value(
                                        &mut editor.environment,
                                        "server".into(),
                                        tr!("Server"),
                                    );
                                    ui.selectable_value(
                                        &mut editor.environment,
                                        "both".into(),
                                        tr!("Both"),
                                    );
                                });
                            let configured = editor
                                .package
                                .configured_environment
                                .as_deref()
                                .unwrap_or("auto");
                            if ui
                                .add_enabled(editor.environment != configured, egui::Button::new(tr!("Apply")))
                                .clicked()
                            {
                                self.set_package_environment(
                                    &editor.package.mod_id,
                                    &editor.environment,
                                );
                                keep_open = false;
                            }
                        });

                        ui.separator();
                        ui.heading(tr!("Package remotes"));
                        for (index, remote) in editor.package.remotes.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(remote).monospace());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let remove = ui.add_enabled(
                                        editor.package.remotes.len() > 1,
                                        egui::Button::new(tr!("Remove")),
                                    );
                                    if remove.clicked() {
                                        self.remove_package_remote(
                                            &editor.package.mod_id,
                                            index + 1,
                                        );
                                        keep_open = false;
                                    }
                                });
                            });
                        }
                        if editor.package.remotes.len() == 1 {
                            ui.label(
                                RichText::new(tr!("The final remote cannot be removed."))
                                    .size(11.0)
                                    .color(theme::muted()),
                            );
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let providers = ["file", "modrinth", "curseforge"];
                            ComboBox::from_id_salt("remote-provider")
                                .selected_text(providers[editor.remote_provider])
                                .show_ui(ui, |ui| {
                                    for (index, provider) in providers.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut editor.remote_provider,
                                            index,
                                            *provider,
                                        );
                                    }
                                });
                            ui.add(
                                TextEdit::singleline(&mut editor.remote_locator)
                                    .hint_text(match editor.remote_provider {
                                        0 => "Local JAR path",
                                        1 => "Modrinth project ID",
                                        _ => "CurseForge numeric project ID",
                                    })
                                    .desired_width(310.0),
                            );
                            if editor.remote_provider == 0 && ui.button(tr!("Browse…")).clicked()
                                && let Some(path) = rfd::FileDialog::new()
                                    .add_filter(tr!("Java archive").into_owned(), &["jar"])
                                    .pick_file()
                            {
                                editor.remote_locator = path.display().to_string();
                            }
                            if ui
                                .add_enabled(
                                    !editor.remote_locator.trim().is_empty(),
                                    egui::Button::new(tr!("Add remote")),
                                )
                                .clicked()
                            {
                                self.add_package_remote(
                                    &editor.package.mod_id,
                                    providers[editor.remote_provider],
                                    editor.remote_locator.trim(),
                                );
                                keep_open = false;
                            }
                        });
                    }
                    ui.separator();
                    if ui.button(tr!("Close")).clicked() {
                        keep_open = false;
                    }
                });
            self.package_editor = keep_open.then_some(editor);
        }
        if let Some(project) = self.project_info.clone() {
            egui::Window::new(&project.name)
                .collapsible(false)
                .default_size([820.0, 650.0])
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    theme::apply_ui(ui);
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(icon) = &project.icon_url {
                                ui.add(
                                    egui::Image::new(icon)
                                        .fit_to_exact_size(Vec2::splat(84.0))
                                        .corner_radius(14),
                                );
                            }
                            ui.vertical(|ui| {
                                ui.heading(&project.name);
                                ui.label(
                                    RichText::new(tr!(
                                        "%{provider} · %{slug} · %{downloads} downloads",
                                        provider = project.provider,
                                        slug = project.slug,
                                        downloads = compact_number(project.downloads)
                                    ))
                                    .color(theme::muted()),
                                );
                                ui.label(tr!(
                                    "Latest %{version} · client %{client} · server %{server}",
                                    version = project.latest_version,
                                    client = project.client_side,
                                    server = project.server_side
                                ));
                                if !project.authors.is_empty() {
                                    ui.label(
                                        RichText::new(tr!(
                                            "By %{authors}",
                                            authors = project.authors.join(", ")
                                        ))
                                        .color(theme::muted()),
                                    );
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.add(theme::primary_button("Add to instance")).clicked() {
                                    let result = SearchResult {
                                        slug: project.slug.clone(),
                                        name: project.name.clone(),
                                        project_id: project.project_id.clone(),
                                        platform: project.provider.clone(),
                                        description: project.description.clone(),
                                        latest_version: project.latest_version.clone(),
                                        downloads: project.downloads,
                                        mc_versions: Vec::new(),
                                        client_side: project.client_side.clone(),
                                        server_side: project.server_side.clone(),
                                        categories: project.categories.clone(),
                                        icon_url: project.icon_url.clone(),
                                        accent_color: project.accent_color,
                                        compatible: None,
                                    };
                                    self.add_search_result(&result);
                                    self.project_info = None;
                                }
                            });
                        });
                        ui.add_space(12.0);
                        ui.label(&project.description);
                        ui.horizontal_wrapped(|ui| {
                            if let Some(license) = &project.license {
                                ui.label(
                                    RichText::new(tr!("License %{license}", license = license))
                                        .color(theme::muted()),
                                );
                            }
                            for category in &project.categories {
                                ui.label(RichText::new(category).color(theme::muted()));
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            project_link(ui, "Website", project.website_url.as_deref());
                            project_link(ui, "Source", project.source_url.as_deref());
                            project_link(ui, "Issues", project.issues_url.as_deref());
                            project_link(ui, "Wiki", project.wiki_url.as_deref());
                        });

                        if !project.gallery.is_empty() {
                            ui.separator();
                            ui.heading(tr!("Gallery"));
                            ui.horizontal_wrapped(|ui| {
                                for image in project.gallery.iter().take(8) {
                                    ui.vertical(|ui| {
                                        ui.add(
                                            egui::Image::new(
                                                image.thumbnail_url.as_ref().unwrap_or(&image.url),
                                            )
                                            .fit_to_exact_size(Vec2::new(220.0, 124.0))
                                            .corner_radius(10),
                                        );
                                        if let Some(title) = &image.title {
                                            ui.label(RichText::new(title).strong());
                                        }
                                        if let Some(description) = &image.description {
                                            ui.label(
                                                RichText::new(description)
                                                    .size(11.0)
                                                    .color(theme::muted()),
                                            );
                                        }
                                    });
                                }
                            });
                        }

                        if !project.recent_versions.is_empty() {
                            ui.separator();
                            ui.heading(tr!("Recent versions"));
                            egui::Grid::new("project-versions")
                                .num_columns(4)
                                .striped(true)
                                .show(ui, |ui| {
                                    ui.strong(tr!("Version"));
                                    ui.strong(tr!("Loader"));
                                    ui.strong(tr!("Minecraft"));
                                    ui.strong(tr!("Released"));
                                    ui.end_row();
                                    for version in project.recent_versions.iter().take(12) {
                                        ui.label(&version.version);
                                        ui.label(&version.loader);
                                        ui.label(version.mc_versions.join(", "));
                                        ui.label(&version.released_at);
                                        ui.end_row();
                                    }
                                });
                        }
                        if !project.dependencies.is_empty() {
                            ui.separator();
                            ui.heading(tr!("Catalog relationships"));
                            for dependency in &project.dependencies {
                                let project = dependency
                                    .slug
                                    .clone()
                                    .or_else(|| dependency.project_id.clone())
                                    .unwrap_or_else(|| tr!("unknown project").into_owned());
                                ui.label(format!(
                                    "{} {}",
                                    if dependency.required {
                                        tr!("Required")
                                    } else {
                                        tr!("Optional")
                                    },
                                    project
                                ));
                            }
                        }
                    });
                    ui.separator();
                    if ui.button(tr!("Close")).clicked() {
                        self.project_info = None;
                    }
                });
        }
        if let Some(pending) = self.interaction.clone() {
            egui::Window::new(match pending.envelope.interaction {
                orbit_machine_protocol::InteractionKind::Package => tr!("Choose package identity"),
                orbit_machine_protocol::InteractionKind::Resolution => {
                    tr!("Choose dependency solution")
                }
                orbit_machine_protocol::InteractionKind::Confirmation => tr!("Review changes"),
            })
            .collapsible(false)
            .resizable(true)
            .default_width(680.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                theme::apply_ui(ui);
                ui.label(RichText::new(&pending.envelope.prompt).size(17.0).strong());
                ui.label(
                    RichText::new(tr!("Review the differences before continuing."))
                        .size(11.0)
                        .color(theme::muted()),
                );
                ui.add_space(10.0);
                ScrollArea::vertical().max_height(430.0).show(ui, |ui| {
                    for choice in &pending.envelope.choices {
                        let is_default =
                            pending.envelope.default_choice.as_deref() == Some(choice.id.as_str());
                        theme::card().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&choice.label).strong());
                                        if is_default {
                                            ui.label(
                                                RichText::new(tr!("DEFAULT"))
                                                    .size(10.0)
                                                    .color(theme::muted()),
                                            );
                                        }
                                    });
                                    if let Some(description) = &choice.description {
                                        ui.label(
                                            RichText::new(description)
                                                .size(12.0)
                                                .color(theme::muted()),
                                        );
                                    }
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(theme::primary_button("Choose")).clicked() {
                                        let response =
                                            orbit_machine_protocol::InteractionResponse::selected(
                                                pending.envelope.interaction_id.clone(),
                                                choice.id.clone(),
                                            );
                                        self.bridge.send_line(
                                            pending.task_id,
                                            serde_json::to_string(&response)
                                                .expect("interaction response is serializable"),
                                        );
                                        self.interaction = None;
                                    }
                                });
                            });
                            render_interaction_data(ui, &choice.data);
                        });
                        ui.add_space(8.0);
                    }
                });
                if pending.envelope.allow_cancel && ui.button(tr!("Cancel operation")).clicked() {
                    let response = orbit_machine_protocol::InteractionResponse::cancelled(
                        pending.envelope.interaction_id,
                    );
                    self.bridge.send_line(
                        pending.task_id,
                        serde_json::to_string(&response)
                            .expect("interaction response is serializable"),
                    );
                    if let Some(task) = self.tasks.get_mut(&pending.task_id) {
                        task.state = TaskState::Cancelled;
                        task.status_line = tr!("Cancelled by user").into_owned();
                    }
                    self.interaction = None;
                }
            });
        }
        if let Some(confirmation) = self.confirmation.clone() {
            egui::Window::new(&confirmation.title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    theme::apply_ui(ui);
                    ui.set_max_width(460.0);
                    ui.label(&confirmation.body);
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::danger_button("Continue")).clicked() {
                            self.confirmation = None;
                            self.execute_confirmation(confirmation.action);
                        }
                        if ui.button(tr!("Cancel")).clicked() {
                            self.confirmation = None;
                        }
                    });
                });
        }
        if let Some(session) = self.microsoft_session.clone() {
            egui::Window::new(tr!("Microsoft sign in"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    theme::apply_ui(ui);
                    let code = session
                        .get("user_code")
                        .and_then(Value::as_str)
                        .unwrap_or("—");
                    let url = session
                        .get("verification_uri")
                        .and_then(Value::as_str)
                        .unwrap_or("https://microsoft.com/devicelogin");
                    ui.label(tr!("Open the Microsoft device page and enter:"));
                    ui.label(
                        RichText::new(code)
                            .size(30.0)
                            .strong()
                            .color(theme::accent()),
                    );
                    ui.hyperlink_to(url, url);
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button("Complete sign in")).clicked()
                            && let Some(id) =
                                session.get("login_session_id").and_then(Value::as_str)
                        {
                            self.launcher_task_args(
                                "Completing Microsoft sign in",
                                Intent::AccountMutated,
                                None,
                                vec![
                                    "account".into(),
                                    "login".into(),
                                    "microsoft".into(),
                                    "complete".into(),
                                    id.into(),
                                ],
                                None,
                            );
                            self.microsoft_session = None;
                        }
                        if ui.button(tr!("Close")).clicked() {
                            self.microsoft_session = None;
                        }
                    });
                });
        }
        if let Some(document) = self.eula_document.clone() {
            egui::Window::new(tr!("Minecraft EULA"))
                .collapsible(false)
                .default_size([720.0, 600.0])
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                theme::apply_ui(ui);
                    let url = document.get("url").and_then(Value::as_str).unwrap_or_default();
                    let digest = document
                        .get("digest_sha256")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    ui.hyperlink_to(tr!("Official document"), url);
                    ui.label(RichText::new(format!("SHA-256 {digest}")).size(11.0).color(theme::muted()));
                    ui.separator();
                    ScrollArea::vertical().max_height(430.0).show(ui, |ui| {
                        ui.label(document.get("text").and_then(Value::as_str).unwrap_or(""));
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button("I agree")).clicked() && !digest.is_empty() {
                            self.confirmation = Some(Confirmation {
                                title: tr!("Accept the Minecraft EULA?").into_owned(),
                                body: tr!("This records acceptance of exactly the displayed document digest for the selected server.").into_owned(),
                                action: ConfirmationAction::AcceptEula(digest.to_string()),
                            });
                            self.eula_document = None;
                        }
                        if ui.button(tr!("Close without accepting")).clicked() {
                            self.eula_document = None;
                        }
                    });
                });
        }
        if let Some((message, color)) = self.toast.clone() {
            egui::Area::new("toast".into())
                .anchor(egui::Align2::RIGHT_TOP, [-24.0, 82.0])
                .show(ctx, |ui| {
                    theme::apply_ui(ui);
                    theme::card().show(ui, |ui| {
                        ui.label(RichText::new(message).color(color));
                        if ui.small_button(tr!("Dismiss")).clicked() {
                            self.toast = None;
                        }
                    });
                });
        }
    }
}
