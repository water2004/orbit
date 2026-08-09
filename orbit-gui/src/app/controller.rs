use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use gpui::{Context, Entity, Window};
use gpui_component::{IndexPath, select::SelectState};
use orbit_machine_protocol::InteractionResponse;
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeroize::Zeroizing;

use super::*;
use crate::process::{BridgeEvent, CliKind, ProcessRequest};
use crate::wire;

const PREFERENCES_FILE: &str = "preferences.json";

pub(super) fn load_preferences() -> Preferences {
    let adjacent = adjacent_binaries();
    let fallback = Preferences {
        page: Page::Home,
        orbit_binary: adjacent.0,
        launcher_binary: adjacent.1,
        selected_instance: None,
        language: orbit_i18n::LanguageMode::default(),
        theme_mode: crate::theme::ThemeMode::default(),
        accent_theme: crate::theme::AccentTheme::default(),
    };
    let Some(path) = preferences_path() else {
        return fallback;
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|document| serde_json::from_str(&document).ok())
        .unwrap_or(fallback)
}

fn preferences_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "Orbit", "Orbit GUI")
        .map(|dirs| dirs.config_dir().join(PREFERENCES_FILE))
}

impl OrbitApp {
    pub(super) fn save_preferences(&self) {
        let Some(path) = preferences_path() else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        if let Ok(document) = serde_json::to_vec_pretty(&self.preferences) {
            let _ = std::fs::write(path, document);
        }
    }

    pub(super) fn refresh_registries(&mut self) {
        if self.preferences.launcher_binary.is_file() {
            self.launcher_task(
                "Loading runtime instances",
                Intent::LauncherInstances,
                None,
                ["instance", "list"],
                None,
            );
            self.launcher_task(
                "Loading Minecraft versions",
                Intent::MinecraftVersions,
                None,
                ["versions", "minecraft"],
                None,
            );
            self.launcher_task(
                "Loading managed Java runtimes",
                Intent::JavaRuntimes,
                None,
                ["java", "list"],
                None,
            );
            self.launcher_task(
                "Loading Yggdrasil providers",
                Intent::YggdrasilProviders,
                None,
                ["config", "yggdrasil", "list"],
                None,
            );
            self.reload_accounts();
            self.launcher_task(
                "Loading launcher settings",
                Intent::LauncherConfig,
                None,
                ["config", "list"],
                None,
            );
            self.launcher_task(
                "Loading Minecraft directory",
                Intent::MinecraftDirectory,
                None,
                ["minecraft", "directory"],
                None,
            );
        } else {
            self.runtime_instances.clear();
            self.instance_detail = None;
            self.accounts_error = Some(tr!(
                "Orbit Launcher was not found at %{path}.",
                path = self.preferences.launcher_binary.display()
            ));
            self.yggdrasil_providers.clear();
            self.java_runtimes.clear();
            self.minecraft_versions.clear();
            self.launcher_config.clear();
            self.minecraft_directory = None;
            self.toast = Some(Toast {
                message: tr!(
                    "Orbit Launcher was not found at %{path}.",
                    path = self.preferences.launcher_binary.display()
                ),
                kind: ToastKind::Warning,
            });
        }
        if self.preferences.orbit_binary.is_file() {
            self.orbit_task(
                "Loading Orbit settings",
                Intent::OrbitConfig,
                ["config", "list"],
            );
        } else {
            self.orbit_config.clear();
            self.orbit_config_path = None;
        }
    }

    pub(super) fn reload_accounts(&mut self) {
        self.launcher_task(
            "Loading accounts",
            Intent::Accounts,
            None,
            ["account", "list"],
            None,
        );
    }

    pub(super) fn load_selected(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        let Some(instance) = self.selected_instance().cloned() else {
            self.instance_detail = None;
            self.packages.clear();
            return;
        };
        self.launcher_task(
            "Loading instance",
            Intent::LauncherInstanceDetail,
            Some(instance.id.clone()),
            ["instance", "show"],
            None,
        );
        if instance.directory.join("orbit.toml").is_file() {
            self.orbit_task_at(
                "Loading installed mods",
                Intent::Packages,
                ["list"],
                Some(instance.directory.clone()),
                None,
            );
        } else {
            self.packages.clear();
        }
        if instance.kind == "server" {
            self.launcher_task(
                "Checking server",
                Intent::ServerStatus,
                Some(instance.id),
                ["server", "status"],
                None,
            );
        }
    }

    pub(super) fn launcher_task<const N: usize>(
        &mut self,
        label: &str,
        intent: Intent,
        instance: Option<String>,
        command: [&str; N],
        initial_stdin: Option<Zeroizing<String>>,
    ) -> TaskId {
        self.launcher_task_args(
            label,
            intent,
            instance,
            command.into_iter().map(str::to_string).collect(),
            initial_stdin,
        )
    }

    pub(super) fn launcher_task_args(
        &mut self,
        label: &str,
        intent: Intent,
        instance: Option<String>,
        command: Vec<String>,
        initial_stdin: Option<Zeroizing<String>>,
    ) -> TaskId {
        let mut args = vec![
            "--language".to_string(),
            self.preferences.language.argument().to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--progress-format".to_string(),
            "ndjson".to_string(),
            "--non-interactive".to_string(),
        ];
        if let Some(instance) = instance {
            args.push("--instance".to_string());
            args.push(instance);
        }
        args.extend(command);
        self.spawn(
            ProcessRequest {
                kind: CliKind::Launcher,
                program: self.preferences.launcher_binary.clone(),
                args,
                working_directory: None,
                label: label.to_string(),
                initial_stdin,
            },
            intent,
        )
    }

    pub(super) fn orbit_task<const N: usize>(
        &mut self,
        label: &str,
        intent: Intent,
        command: [&str; N],
    ) -> TaskId {
        self.orbit_task_at(label, intent, command, None, None)
    }

    pub(super) fn orbit_task_at<const N: usize>(
        &mut self,
        label: &str,
        intent: Intent,
        command: [&str; N],
        working_directory: Option<PathBuf>,
        initial_stdin: Option<Zeroizing<String>>,
    ) -> TaskId {
        self.orbit_task_args(
            label,
            intent,
            command.into_iter().map(str::to_string).collect(),
            working_directory,
            initial_stdin,
        )
    }

    pub(super) fn orbit_task_args(
        &mut self,
        label: &str,
        intent: Intent,
        command: Vec<String>,
        working_directory: Option<PathBuf>,
        initial_stdin: Option<Zeroizing<String>>,
    ) -> TaskId {
        let mut args = vec![
            "--language".to_string(),
            self.preferences.language.argument().to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--progress-format".to_string(),
            "ndjson".to_string(),
        ];
        args.extend(command);
        self.spawn(
            ProcessRequest {
                kind: CliKind::Orbit,
                program: self.preferences.orbit_binary.clone(),
                args,
                working_directory,
                label: label.to_string(),
                initial_stdin,
            },
            intent,
        )
    }

    fn spawn(&mut self, request: ProcessRequest, intent: Intent) -> TaskId {
        let label = orbit_i18n::text(&request.label).into_owned();
        let id = self.bridge.spawn(request);
        self.tasks.insert(id, TaskView::running(id, label));
        self.intents.insert(id, intent);
        id
    }

