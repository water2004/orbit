use super::super::*;

impl OrbitApp {
    pub(crate) fn show_accounts(&mut self, ui: &mut egui::Ui) {
        if let Some(flow) = self.account_flow {
            self.show_account_flow(ui, flow);
            return;
        }

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(tr!("Accounts")).size(23.0).strong());
                ui.label(
                    RichText::new(tr!("Choose who launches the selected client installation"))
                        .color(theme::muted()),
                );
            });
            if !self.accounts.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(theme::primary_button("Add account")).clicked() {
                        self.account_flow = Some(AccountFlow::Choose);
                    }
                });
            }
        });
        ui.add_space(14.0);

        if self.accounts.is_empty() {
            theme::elevated_card().show(ui, |ui| {
                ui.horizontal(|ui| {
                    version_badge(ui, "A", 52.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(tr!("No account added")).size(19.0).strong());
                        ui.label(
                            RichText::new(tr!(
                                "Microsoft, offline, and custom Yggdrasil accounts are supported."
                            ))
                            .color(theme::muted()),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(theme::primary_button("Choose sign-in method"))
                            .clicked()
                        {
                            self.account_flow = Some(AccountFlow::Choose);
                        }
                    });
                });
            });
            return;
        }

        let selected_for_instance = self
            .instance_detail
            .as_ref()
            .and_then(|detail| detail.selected_account_id.clone());
        let selected_instance = self.selected_instance().cloned();

        if let Some(account) = selected_for_instance
            .as_ref()
            .and_then(|id| self.accounts.iter().find(|account| &account.id == id))
            .cloned()
        {
            theme::elevated_card().show(ui, |ui| {
                ui.horizontal(|ui| {
                    account_avatar(ui, &account.profile_name, 54.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(tr!("USED BY THIS INSTALLATION"))
                                .size(10.0)
                                .color(theme::accent()),
                        );
                        ui.label(RichText::new(&account.profile_name).size(20.0).strong());
                        ui.label(
                            RichText::new(tr!(
                                "%{provider} account%{login}",
                                provider = account_provider_label(&account.provider),
                                login = account
                                    .login_name
                                    .as_deref()
                                    .map(|login| format!(" · {login}"))
                                    .unwrap_or_default()
                            ))
                            .color(theme::muted()),
                        );
                    });
                });
            });
            ui.add_space(14.0);
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("All accounts")).size(18.0).strong());
            ui.label(
                RichText::new(tr!("%{count} saved", count = self.accounts.len()))
                    .size(12.0)
                    .color(theme::muted()),
            );
        });
        ui.add_space(7.0);

        for pair in self.accounts.clone().chunks(2) {
            ui.columns(2, |columns| {
                for (column, account) in pair.iter().enumerate() {
                    let used_here = selected_for_instance.as_deref() == Some(account.id.as_str());
                    theme::card().show(&mut columns[column], |ui| {
                        ui.horizontal(|ui| {
                            account_avatar(ui, &account.profile_name, 44.0);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&account.profile_name).size(16.0).strong());
                                    if used_here {
                                        info_chip(ui, "CURRENT", theme::accent());
                                    } else if account.is_default {
                                        info_chip(ui, "DEFAULT", theme::success());
                                    }
                                });
                                ui.label(
                                    RichText::new(format!(
                                        "{}{}",
                                        account_provider_label(&account.provider),
                                        account
                                            .provider_id
                                            .as_deref()
                                            .map(|provider| format!(" · {provider}"))
                                            .unwrap_or_default()
                                    ))
                                    .size(12.0)
                                    .color(theme::muted()),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.menu_button(tr!("Actions"), |ui| {
                                    if selected_instance
                                        .as_ref()
                                        .is_some_and(|instance| instance.kind == "client")
                                        && !used_here
                                        && ui.button(tr!("Use for this installation")).clicked()
                                    {
                                        if let Some(instance) = selected_instance.clone() {
                                            self.launcher_task_args(
                                                "Selecting installation account",
                                                Intent::AccountMutated,
                                                Some(instance.id),
                                                vec![
                                                    "account".into(),
                                                    "select".into(),
                                                    account.id.clone(),
                                                ],
                                                None,
                                            );
                                        }
                                        ui.close();
                                    }
                                    if !account.is_default && ui.button(tr!("Make global default")).clicked() {
                                        self.launcher_task_args(
                                            "Selecting default account",
                                            Intent::AccountMutated,
                                            None,
                                            vec![
                                                "account".into(),
                                                "select".into(),
                                                account.id.clone(),
                                                "--global".into(),
                                            ],
                                            None,
                                        );
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button(tr!("Log out and remove")).clicked() {
                                        self.confirmation = Some(Confirmation {
                                            title: tr!("Log out %{name}?", name = account.profile_name),
                                            body: tr!("The saved session and its local secret will be deleted. Game files are not changed.").into_owned(),
                                            action: ConfirmationAction::LogoutAccount(
                                                account.id.clone(),
                                            ),
                                        });
                                        ui.close();
                                    }
                                });
                            });
                        });
                    });
                }
            });
            ui.add_space(9.0);
        }
    }

    fn show_account_flow(&mut self, ui: &mut egui::Ui, flow: AccountFlow) {
        ui.horizontal(|ui| {
            if ui.add(theme::ghost_button("Back")).clicked() {
                self.account_flow = if flow == AccountFlow::Choose {
                    None
                } else {
                    Some(AccountFlow::Choose)
                };
            }
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(match flow {
                        AccountFlow::Choose => tr!("Add an account"),
                        AccountFlow::Offline => tr!("Offline profile"),
                        AccountFlow::Yggdrasil => tr!("Yggdrasil sign in"),
                    })
                    .size(23.0)
                    .strong(),
                );
                ui.label(
                    RichText::new(match flow {
                        AccountFlow::Choose => tr!("Choose one authentication method"),
                        AccountFlow::Offline => {
                            tr!("Create a local profile without online authentication")
                        }
                        AccountFlow::Yggdrasil => {
                            tr!("Choose or add an authentication endpoint, then sign in")
                        }
                    })
                    .color(theme::muted()),
                );
            });
        });
        ui.add_space(16.0);

        match flow {
            AccountFlow::Choose => {
                ui.columns(3, |columns| {
                    if login_method_card(
                        &mut columns[0],
                        "MS",
                        "Microsoft",
                        "Official Minecraft account using device authorization",
                        true,
                    )
                    .clicked()
                    {
                        self.launcher_task(
                            "Starting Microsoft sign in",
                            Intent::MicrosoftBegin,
                            None,
                            ["account", "login", "microsoft", "begin"],
                            None,
                        );
                        self.account_flow = None;
                    }
                    if login_method_card(
                        &mut columns[1],
                        "OF",
                        "Offline",
                        "Local name only; no online authentication",
                        true,
                    )
                    .clicked()
                    {
                        self.offline_name.clear();
                        self.account_flow = Some(AccountFlow::Offline);
                    }
                    if login_method_card(
                        &mut columns[2],
                        "YG",
                        "Yggdrasil",
                        "Use a standard external authentication endpoint",
                        true,
                    )
                    .clicked()
                    {
                        self.begin_yggdrasil_flow();
                    }
                });
            }
            AccountFlow::Offline => {
                theme::elevated_card().show(ui, |ui| {
                    ui.set_max_width(560.0);
                    ui.label(RichText::new(tr!("Profile name")).strong());
                    ui.label(
                        RichText::new(tr!(
                            "This is the player name visible to offline-mode servers."
                        ))
                        .size(12.0)
                        .color(theme::muted()),
                    );
                    ui.add_space(7.0);
                    ui.add_sized(
                        [420.0, 38.0],
                        TextEdit::singleline(&mut self.offline_name).hint_text(tr!("Player name")),
                    );
                    ui.add_space(10.0);
                    if ui
                        .add_enabled(
                            !self.offline_name.trim().is_empty(),
                            theme::primary_button("Create offline profile"),
                        )
                        .clicked()
                    {
                        let name = self.offline_name.trim().to_string();
                        self.launcher_task_args(
                            "Creating offline account",
                            Intent::AccountMutated,
                            None,
                            vec!["account".into(), "login".into(), "offline".into(), name],
                            None,
                        );
                        self.account_flow = None;
                    }
                });
            }
            AccountFlow::Yggdrasil => {
                theme::elevated_card().show(ui, |ui| {
                    ui.set_max_width(620.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(tr!("Authentication endpoint")).strong());
                            if self.yggdrasil_providers.is_empty() {
                                ui.label(
                                    RichText::new(tr!("No Yggdrasil endpoint is configured yet."))
                                        .size(12.0)
                                        .color(theme::muted()),
                                );
                            } else {
                                ComboBox::from_id_salt("account-yggdrasil-provider")
                                    .width(390.0)
                                    .selected_text(
                                        self.yggdrasil_providers
                                            .iter()
                                            .find(|provider| provider.id == self.ygg_provider)
                                            .map(|provider| provider.id.clone())
                                            .unwrap_or_else(|| tr!("Choose an endpoint").into_owned()),
                                    )
                                    .show_ui(ui, |ui| {
                                        for provider in &self.yggdrasil_providers {
                                            ui.selectable_value(
                                                &mut self.ygg_provider,
                                                provider.id.clone(),
                                                &provider.id,
                                            );
                                        }
                                    });
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::secondary_button(tr!("Add endpoint"))).clicked() {
                                self.open_yggdrasil_endpoint_editor();
                            }
                        });
                    });
                    if let Some(provider) = self
                        .yggdrasil_providers
                        .iter()
                        .find(|provider| provider.id == self.ygg_provider)
                        .cloned()
                    {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&provider.api_root)
                                    .size(12.0)
                                    .color(theme::muted()),
                            );
                            info_chip(
                                ui,
                                if provider.allow_insecure_http {
                                    "INSECURE HTTP"
                                } else {
                                    "HTTPS"
                                },
                                if provider.allow_insecure_http {
                                    theme::warning()
                                } else {
                                    theme::success()
                                },
                            );
                            if ui.add(theme::ghost_button(tr!("Remove endpoint"))).clicked() {
                                self.confirmation = Some(Confirmation {
                                    title: tr!(
                                        "Remove provider %{provider}?",
                                        provider = provider.id
                                    ),
                                    body: tr!("Existing account metadata remains, but its session cannot be refreshed until the service is configured again.").into_owned(),
                                    action: ConfirmationAction::RemoveYggdrasilProvider(
                                        provider.id,
                                    ),
                                });
                            }
                        });
                    }
                    if self.ygg_endpoint_editor_open {
                        self.show_yggdrasil_endpoint_editor(ui);
                    }
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label(RichText::new(tr!("Username")).strong());
                    ui.add_sized(
                        [440.0, 38.0],
                        TextEdit::singleline(&mut self.ygg_username)
                            .hint_text(tr!("Email or username")),
                    );
                    ui.label(RichText::new(tr!("Password")).strong());
                    ui.add_sized(
                        [440.0, 38.0],
                        TextEdit::singleline(&mut *self.ygg_password)
                            .password(true)
                            .hint_text(tr!("Password")),
                    );
                    egui::CollapsingHeader::new(tr!("Choose a specific game profile"))
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(tr!(
                                    "Leave empty to use the service's default profile."
                                ))
                                .size(12.0)
                                .color(theme::muted()),
                            );
                            ui.add_sized(
                                [440.0, 38.0],
                                TextEdit::singleline(&mut self.ygg_profile)
                                    .hint_text(tr!("Profile name or UUID")),
                            );
                        });
                    ui.add_space(10.0);
                    let endpoint_selected = self
                        .yggdrasil_providers
                        .iter()
                        .any(|provider| provider.id == self.ygg_provider);
                    if ui
                        .add_enabled(
                            endpoint_selected
                                && !self.ygg_username.trim().is_empty()
                                && !self.ygg_password.is_empty(),
                            theme::primary_button("Sign in"),
                        )
                        .clicked()
                    {
                        let password = std::mem::take(&mut *self.ygg_password);
                        let mut command = vec![
                            "account".into(),
                            "login".into(),
                            "yggdrasil".into(),
                            "--provider".into(),
                            self.ygg_provider.clone(),
                            "--username".into(),
                            self.ygg_username.trim().to_string(),
                            "--password-stdin".into(),
                        ];
                        if !self.ygg_profile.trim().is_empty() {
                            command
                                .extend(["--profile".into(), self.ygg_profile.trim().to_string()]);
                        }
                        self.launcher_task_args(
                            "Signing in",
                            Intent::AccountMutated,
                            None,
                            command,
                            Some(Zeroizing::new(password)),
                        );
                        self.account_flow = None;
                    }
                });
            }
        }
    }

    fn begin_yggdrasil_flow(&mut self) {
        self.ygg_provider = self
            .yggdrasil_providers
            .first()
            .map(|provider| provider.id.clone())
            .unwrap_or_default();
        self.ygg_endpoint_editor_open = self.yggdrasil_providers.is_empty();
        if self.ygg_endpoint_editor_open {
            self.reset_yggdrasil_endpoint_editor();
        }
        self.account_flow = Some(AccountFlow::Yggdrasil);
    }

    fn open_yggdrasil_endpoint_editor(&mut self) {
        self.reset_yggdrasil_endpoint_editor();
        self.ygg_endpoint_editor_open = true;
    }

    fn reset_yggdrasil_endpoint_editor(&mut self) {
        self.ygg_new_provider_id.clear();
        self.ygg_api_root.clear();
        self.ygg_allow_insecure_http = false;
    }

    fn show_yggdrasil_endpoint_editor(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(theme::surface_high())
            .stroke(Stroke::new(1.0, theme::border()))
            .corner_radius(9)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.label(RichText::new(tr!("New authentication endpoint")).strong());
                ui.label(
                    RichText::new(tr!(
                        "The endpoint is saved globally and can be reused by other accounts."
                    ))
                    .size(12.0)
                    .color(theme::muted()),
                );
                egui::Grid::new("account-yggdrasil-endpoint-editor")
                    .num_columns(2)
                    .spacing([12.0, 9.0])
                    .show(ui, |ui| {
                        ui.label(tr!("Endpoint name"));
                        ui.add(
                            TextEdit::singleline(&mut self.ygg_new_provider_id)
                                .hint_text("my-service"),
                        );
                        ui.end_row();
                        ui.label(tr!("API root"));
                        ui.add(
                            TextEdit::singleline(&mut self.ygg_api_root)
                                .hint_text("https://auth.example.com/api/yggdrasil")
                                .desired_width(390.0),
                        );
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut self.ygg_allow_insecure_http,
                    tr!("Allow unencrypted HTTP (credentials can be intercepted)"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.ygg_new_provider_id.trim().is_empty()
                                && !self.ygg_api_root.trim().is_empty(),
                            theme::primary_button(tr!("Save endpoint")),
                        )
                        .clicked()
                    {
                        let provider_id = self.ygg_new_provider_id.trim().to_string();
                        let mut command = vec![
                            "config".into(),
                            "yggdrasil".into(),
                            "add".into(),
                            provider_id.clone(),
                            self.ygg_api_root.trim().to_string(),
                        ];
                        if self.ygg_allow_insecure_http {
                            command.push("--allow-insecure-http".into());
                        }
                        self.ygg_provider = provider_id;
                        self.launcher_task_args(
                            &tr!("Saving authentication endpoint"),
                            Intent::YggdrasilProviderMutated,
                            None,
                            command,
                            None,
                        );
                        self.ygg_endpoint_editor_open = false;
                    }
                    if ui.add(theme::ghost_button(tr!("Cancel"))).clicked() {
                        self.ygg_endpoint_editor_open = false;
                    }
                });
            });
    }
}