    pub(super) fn process_events(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let events = self.bridge.drain();
        let images_changed = self.remote_images.drain();
        if events.is_empty() {
            return images_changed;
        }
        let mut reload_selected = false;
        let mut refresh_accounts = false;
        let mut refresh_yggdrasil_providers = false;
        let mut refresh_java_runtimes = false;
        let mut refresh_server = false;
        let mut refresh_registries = false;
        let mut refresh_launcher_config = false;
        let mut refresh_orbit_config = false;
        let mut refresh_minecraft_directory = false;

        for event in events {
            match event {
                BridgeEvent::Progress { task_id, envelope } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        let (completed, total) = wire::progress_numbers(&envelope.data);
                        task.phase = Some(envelope.phase);
                        task.completed = completed;
                        task.total = total;
                        task.status_line = wire::progress_label(&envelope.data);
                    }
                }
                BridgeEvent::MachineError { task_id, envelope } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.error_code = Some(envelope.code);
                        task.error_message = Some(envelope.message.clone());
                        task.status_line = envelope.message;
                    }
                }
                BridgeEvent::Interaction { task_id, envelope } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.status_line = envelope.prompt.clone();
                    }
                    self.interaction = Some(PendingInteraction { task_id, envelope });
                }
                BridgeEvent::ProtocolError { task_id, message } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.error_code = Some("protocol".to_string());
                        task.error_message = Some(message.clone());
                        task.status_line = message;
                    }
                }
                BridgeEvent::SpawnFailed { task_id, message } => {
                    let intent = self.intents.remove(&task_id).unwrap_or(Intent::Generic);
                    if matches!(intent, Intent::Search) {
                        self.search_state = SearchState::Failed(message.clone());
                    }
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.state = TaskState::Failed;
                        task.status_line = message.clone();
                        task.error_message = Some(message);
                    }
                }
                BridgeEvent::Finished {
                    task_id,
                    status,
                    stdout,
                    cancelled,
                } => {
                    let intent = self.intents.remove(&task_id).unwrap_or(Intent::Generic);
                    let result = if status == Some(0) && !cancelled {
                        wire::success_document(&stdout).map(|envelope| envelope.result)
                    } else {
                        Err(anyhow::anyhow!(
                            "{}",
                            tr!(
                                "Command exited with status %{status}",
                                status = status.map_or_else(
                                    || tr!("unknown").into_owned(),
                                    |value| value.to_string()
                                )
                            )
                        ))
                    };
                    match result.and_then(|value| self.apply_result(&intent, value, window, cx)) {
                        Ok(()) => {
                            if let Some(task) = self.tasks.get_mut(&task_id) {
                                task.state = TaskState::Succeeded;
                                task.status_line = tr!("Completed").into_owned();
                            }
                            match intent {
                                Intent::LauncherInstances
                                | Intent::Mutated {
                                    refresh_packages: true,
                                } => reload_selected = true,
                                Intent::RuntimeMutated => refresh_registries = true,
                                Intent::MigrationRegistered { .. } => refresh_registries = true,
                                Intent::RuntimeInstalledAfterUpdate { .. } => {
                                    refresh_registries = true
                                }
                                Intent::AccountMutated => {
                                    refresh_accounts = true;
                                    reload_selected = true;
                                }
                                Intent::YggdrasilProviderMutated => {
                                    refresh_yggdrasil_providers = true
                                }
                                Intent::JavaRuntimeMutated => refresh_java_runtimes = true,
                                Intent::ServerMutated => refresh_server = true,
                                Intent::LauncherConfigMutated => refresh_launcher_config = true,
                                Intent::OrbitConfigMutated => refresh_orbit_config = true,
                                Intent::MinecraftDirectoryMoved => {
                                    refresh_minecraft_directory = true;
                                    refresh_registries = true;
                                }
                                _ => {}
                            }
                        }
                        Err(error) => {
                            let message = self
                                .tasks
                                .get(&task_id)
                                .and_then(|task| task.error_message.clone())
                                .unwrap_or_else(|| error.to_string());
                            let error_code = self
                                .tasks
                                .get(&task_id)
                                .and_then(|task| task.error_code.as_deref());
                            let command_cancelled = error_code == Some("cancelled");
                            if matches!(intent, Intent::Search) {
                                self.search_state = SearchState::Failed(message.clone());
                            }
                            if matches!(intent, Intent::Accounts) {
                                self.accounts_error = Some(message.clone());
                            }
                            if error_code == Some("reauthentication_required") {
                                refresh_accounts = true;
                            }
                            if let Some(task) = self.tasks.get_mut(&task_id) {
                                task.state = completion_failure_state(
                                    cancelled,
                                    command_cancelled,
                                    task.state,
                                );
                                task.status_line = message.clone();
                                task.error_message = Some(message);
                            }
                        }
                    }
                }
            }
        }

        if reload_selected {
            self.load_selected(window, cx);
        }
        if refresh_registries {
            self.refresh_registries();
        }
        if refresh_accounts {
            self.launcher_task(
                "Refreshing accounts",
                Intent::Accounts,
                None,
                ["account", "list"],
                None,
            );
        }
        if refresh_yggdrasil_providers {
            self.launcher_task(
                "Refreshing Yggdrasil providers",
                Intent::YggdrasilProviders,
                None,
                ["config", "yggdrasil", "list"],
                None,
            );
        }
        if refresh_java_runtimes {
            self.refresh_java_runtimes(false);
        }
        if refresh_server && let Some(instance) = self.selected_instance().cloned() {
            self.launcher_task(
                "Refreshing server state",
                Intent::ServerStatus,
                Some(instance.id),
                ["server", "status"],
                None,
            );
        }
        if refresh_launcher_config {
            self.launcher_task(
                "Loading launcher settings",
                Intent::LauncherConfig,
                None,
                ["config", "list"],
                None,
            );
        }
        if refresh_orbit_config {
            self.orbit_task(
                "Loading Orbit settings",
                Intent::OrbitConfig,
                ["config", "list"],
            );
        }
        if refresh_minecraft_directory {
            self.launcher_task(
                "Loading Minecraft directory",
                Intent::MinecraftDirectory,
                None,
                ["minecraft", "directory"],
                None,
            );
        }
        true
    }

    fn apply_result(
        &mut self,
        intent: &Intent,
        result: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        match intent {
            Intent::LauncherInstances => {
                self.runtime_instances = decode::<RuntimeInstanceList>(result)?.instances;
                let selected_exists = self
                    .preferences
                    .selected_instance
                    .as_ref()
                    .is_some_and(|id| self.runtime_instances.iter().any(|item| &item.id == id));
                if !selected_exists {
                    self.preferences.selected_instance = self
                        .runtime_instances
                        .iter()
                        .find(|item| item.is_default)
                        .or_else(|| self.runtime_instances.first())
                        .map(|item| item.id.clone());
                    self.save_preferences();
                }
                let options: Vec<InstanceOption> = self
                    .runtime_instances
                    .iter()
                    .map(|item| InstanceOption {
                        id: item.id.clone(),
                        title: format!("{} · {}", item.name, title_case(&item.kind)).into(),
                    })
                    .collect();
                let selected = self.preferences.selected_instance.clone();
                self.instance_select.update(cx, |state, cx| {
                    state.set_items(options, window, cx);
                    if let Some(selected) = selected {
                        state.set_selected_value(&selected, window, cx);
                    }
                });
            }
            Intent::LauncherInstanceDetail => {
                let detail: RuntimeInstanceDetail = decode(result)?;
                self.instance_detail = Some(detail);
            }
            Intent::Packages => self.packages = decode::<PackageList>(result)?.packages,
            Intent::PackageVersions { package } => {
                let versions: PackageVersions = decode(result)?;
                if versions.package == *package
                    && self
                        .package_editor
                        .as_ref()
                        .is_some_and(|editor| editor.package.mod_id == *package)
                {
                    if let Some(editor) = &mut self.package_editor {
                        editor.policy =
                            PackagePolicyDraft::from_policy(&versions.policy, &versions.string)?;
                    }
                    self.package_versions = Some(versions);
                }
            }
            Intent::Search => {
                let response: SearchResults = decode(result)?;
                self.search_results = response.results;
                self.search_truncated = response.truncated;
                self.search_state = SearchState::Completed;
                for url in self
                    .search_results
                    .iter()
                    .filter_map(|result| result.icon_url.as_deref())
                {
                    self.remote_images.request(url);
                }
            }
            Intent::Outdated => {
                let response: OutdatedResults = decode(result)?;
                self.outdated = response.updates;
                self.outdated_diagnostics = response.diagnostics;
                self.outdated_warnings = response.warnings;
                self.outdated_checked = true;
                self.mod_view = 1;
            }
            Intent::Audit => self.audit = Some(wire::audit_summary(&result)?),
            Intent::Accounts => {
                self.accounts = decode::<AccountList>(result)?.accounts;
                self.accounts_error = None;
            }
            Intent::YggdrasilProviders => {
                self.yggdrasil_providers = decode::<YggdrasilProviderList>(result)?.providers;
                if !self
                    .yggdrasil_providers
                    .iter()
                    .any(|provider| provider.id == self.ygg_provider)
                {
                    self.ygg_provider = self
                        .yggdrasil_providers
                        .first()
                        .map(|provider| provider.id.clone())
                        .unwrap_or_default();
                }
            }
            Intent::JavaRuntimes => {
                let response: JavaRuntimeList = decode(result)?;
                self.java_verification_requested = response.verification_requested;
                self.java_runtimes = response.runtimes;
            }
            Intent::MinecraftVersions => {
                let response: MinecraftVersionCatalog = decode(result)?;
                self.latest_minecraft_release = Some(response.latest_release.clone());
                self.latest_minecraft_snapshot = Some(response.latest_snapshot);
                self.minecraft_versions = response.versions;
                if self.new_instance.minecraft.is_empty() {
                    self.new_instance.minecraft = response.latest_release;
                    let minecraft = self.new_instance.minecraft.clone();
                    self.request_runtime_metadata(&minecraft, self.new_instance.loader);
                }
            }
            Intent::LoaderVersions => {
                let response: LoaderVersionCatalog = decode(result)?;
                self.loader_version_catalogs
                    .insert((response.loader, response.minecraft), response.versions);
            }
            Intent::JavaRequirement => {
                let response: JavaRequirement = decode(result)?;
                self.java_requirements
                    .insert(response.minecraft.clone(), response);
            }
            Intent::ServerStatus => self.server_status = Some(decode(result)?),
            Intent::MicrosoftBegin => {
                let session: MicrosoftDeviceSession = decode(result)?;
                let verification_uri = microsoft_verification_uri(&session)?;
                cx.open_url(verification_uri.as_str());
                self.microsoft_session = Some(session);
            }
            Intent::EulaShow => self.eula_document = Some(result),
            Intent::LauncherConfig => {
                let response: LauncherConfigList = decode(result)?;
                self.launcher_config = response.settings;
            }
            Intent::OrbitConfig => {
                let response: OrbitConfigList = decode(result)?;
                self.orbit_config_path = Some(response.config_path);
                self.orbit_config = response.entries;
            }
            Intent::MinecraftDirectory => {
                self.minecraft_directory = Some(decode(result)?);
            }
            Intent::MigrationSourceExported {
                source_pack,
                state_pack,
                source_id,
                launcher_args,
            } => {
                self.launcher_task_args(
                    "Exporting game state",
                    Intent::MigrationStateExported {
                        source_pack: source_pack.clone(),
                        state_pack: state_pack.clone(),
                        launcher_args: launcher_args.clone(),
                    },
                    Some(source_id.clone()),
                    vec!["export".into(), state_pack.to_string_lossy().into_owned()],
                    None,
                );
            }
            Intent::MigrationStateExported {
                source_pack,
                state_pack,
                launcher_args,
            } => {
                let mut launcher_args = launcher_args.clone();
                launcher_args.extend([
                    "--from".into(),
                    state_pack.to_string_lossy().into_owned(),
                    "--consume-from".into(),
                ]);
                self.launcher_task_args(
                    "Creating migration target",
                    Intent::RuntimeCreatedForMigration {
                        source_pack: source_pack.clone(),
                    },
                    None,
                    launcher_args,
                    None,
                );
            }
            Intent::RuntimeCreatedForMigration { source_pack } => {
                let installed: LauncherInstallResult = decode(result)?;
                self.launcher_task(
                    "Loading migration target",
                    Intent::MigrationTargetResolved {
                        source_pack: source_pack.clone(),
                        target_id: installed.instance_id.clone(),
                    },
                    Some(installed.instance_id),
                    ["instance", "show"],
                    None,
                );
            }
            Intent::MigrationTargetResolved {
                source_pack,
                target_id,
            } => {
                let detail: RuntimeInstanceDetail = decode(result)?;
                self.orbit_task_args(
                    "Checking mod migration",
                    Intent::MigrationChecked {
                        source_pack: source_pack.clone(),
                        target: detail.instance.directory.clone(),
                        target_id: target_id.clone(),
                        target_name: detail.instance.name.clone(),
                    },
                    vec![
                        "migrate".into(),
                        "check".into(),
                        detail.instance.directory.to_string_lossy().into_owned(),
                        "--source-pack".into(),
                        source_pack.to_string_lossy().into_owned(),
                    ],
                    Some(detail.instance.directory.clone()),
                    None,
                );
            }
            Intent::MigrationChecked {
                source_pack,
                target,
                target_id,
                target_name,
            } => {
                let plan: MigrationResult = decode(result)?;
                if plan.subcommand != "check" {
                    anyhow::bail!("migrate check returned an unexpected result");
                }
                self.migration_review = Some(MigrationReview {
                    source_pack: source_pack.clone(),
                    target: target.clone(),
                    target_id: target_id.clone(),
                    target_name: target_name.clone(),
                    plan,
                });
            }
            Intent::MigrationExported {
                target,
                target_id,
                target_name,
            } => {
                let migration: MigrationResult = decode(result)?;
                if migration.export.is_some_and(|export| export.applied) {
                    self.orbit_task_args(
                        "Registering migrated Orbit instance",
                        Intent::MigrationRegistered {
                            target_id: target_id.clone(),
                            target: target.clone(),
                        },
                        vec![
                            "instances".into(),
                            "register".into(),
                            target_name.clone(),
                            target.to_string_lossy().into_owned(),
                        ],
                        None,
                        None,
                    );
                } else {
                    self.toast = Some(Toast {
                        message: tr!(
                            "Migration export was cancelled; the new runtime was left without migrated mods."
                        )
                        .into_owned(),
                        kind: ToastKind::Warning,
                    });
                }
            }
            Intent::MigrationRegistered { target_id, target } => {
                self.preferences.selected_instance = Some(target_id.clone());
                self.save_preferences();
                self.orbit_task_args(
                    "Installing migrated mod environment",
                    Intent::MigrationInstalled {
                        target_id: target_id.clone(),
                    },
                    vec!["install".into()],
                    Some(target.clone()),
                    None,
                );
            }
            Intent::MigrationInstalled { target_id } => {
                self.preferences.selected_instance = Some(target_id.clone());
                self.save_preferences();
            }
            Intent::RuntimeConfiguredForInstall {
                target_id,
                target,
                sync_orbit,
            } => {
                let _: RuntimeInstanceDetail = decode(result)?;
                self.launcher_task(
                    "Installing Loader update",
                    Intent::RuntimeInstalledAfterUpdate {
                        target_id: target_id.clone(),
                        target: target.clone(),
                        sync_orbit: *sync_orbit,
                    },
                    Some(target_id.clone()),
                    ["install"],
                    None,
                );
            }
            Intent::RuntimeInstalledAfterUpdate {
                target_id,
                target,
                sync_orbit,
            } => {
                let _: LauncherInstallResult = decode(result)?;
                if *sync_orbit {
                    self.orbit_task_args(
                        "Synchronizing updated Loader metadata",
                        Intent::RuntimeMutated,
                        vec!["sync".into()],
                        Some(target.clone()),
                        None,
                    );
                } else {
                    self.preferences.selected_instance = Some(target_id.clone());
                    self.save_preferences();
                }
            }
            Intent::ModpackImported { target } => {
                self.orbit_task_args(
                    "Resolving imported modpack",
                    Intent::Mutated {
                        refresh_packages: true,
                    },
                    vec!["fix".into()],
                    Some(target.clone()),
                    None,
                );
            }
            Intent::LauncherConfigMutated
            | Intent::OrbitConfigMutated
            | Intent::MinecraftDirectoryMoved
            | Intent::Mutated { .. }
            | Intent::RuntimeMutated
            | Intent::AccountMutated
            | Intent::YggdrasilProviderMutated
            | Intent::JavaRuntimeMutated
            | Intent::ServerMutated
            | Intent::Generic => {}
        }
        Ok(())
    }

    pub(super) fn request_runtime_metadata(&mut self, minecraft: &str, loader_index: usize) {
        if minecraft.is_empty() {
            return;
        }
        if !self.java_requirements.contains_key(minecraft) {
            self.launcher_task_args(
                "Resolving Java requirement",
                Intent::JavaRequirement,
                None,
                vec![
                    "versions".into(),
                    "java".into(),
                    "--minecraft".into(),
                    minecraft.into(),
                ],
                None,
            );
        }
        let loader = loaders()[loader_index];
        let key = (loader.to_string(), minecraft.to_string());
        if loader != "vanilla" && !self.loader_version_catalogs.contains_key(&key) {
            self.launcher_task_args(
                "Loading compatible loader versions",
                Intent::LoaderVersions,
                None,
                vec![
                    "versions".into(),
                    "loader".into(),
                    "--loader".into(),
                    loader.into(),
                    "--minecraft".into(),
                    minecraft.into(),
                ],
                None,
            );
        }
    }

    pub(super) fn selected_root(&self) -> Option<PathBuf> {
        self.selected_instance().map(|item| item.directory.clone())
    }

    pub(super) fn reload_packages(&mut self) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_at(
                "Loading installed mods",
                Intent::Packages,
                ["list"],
                Some(root),
                None,
            );
        }
    }

    pub(super) fn run_outdated(&mut self) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_at(
                "Checking feasible upgrades",
                Intent::Outdated,
                ["outdated"],
                Some(root),
                None,
            );
        }
    }

    pub(super) fn run_audit(&mut self, report: Option<PathBuf>, mod_filter: String) {
        if let Some(root) = self.selected_root() {
            let threshold = [0_u8, 35, 70]
                .get(self.audit_min_risk)
                .copied()
                .unwrap_or_default();
            let mut command = vec!["audit".into(), "--min-risk".into(), threshold.to_string()];
            if !mod_filter.is_empty() {
                command.extend(["--mod".into(), mod_filter]);
            }
            if let Some(report) = report {
                command.extend(["--report".into(), report.to_string_lossy().into_owned()]);
            }
            self.orbit_task_args(
                "Auditing bytecode compatibility",
                Intent::Audit,
                command,
                Some(root),
                None,
            );
        }
    }

    pub(super) fn install_mods(&mut self) {
        self.orbit_mutation("Installing locked mod environment", vec!["install".into()]);
    }

    pub(super) fn fix_mods(&mut self) {
        self.orbit_mutation("Repairing mod environment", vec!["fix".into()]);
    }

    pub(super) fn sync_instance(&mut self) {
        self.orbit_mutation("Rebuilding local package inventory", vec!["sync".into()]);
    }

    fn orbit_mutation(&mut self, label: &str, command: Vec<String>) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                label,
                Intent::Mutated {
                    refresh_packages: true,
                },
                command,
                Some(root),
                None,
            );
        }
    }

    pub(super) fn initialize_orbit(&mut self) {
        let Some(detail) = self.instance_detail.clone() else {
            self.warn(tr!("Select a Launcher instance first.").into_owned());
            return;
        };
        let Some(installed) = detail.installed else {
            self.warn(tr!("Install the Minecraft runtime before initializing Orbit.").into_owned());
            return;
        };
        if installed.loader == "vanilla" {
            self.warn(
                tr!("Orbit mod management requires Fabric, Quilt, Forge, or NeoForge.")
                    .into_owned(),
            );
            return;
        }
        let Some(loader_version) = installed.loader_version else {
            self.toast = Some(Toast {
                message: tr!("The installed runtime lock has no exact Loader version.")
                    .into_owned(),
                kind: ToastKind::Danger,
            });
            return;
        };
        self.orbit_task_args(
            "Initializing mod workspace",
            Intent::Mutated {
                refresh_packages: true,
            },
            vec![
                "init".into(),
                detail.instance.name,
                "--mc-version".into(),
                installed.minecraft,
                "--modloader".into(),
                installed.loader,
                "--modloader-version".into(),
                loader_version,
            ],
            Some(detail.instance.directory),
            None,
        );
    }

    fn warn(&mut self, message: String) {
        self.toast = Some(Toast {
            message,
            kind: ToastKind::Warning,
        });
    }

    pub(super) fn upgrade_package(&mut self, package: &str) {
        self.orbit_mutation(
            &tr!("Upgrading %{package}", package = package),
            vec!["upgrade".into(), package.into()],
        );
    }

    pub(super) fn set_package_activation(&mut self, package: &str, enabled: bool) {
        self.orbit_mutation(
            &if enabled {
                tr!("Enabling %{package}", package = package)
            } else {
                tr!("Disabling %{package}", package = package)
            },
            vec![
                if enabled { "enable" } else { "disable" }.into(),
                package.into(),
            ],
        );
    }

    pub(super) fn remove_package(&mut self, package: &str) {
        self.orbit_mutation(
            &tr!("Removing %{package}", package = package),
            vec!["remove".into(), package.into()],
        );
    }

    pub(super) fn purge_package(&mut self, package: &str) {
        self.orbit_mutation(
            &tr!("Purging %{package}", package = package),
            vec!["purge".into(), package.into()],
        );
    }

    pub(super) fn upgrade_all_packages(&mut self) {
        self.orbit_mutation("Upgrading mod environment", vec!["upgrade".into()]);
    }

    pub(super) fn set_package_environment(&mut self, package: &str, environment: &str) {
        self.orbit_mutation(
            &tr!("Updating %{package} environment", package = package),
            vec!["env".into(), package.into(), environment.into()],
        );
    }

    pub(super) fn load_package_versions(&mut self, package: &str) {
        self.package_versions = None;
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                &tr!("Loading %{package} versions", package = package),
                Intent::PackageVersions {
                    package: package.to_string(),
                },
                vec!["versions".into(), package.into()],
                Some(root),
                None,
            );
        }
    }

    pub(super) fn apply_package_policy(&mut self, package: &str, policy: Vec<String>) {
        let mut arguments = vec!["constraint".into(), "set".into(), package.into()];
        arguments.extend(policy);
        self.orbit_mutation(
            &tr!("Updating %{package} version policy", package = package),
            arguments,
        );
    }

    pub(super) fn add_package_remote(&mut self, package: &str, provider: &str, locator: &str) {
        self.orbit_mutation(
            &tr!(
                "Adding %{provider} remote to %{package}",
                provider = provider,
                package = package
            ),
            vec![
                "remote".into(),
                "add".into(),
                package.into(),
                provider.into(),
                locator.into(),
            ],
        );
    }

    pub(super) fn remove_package_remote(&mut self, package: &str, index: usize) {
        self.orbit_mutation(
            &tr!("Removing remote from %{package}", package = package),
            vec![
                "remote".into(),
                "remove".into(),
                package.into(),
                "--index".into(),
                index.to_string(),
            ],
        );
    }

    pub(super) fn search_catalog(&mut self, query: String) {
        self.search_results.clear();
        self.search_truncated = false;
        self.search_state = SearchState::Running;
        let mut command = vec!["search".into(), query];
        if let Some(detail) = &self.instance_detail {
            command.extend(["--mc-version".into(), detail.desired.minecraft.clone()]);
            if detail.desired.loader != "vanilla" {
                command.extend(["--modloader".into(), detail.desired.loader.clone()]);
            }
        }
        self.orbit_task_args(
            "Searching mod catalogs",
            Intent::Search,
            command,
            self.selected_root(),
            None,
        );
    }

    pub(super) fn add_search_result(
        &mut self,
        result: &SearchResult,
        environment: usize,
        optional: bool,
        recommended_constraint: bool,
    ) {
        let locator = match result.platform.as_str() {
            "modrinth" => format!("mr:{}", result.project_id),
            "curseforge" => format!("cf:{}", result.project_id),
            _ => result.project_id.clone(),
        };
        let mut command = vec!["add".into(), locator];
        if let Some(environment) = [None, Some("client"), Some("server"), Some("both")]
            .get(environment)
            .copied()
            .flatten()
        {
            command.extend(["--env".into(), environment.into()]);
        }
        if optional {
            command.push("--optional".into());
        }
        if recommended_constraint {
            command.extend([
                "--string".into(),
                orbit_machine_protocol::RECOMMENDED_NEW_PACKAGE_STRING.into(),
            ]);
        }
        self.orbit_mutation(&tr!("Adding %{name}", name = result.name), command);
    }

    pub(super) fn install_runtime(&mut self) {
        if let Some(instance) = self.selected_instance().cloned() {
            self.launcher_task(
                "Installing runtime",
                Intent::RuntimeMutated,
                Some(instance.id),
                ["install"],
                None,
            );
        }
    }

    pub(super) fn create_runtime(&mut self, mode: RuntimeFlowMode) {
        if mode == RuntimeFlowMode::UpdateLoader {
            self.update_loader_runtime();
            return;
        }
        let mut command = vec![
            "install".into(),
            "--new".into(),
            self.new_instance.name.clone(),
            "--kind".into(),
            if self.new_instance.kind == 0 {
                "client"
            } else {
                "server"
            }
            .into(),
            "--minecraft".into(),
            self.new_instance.minecraft.clone(),
            "--loader".into(),
            loaders()[self.new_instance.loader].into(),
        ];
        if self.new_instance.kind == 1 {
            command.extend([
                "--server-directory".into(),
                self.new_instance.server_directory.clone(),
            ]);
        }
        if self.new_instance.loader != 0 {
            command.extend([
                "--loader-version".into(),
                self.new_instance.loader_version.clone(),
            ]);
        }
        if mode == RuntimeFlowMode::Migrate {
            let Some(source) = self.migration_source.clone() else {
                return;
            };
            let Some(source_id) = self
                .runtime_instances
                .iter()
                .find(|instance| instance.directory == source)
                .map(|instance| instance.id.clone())
            else {
                self.toast = Some(Toast {
                    message: tr!("The migration source is no longer registered.").into_owned(),
                    kind: ToastKind::Warning,
                });
                return;
            };
            let source_pack = migration_source_pack_path();
            let state_pack = migration_state_pack_path();
            self.orbit_task_args(
                "Exporting migration source",
                Intent::MigrationSourceExported {
                    source_pack: source_pack.clone(),
                    state_pack,
                    source_id,
                    launcher_args: command,
                },
                vec![
                    "export".into(),
                    source_pack.to_string_lossy().into_owned(),
                    "--format".into(),
                    "zip".into(),
                ],
                Some(source),
                None,
            );
        } else {
            self.launcher_task_args(
                "Creating runtime",
                Intent::RuntimeMutated,
                None,
                command,
                None,
            );
        }
        self.migration_source = None;
    }

    pub(super) fn apply_migration_review(&mut self) {
        let Some(review) = self.migration_review.take() else {
            return;
        };
        let mut command = vec![
            "migrate".into(),
            "export".into(),
            review.target.to_string_lossy().into_owned(),
            "--source-pack".into(),
            review.source_pack.to_string_lossy().into_owned(),
            "--consume-source-pack".into(),
        ];
        if review.plan.summary.removals > 0 {
            // The user already accepted package removal while checking this
            // exact migration. Replay that consent without exposing a second
            // GUI-only workflow or asking the same question twice.
            command.push("--allow-removals".into());
        }
        self.orbit_task_args(
            "Exporting checked mod migration",
            Intent::MigrationExported {
                target: review.target.clone(),
                target_id: review.target_id,
                target_name: review.target_name,
            },
            command,
            Some(review.target),
            None,
        );
    }

    fn update_loader_runtime(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        if self.new_instance.loader == 0 || self.new_instance.loader_version.is_empty() {
            return;
        }
        let sync_orbit = instance.directory.join("orbit.toml").is_file();
        self.launcher_task_args(
            "Configuring Loader update",
            Intent::RuntimeConfiguredForInstall {
                target_id: instance.id.clone(),
                target: instance.directory,
                sync_orbit,
            },
            Some(instance.id),
            vec![
                "instance".into(),
                "configure".into(),
                "--loader-version".into(),
                self.new_instance.loader_version.clone(),
            ],
            None,
        );
    }

    pub(super) fn set_default_runtime(&mut self) {
        if let Some(instance) = self.selected_instance().cloned() {
            self.launcher_task_args(
                "Selecting default instance",
                Intent::RuntimeMutated,
                None,
                vec![
                    "instance".into(),
                    "default".into(),
                    "set".into(),
                    instance.id,
                ],
                None,
            );
        }
    }

    pub(super) fn rename_runtime(&mut self, name: String) {
        if name.is_empty() {
            return;
        }
        if let Some(instance) = self.selected_instance().cloned() {
            self.launcher_task_args(
                "Renaming runtime instance",
                Intent::RuntimeMutated,
                Some(instance.id),
                vec!["instance".into(), "rename".into(), name],
                None,
            );
        }
    }

    pub(super) fn import_runtime(&mut self, root: String) {
        self.launcher_task_args(
            "Importing runtime instance",
            Intent::RuntimeMutated,
            None,
            vec![
                "instance".into(),
                "import".into(),
                "--directory".into(),
                root,
            ],
            None,
        );
    }

    pub(super) fn install_modpack(&mut self, path: PathBuf) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        self.orbit_task_args(
            "Importing modpack",
            Intent::ModpackImported {
                target: instance.directory.clone(),
            },
            vec![
                "import".into(),
                path.to_string_lossy().into_owned(),
                "--merge-strategy".into(),
                "prefer-import".into(),
            ],
            Some(instance.directory),
            None,
        );
    }

    pub(super) fn export_modpack(&mut self, path: PathBuf, format: &'static str) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        self.orbit_task_args(
            if format == "mrpack" {
                "Exporting Modrinth modpack"
            } else {
                "Exporting Orbit modpack"
            },
            Intent::Generic,
            vec![
                "export".into(),
                path.to_string_lossy().into_owned(),
                "--format".into(),
                format.into(),
            ],
            Some(instance.directory),
            None,
        );
    }

    pub(super) fn refresh_java_runtimes(&mut self, verify: bool) {
        let mut command = vec!["java".into(), "list".into()];
        if verify {
            command.push("--verify".into());
        }
        self.launcher_task_args(
            if verify {
                "Verifying managed Java runtimes"
            } else {
                "Loading managed Java runtimes"
            },
            Intent::JavaRuntimes,
            None,
            command,
            None,
        );
    }

    pub(super) fn verify_java_runtime(&mut self, runtime_id: &str) {
        self.launcher_task_args(
            "Verifying managed Java runtime",
            Intent::JavaRuntimeMutated,
            None,
            vec!["java".into(), "verify".into(), runtime_id.into()],
            None,
        );
    }

    pub(super) fn launch_selected(&mut self) {
        if !self.preferences.orbit_binary.is_file() || !self.preferences.launcher_binary.is_file() {
            self.toast = Some(Toast {
                message: tr!("Runtime data-aware launch requires both Orbit and Orbit Launcher.")
                    .into_owned(),
                kind: ToastKind::Warning,
            });
            return;
        }
        if let Some(instance) = self.selected_instance().cloned() {
            let command = joint_launch_arguments(&instance, &self.preferences.launcher_binary);
            let (label, intent) = if instance.kind == "server" {
                ("Starting server", Intent::ServerMutated)
            } else {
                ("Launching game", Intent::Generic)
            };
            self.orbit_task_args(label, intent, command, Some(instance.directory), None);
        }
    }

    pub(super) fn refresh_account(&mut self, account_id: &str) {
        self.launcher_task_args(
            "Refreshing account profile",
            Intent::AccountMutated,
            None,
            vec!["account".into(), "refresh".into(), account_id.into()],
            None,
        );
    }

    pub(super) fn server_action(&mut self, action: &str) {
        if let Some(instance) = self.selected_instance().cloned() {
            if action == "start" {
                self.launch_selected();
                return;
            }
            let (label, intent) = match action {
                "stop" => ("Stopping server", Intent::ServerMutated),
                "eula" => ("Loading Minecraft EULA", Intent::EulaShow),
                _ => return,
            };
            let command = if action == "eula" {
                vec!["server".into(), "eula".into(), "show".into()]
            } else {
                vec!["server".into(), action.into()]
            };
            self.launcher_task_args(label, intent, Some(instance.id), command, None);
        }
    }

    pub(super) fn send_server_command(&mut self, command: String) {
        if let Some(instance) = self.selected_instance().cloned() {
            let mut args = vec!["server".into(), "command".into()];
            args.extend(command.split_whitespace().map(str::to_string));
            self.launcher_task_args(
                "Sending server command",
                Intent::Generic,
                Some(instance.id),
                args,
                None,
            );
        }
    }

    pub(super) fn begin_microsoft_login(&mut self) {
        self.launcher_task(
            "Starting Microsoft sign in",
            Intent::MicrosoftBegin,
            None,
            ["account", "login", "microsoft", "begin"],
            None,
        );
        self.account_flow = None;
    }

    pub(super) fn complete_microsoft_login(&mut self, session: String) {
        self.launcher_task_args(
            "Completing Microsoft sign in",
            Intent::AccountMutated,
            None,
            vec![
                "account".into(),
                "login".into(),
                "microsoft".into(),
                "complete".into(),
                session,
            ],
            None,
        );
        self.microsoft_session = None;
    }

    pub(super) fn create_offline_account(&mut self, name: String) {
        self.launcher_task_args(
            "Creating offline account",
            Intent::AccountMutated,
            None,
            vec!["account".into(), "login".into(), "offline".into(), name],
            None,
        );
        self.account_flow = None;
    }

    pub(super) fn yggdrasil_login(
        &mut self,
        username: String,
        profile: String,
        password: Zeroizing<String>,
    ) {
        let mut command = vec![
            "account".into(),
            "login".into(),
            "yggdrasil".into(),
            "--provider".into(),
            self.ygg_provider.clone(),
            "--username".into(),
            username,
            "--password-stdin".into(),
        ];
        if !profile.is_empty() {
            command.extend(["--profile".into(), profile]);
        }
        self.launcher_task_args(
            "Signing in",
            Intent::AccountMutated,
            None,
            command,
            Some(password),
        );
        self.account_flow = None;
    }

    pub(super) fn add_yggdrasil_provider(&mut self, id: String, root: String) {
        let mut command = vec![
            "config".into(),
            "yggdrasil".into(),
            "add".into(),
            id.clone(),
            root,
        ];
        if self.ygg_allow_insecure_http {
            command.push("--allow-insecure-http".into());
        }
        self.ygg_provider = id;
        self.launcher_task_args(
            "Saving authentication endpoint",
            Intent::YggdrasilProviderMutated,
            None,
            command,
            None,
        );
        self.ygg_endpoint_editor_open = false;
    }

    pub(super) fn select_account(&mut self, account: String, global: bool) {
        let mut command = vec!["account".into(), "select".into(), account];
        let instance = if global {
            command.push("--global".into());
            None
        } else {
            self.selected_instance().map(|item| item.id.clone())
        };
        self.launcher_task_args(
            if global {
                "Selecting default account"
            } else {
                "Selecting installation account"
            },
            Intent::AccountMutated,
            instance,
            command,
            None,
        );
    }

    pub(super) fn clear_account_selection(&mut self, global: bool) {
        let mut command = vec!["account".into(), "clear".into()];
        let instance = if global {
            command.push("--global".into());
            None
        } else {
            self.selected_instance().map(|item| item.id.clone())
        };
        self.launcher_task_args(
            if global {
                "Clearing default account"
            } else {
                "Using the default account for this installation"
            },
            Intent::AccountMutated,
            instance,
            command,
            None,
        );
    }

    pub(super) fn execute_confirmation(&mut self, action: ConfirmationAction) {
        match action {
            ConfirmationAction::LogoutAccount(id) => self.launcher_task_args(
                "Logging out account",
                Intent::AccountMutated,
                None,
                vec!["account".into(), "logout".into(), id],
                None,
            ),
            ConfirmationAction::RemoveYggdrasilProvider(id) => self.launcher_task_args(
                "Removing Yggdrasil provider",
                Intent::YggdrasilProviderMutated,
                None,
                vec!["config".into(), "yggdrasil".into(), "remove".into(), id],
                None,
            ),
            ConfirmationAction::UnregisterInstance(id) => self.launcher_task(
                "Unregistering instance",
                Intent::RuntimeMutated,
                Some(id),
                ["instance", "remove"],
                None,
            ),
            ConfirmationAction::RemoveJavaRuntime(id) => self.launcher_task_args(
                "Removing managed Java runtime",
                Intent::JavaRuntimeMutated,
                None,
                vec!["java".into(), "remove".into(), id],
                None,
            ),
            ConfirmationAction::RemovePackage(id) => {
                self.remove_package(&id);
                0
            }
            ConfirmationAction::CleanOrbitCache => self.orbit_task(
                "Cleaning Orbit JAR cache",
                Intent::Generic,
                ["cache", "clean"],
            ),
            ConfirmationAction::InstallModpack(path) => {
                self.install_modpack(path);
                0
            }
            ConfirmationAction::AcceptEula(digest) => {
                if let Some(instance) = self.selected_instance().cloned() {
                    self.launcher_task_args(
                        "Accepting Minecraft EULA",
                        Intent::ServerMutated,
                        Some(instance.id),
                        vec!["server".into(), "eula".into(), "accept".into(), digest],
                        None,
                    )
                } else {
                    0
                }
            }
        };
    }

    pub(super) fn answer_interaction(&mut self, choice: Option<String>) {
        let Some(pending) = self.interaction.take() else {
            return;
        };
        let interaction_id = pending.envelope.interaction_id;
        let response = match choice {
            Some(choice) => InteractionResponse::selected(interaction_id, choice),
            None => InteractionResponse::cancelled(interaction_id),
        };
        self.bridge.send_line(
            pending.task_id,
            serde_json::to_string(&response).expect("interaction response serializes"),
        );
        if response.cancelled
            && let Some(task) = self.tasks.get_mut(&pending.task_id)
        {
            task.state = TaskState::Cancelled;
            task.status_line = tr!("Cancelled by user").into_owned();
        }
    }

    pub(super) fn set_language(&mut self, language: orbit_i18n::LanguageMode) {
        self.preferences.language = language;
        orbit_i18n::install(language);
        self.save_preferences();
    }

    pub(super) fn set_theme(
        &mut self,
        mode: crate::theme::ThemeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preferences.theme_mode = mode;
        crate::theme::apply(window, cx, mode, self.preferences.accent_theme);
        self.save_preferences();
    }

    pub(super) fn set_accent(
        &mut self,
        accent: crate::theme::AccentTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preferences.accent_theme = accent;
        crate::theme::apply(window, cx, self.preferences.theme_mode, accent);
        self.save_preferences();
    }

    pub(super) fn save_binary_paths(&mut self, orbit: String, launcher: String) {
        self.preferences.orbit_binary = PathBuf::from(orbit);
        self.preferences.launcher_binary = PathBuf::from(launcher);
        self.save_preferences();
        self.refresh_registries();
    }

    pub(super) fn set_launcher_config(&mut self, key: String, value: String) {
        self.launcher_task_args(
            "Saving launcher setting",
            Intent::LauncherConfigMutated,
            None,
            vec!["config".into(), "set".into(), key, value],
            None,
        );
    }

    pub(super) fn unset_launcher_config(&mut self, key: String) {
        self.launcher_task_args(
            "Resetting launcher setting",
            Intent::LauncherConfigMutated,
            None,
            vec!["config".into(), "unset".into(), key],
            None,
        );
    }

    pub(super) fn set_orbit_config(&mut self, key: String, value: String) {
        self.orbit_task_args(
            "Saving Orbit setting",
            Intent::OrbitConfigMutated,
            vec!["config".into(), "set".into(), key, value],
            None,
            None,
        );
    }

    pub(super) fn unset_orbit_config(&mut self, key: String) {
        self.orbit_task_args(
            "Resetting Orbit setting",
            Intent::OrbitConfigMutated,
            vec!["config".into(), "unset".into(), key],
            None,
            None,
        );
    }

    pub(super) fn move_minecraft_directory(&mut self, destination: String) {
        if destination.is_empty() {
            return;
        }
        self.launcher_task_args(
            "Moving Minecraft directory",
            Intent::MinecraftDirectoryMoved,
            None,
            vec!["minecraft".into(), "move".into(), destination],
            None,
        );
    }

    pub(super) fn begin_runtime_flow(&mut self, mode: RuntimeFlowMode) {
        if mode == RuntimeFlowMode::Migrate {
            let Some(source) = self.selected_instance().cloned() else {
                return;
            };
            if !source.directory.join("orbit.toml").is_file() {
                self.toast = Some(Toast {
                    message: tr!("Initialize the selected source with Orbit before migrating it.")
                        .into_owned(),
                    kind: ToastKind::Warning,
                });
                return;
            }
            self.new_instance.kind = usize::from(source.kind == "server");
            self.migration_source = Some(source.directory.clone());
        } else if mode == RuntimeFlowMode::UpdateLoader {
            let Some(detail) = self.instance_detail.clone() else {
                return;
            };
            let Some(loader) = loaders()
                .iter()
                .position(|loader| *loader == detail.desired.loader)
            else {
                self.toast = Some(Toast {
                    message: tr!("The selected installation uses an unsupported Loader.")
                        .into_owned(),
                    kind: ToastKind::Warning,
                });
                return;
            };
            if loader == 0 {
                self.toast = Some(Toast {
                    message: tr!("Vanilla installations do not have a Loader version to update.")
                        .into_owned(),
                    kind: ToastKind::Warning,
                });
                return;
            }
            self.new_instance.kind = usize::from(detail.instance.kind == "server");
            self.new_instance.minecraft = detail.desired.minecraft.clone();
            self.new_instance.loader = loader;
            self.new_instance.loader_version = detail
                .installed
                .and_then(|installed| installed.loader_version)
                .or(detail.desired.loader_version)
                .unwrap_or_default();
            self.migration_source = None;
            let minecraft = self.new_instance.minecraft.clone();
            self.request_runtime_metadata(&minecraft, loader);
        } else {
            self.migration_source = None;
        }
        self.runtime_flow = Some(RuntimeFlow {
            mode,
            step: if mode == RuntimeFlowMode::UpdateLoader {
                RuntimeFlowStep::Components
            } else {
                RuntimeFlowStep::Minecraft
            },
        });
    }

    pub(super) fn choose_directory(
        input: &Entity<gpui_component::input::InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            input.update(cx, |state, cx| {
                state.set_value(path.display().to_string(), window, cx)
            });
        }
    }
}

pub(super) fn loaders() -> [&'static str; 5] {
    ["vanilla", "fabric", "forge", "neoforge", "quilt"]
}

pub(super) fn title_case(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "neoforge" => "NeoForge".to_string(),
        "minecraft" => "Minecraft".to_string(),
        "fabric" => "Fabric".to_string(),
        "forge" => "Forge".to_string(),
        "quilt" => "Quilt".to_string(),
        "vanilla" => "Vanilla".to_string(),
        "client" => tr!("Client").into_owned(),
        "server" => tr!("Server").into_owned(),
        other => {
            let mut chars = other.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        }
    }
}

pub(super) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn adjacent_binaries() -> (PathBuf, PathBuf) {
    let directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    if cfg!(windows) {
        (
            directory.join("orbit.exe"),
            directory.join("orbit-launcher.exe"),
        )
    } else {
        (directory.join("orbit"), directory.join("orbit-launcher"))
    }
}

fn migration_source_pack_path() -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!(
        "orbit-migration-source-{}-{nonce}.zip",
        std::process::id()
    ))
}

fn migration_state_pack_path() -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!(
        "orbit-launcher-migration-state-{}-{nonce}.zip",
        std::process::id()
    ))
}

fn decode<T: DeserializeOwned>(value: Value) -> anyhow::Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

fn microsoft_verification_uri(session: &MicrosoftDeviceSession) -> anyhow::Result<url::Url> {
    let verification_uri = url::Url::parse(&session.verification_uri)
        .map_err(|error| anyhow::anyhow!("invalid Microsoft verification URI: {error}"))?;
    anyhow::ensure!(
        verification_uri.scheme() == "https"
            && verification_uri.host_str().is_some()
            && verification_uri.username().is_empty()
            && verification_uri.password().is_none(),
        "Microsoft verification URI must be an HTTPS web address without embedded credentials"
    );
    Ok(verification_uri)
}

fn completion_failure_state(
    process_cancelled: bool,
    command_cancelled: bool,
    current: TaskState,
) -> TaskState {
    if process_cancelled || command_cancelled || current == TaskState::Cancelled {
        TaskState::Cancelled
    } else {
        TaskState::Failed
    }
}

#[allow(dead_code)]
fn set_select_index<D: gpui_component::select::SelectDelegate + 'static>(
    state: &Entity<SelectState<D>>,
    index: usize,
    window: &mut Window,
    cx: &mut Context<OrbitApp>,
) {
    state.update(cx, |state, cx| {
        state.set_selected_index(Some(IndexPath::default().row(index)), window, cx)
    });
}

fn joint_launch_arguments(instance: &RuntimeInstance, launcher: &Path) -> Vec<String> {
    let mut arguments = vec![
        "launch".into(),
        "--launcher".into(),
        launcher.to_string_lossy().into_owned(),
        "--launcher-instance".into(),
        instance.id.clone(),
    ];
    if instance.kind == "server" {
        arguments.push("--server".into());
    }
    arguments
}

#[cfg(test)]
mod completion_tests {
    use super::{
        MicrosoftDeviceSession, RuntimeInstance, TaskState, completion_failure_state,
        joint_launch_arguments, microsoft_verification_uri,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn structured_cli_cancellation_is_not_presented_as_a_failure() {
        assert_eq!(
            completion_failure_state(false, true, TaskState::Running),
            TaskState::Cancelled
        );
    }

    #[test]
    fn ordinary_nonzero_completion_remains_a_failure() {
        assert_eq!(
            completion_failure_state(false, false, TaskState::Running),
            TaskState::Failed
        );
    }

    #[test]
    fn microsoft_device_session_exposes_a_safe_browser_target() {
        let session: MicrosoftDeviceSession = serde_json::from_value(serde_json::json!({
            "login_session_id": "113d22fb-8c5f-45a9-85bd-78282b78a7a9",
            "verification_uri": "https://microsoft.com/devicelogin",
            "user_code": "ABCD-EFGH",
            "expires_at_unix_seconds": 1,
            "polling_interval_seconds": 5,
            "message": "Use a browser"
        }))
        .unwrap();

        assert_eq!(
            microsoft_verification_uri(&session).unwrap().as_str(),
            "https://microsoft.com/devicelogin"
        );
    }

    #[test]
    fn microsoft_device_session_rejects_a_non_https_browser_target() {
        let session = MicrosoftDeviceSession {
            login_session_id: "session".into(),
            verification_uri: "http://microsoft.com/devicelogin".into(),
            user_code: "ABCD-EFGH".into(),
            message: None,
        };

        assert!(microsoft_verification_uri(&session).is_err());
    }

    #[test]
    fn client_launch_is_routed_through_orbit_with_the_exact_launcher_instance() {
        let instance = runtime_instance("client");
        assert_eq!(
            joint_launch_arguments(&instance, Path::new("C:/Orbit/orbit-launcher.exe")),
            [
                "launch",
                "--launcher",
                "C:/Orbit/orbit-launcher.exe",
                "--launcher-instance",
                "instance-id",
            ]
        );
    }

    #[test]
    fn server_start_uses_the_same_joint_launch_path() {
        let instance = runtime_instance("server");
        let arguments = joint_launch_arguments(&instance, Path::new("C:/Orbit/orbit-launcher.exe"));
        assert_eq!(arguments.last().map(String::as_str), Some("--server"));
        assert_eq!(arguments.first().map(String::as_str), Some("launch"));
    }

    fn runtime_instance(kind: &str) -> RuntimeInstance {
        RuntimeInstance {
            id: "instance-id".into(),
            name: "instance".into(),
            directory: PathBuf::from("C:/Games/instance"),
            minecraft_directory: None,
            kind: kind.into(),
            is_default: false,
        }
    }
}
