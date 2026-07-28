use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Color32, ComboBox, Layout, RichText, ScrollArea, Sense, Stroke, Vec2,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::model::*;
use crate::process::{BridgeEvent, CliKind, ProcessBridge, ProcessRequest, TaskId};
use crate::{theme, wire};

mod pages;

const STORAGE_KEY: &str = "orbit-gui-preferences-v1";

#[derive(Debug, Clone)]
enum ConfirmationAction {
    LogoutAccount(String),
    RemoveYggdrasilProvider(String),
    UnregisterInstance(String),
    RemoveJavaRuntime(String),
    AcceptEula(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RuntimeFlowMode {
    Create,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RuntimeFlowStep {
    Minecraft,
    Components,
    Review,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeFlow {
    mode: RuntimeFlowMode,
    step: RuntimeFlowStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountFlow {
    Choose,
    Offline,
    YggdrasilEndpoints,
    YggdrasilLogin,
}

#[derive(Debug, Clone, Default)]
enum SearchState {
    #[default]
    Idle,
    Running,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone)]
struct Confirmation {
    title: String,
    body: String,
    action: ConfirmationAction,
}

#[derive(Default)]
struct NewInstanceForm {
    name: String,
    root: String,
    kind: usize,
    minecraft: String,
    loader: usize,
    loader_version: String,
}

#[derive(Default)]
struct RuntimeEditForm {
    name: String,
    minecraft: String,
    loader: usize,
    loader_version: String,
    java_policy: usize,
    import_root: String,
}

#[derive(Debug, Clone)]
struct PackageEditor {
    package: InstalledPackage,
    environment: String,
    remote_provider: usize,
    remote_locator: String,
}

impl PackageEditor {
    fn new(package: InstalledPackage) -> Self {
        Self {
            environment: package
                .configured_environment
                .clone()
                .unwrap_or_else(|| "auto".to_string()),
            package,
            remote_provider: 0,
            remote_locator: String::new(),
        }
    }
}

pub struct OrbitApp {
    preferences: Preferences,
    bridge: ProcessBridge,
    tasks: BTreeMap<TaskId, TaskView>,
    intents: HashMap<TaskId, Intent>,
    runtime_instances: Vec<RuntimeInstance>,
    orbit_instances: Vec<OrbitInstance>,
    instance_detail: Option<RuntimeInstanceDetail>,
    packages: Vec<InstalledPackage>,
    package_filter: String,
    mod_view: usize,
    search_query: String,
    search_results: Vec<SearchResult>,
    search_truncated: bool,
    search_state: SearchState,
    package_editor: Option<PackageEditor>,
    outdated: Vec<OutdatedPackage>,
    outdated_checked: bool,
    outdated_diagnostics: Vec<ResolutionDiagnostic>,
    outdated_warnings: Vec<String>,
    accounts: Vec<Account>,
    yggdrasil_providers: Vec<YggdrasilProvider>,
    java_runtimes: Vec<JavaRuntime>,
    java_verification_requested: bool,
    minecraft_versions: Vec<MinecraftVersion>,
    latest_minecraft_release: Option<String>,
    latest_minecraft_snapshot: Option<String>,
    minecraft_version_filter: String,
    minecraft_version_type: usize,
    loader_version_catalogs: HashMap<(String, String), Vec<LoaderVersion>>,
    java_requirements: HashMap<String, JavaRequirement>,
    server_status: Option<ServerStatus>,
    audit: Option<AuditSummary>,
    activity_open: bool,
    first_refresh_done: bool,
    confirmation: Option<Confirmation>,
    interaction: Option<PendingInteraction>,
    toast: Option<(String, Color32)>,
    new_instance: NewInstanceForm,
    runtime_edit: RuntimeEditForm,
    offline_name: String,
    ygg_provider: String,
    ygg_username: String,
    ygg_profile: String,
    ygg_password: Zeroizing<String>,
    ygg_new_provider_id: String,
    ygg_api_root: String,
    ygg_allow_insecure_http: bool,
    microsoft_session: Option<Value>,
    eula_document: Option<Value>,
    server_command: String,
    runtime_flow: Option<RuntimeFlow>,
    account_flow: Option<AccountFlow>,
    ygg_endpoint_editor_open: bool,
}

impl OrbitApp {
    pub fn new(creation: &eframe::CreationContext<'_>) -> Self {
        let adjacent = adjacent_binaries();
        let preferences = creation
            .storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_else(|| Preferences {
                page: Page::Home,
                orbit_binary: adjacent.0,
                launcher_binary: adjacent.1,
                selected_instance: None,
                language: orbit_i18n::LanguageMode::default(),
                theme_mode: theme::ThemeMode::default(),
                accent_theme: theme::AccentTheme::default(),
            });
        orbit_i18n::install(preferences.language);
        let font_error =
            theme::install_language_fonts(&creation.egui_ctx, preferences.language).err();
        theme::install(
            &creation.egui_ctx,
            preferences.theme_mode,
            preferences.accent_theme,
        );
        let mut app = Self {
            preferences,
            bridge: ProcessBridge::default(),
            tasks: BTreeMap::new(),
            intents: HashMap::new(),
            runtime_instances: Vec::new(),
            orbit_instances: Vec::new(),
            instance_detail: None,
            packages: Vec::new(),
            package_filter: String::new(),
            mod_view: 0,
            search_query: String::new(),
            search_results: Vec::new(),
            search_truncated: false,
            search_state: SearchState::Idle,
            package_editor: None,
            outdated: Vec::new(),
            outdated_checked: false,
            outdated_diagnostics: Vec::new(),
            outdated_warnings: Vec::new(),
            accounts: Vec::new(),
            yggdrasil_providers: Vec::new(),
            java_runtimes: Vec::new(),
            java_verification_requested: false,
            minecraft_versions: Vec::new(),
            latest_minecraft_release: None,
            latest_minecraft_snapshot: None,
            minecraft_version_filter: String::new(),
            minecraft_version_type: 0,
            loader_version_catalogs: HashMap::new(),
            java_requirements: HashMap::new(),
            server_status: None,
            audit: None,
            activity_open: false,
            first_refresh_done: false,
            confirmation: None,
            interaction: None,
            toast: font_error.map(|message| (message, theme::warning())),
            new_instance: NewInstanceForm::default(),
            runtime_edit: RuntimeEditForm::default(),
            offline_name: String::new(),
            ygg_provider: String::new(),
            ygg_username: String::new(),
            ygg_profile: String::new(),
            ygg_password: Zeroizing::new(String::new()),
            ygg_new_provider_id: String::new(),
            ygg_api_root: String::new(),
            ygg_allow_insecure_http: false,
            microsoft_session: None,
            eula_document: None,
            server_command: String::new(),
            runtime_flow: None,
            account_flow: None,
            ygg_endpoint_editor_open: false,
        };
        app.refresh_registries();
        app
    }

    fn refresh_registries(&mut self) {
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
            self.launcher_task(
                "Loading accounts",
                Intent::Accounts,
                None,
                ["account", "list"],
                None,
            );
        }
        if self.preferences.orbit_binary.is_file() {
            self.orbit_task(
                "Loading Orbit instances",
                Intent::OrbitInstances,
                ["instances", "list"],
            );
        }
    }

    fn selected_instance(&self) -> Option<&RuntimeInstance> {
        let selected = self.preferences.selected_instance.as_deref()?;
        self.runtime_instances
            .iter()
            .find(|instance| instance.id == selected)
    }

    fn is_server(&self) -> bool {
        self.selected_instance()
            .is_some_and(|instance| instance.kind == "server")
    }

    fn load_selected(&mut self) {
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
        if instance.root.join("orbit.toml").is_file() {
            self.orbit_task_at(
                "Loading installed mods",
                Intent::Packages,
                ["list"],
                Some(instance.root.clone()),
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

    fn launcher_task<const N: usize>(
        &mut self,
        label: &str,
        intent: Intent,
        instance: Option<String>,
        command: [&str; N],
        initial_stdin: Option<Zeroizing<String>>,
    ) -> TaskId {
        let mut args = vec![
            "--language".to_string(),
            self.preferences.language.argument().to_string(),
            "--format".to_string(),
            "json".to_string(),
            "--progress-format".to_string(),
            "ndjson".to_string(),
            "--non-interactive".to_string(),
        ];
        if let Some(instance) = instance {
            args.push("--instance".to_string());
            args.push(instance);
        }
        args.extend(command.into_iter().map(str::to_string));
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

    fn launcher_task_args(
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
            "--format".to_string(),
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

    fn orbit_task<const N: usize>(
        &mut self,
        label: &str,
        intent: Intent,
        command: [&str; N],
    ) -> TaskId {
        self.orbit_task_at(label, intent, command, None, None)
    }

    fn orbit_task_at<const N: usize>(
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

    fn orbit_task_args(
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
            "--format".to_string(),
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
        let command = request.command_name();
        let id = self.bridge.spawn(request);
        self.tasks.insert(id, TaskView::running(id, label, command));
        self.intents.insert(id, intent);
        id
    }

    fn process_events(&mut self, ctx: &egui::Context) {
        let mut reload_selected = false;
        let mut refresh_accounts = false;
        let mut refresh_yggdrasil_providers = false;
        let mut refresh_java_runtimes = false;
        let mut refresh_server = false;
        let mut refresh_registries = false;
        let mut install_runtime_after_configure = false;
        for event in self.bridge.drain() {
            match event {
                BridgeEvent::Started {
                    task_id,
                    process_id,
                } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.log.push(tr!("Process %{id} started", id = process_id));
                    }
                }
                BridgeEvent::Progress { task_id, envelope } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        let (completed, total) = wire::progress_numbers(&envelope.data);
                        task.phase = Some(envelope.phase);
                        task.completed = completed;
                        task.total = total;
                        task.status_line = wire::progress_label(&envelope.data);
                        if let Some(line) = envelope
                            .data
                            .get("line")
                            .and_then(Value::as_str)
                            .filter(|line| !line.is_empty())
                        {
                            push_bounded(&mut task.log, line.to_string());
                        }
                    }
                    ctx.request_repaint();
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
                    ctx.request_repaint();
                }
                BridgeEvent::ProtocolError { task_id, message } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.error_code = Some("protocol".to_string());
                        task.error_message = Some(message.clone());
                        task.status_line = message;
                    }
                }
                BridgeEvent::Log { task_id, line } => {
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        push_bounded(&mut task.log, line);
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
                    let succeeded = status == Some(0) && !cancelled;
                    let result = if succeeded {
                        wire::success_document(&stdout)
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
                    match result {
                        Ok(envelope) => {
                            if let Err(error) = self.apply_result(&intent, envelope.result) {
                                if matches!(intent, Intent::Search) {
                                    self.search_state = SearchState::Failed(error.to_string());
                                }
                                if let Some(task) = self.tasks.get_mut(&task_id) {
                                    task.state = TaskState::Failed;
                                    task.status_line = error.to_string();
                                    task.error_message = Some(error.to_string());
                                }
                            } else {
                                if let Some(task) = self.tasks.get_mut(&task_id) {
                                    task.state = TaskState::Succeeded;
                                    task.status_line = tr!("Completed").into_owned();
                                }
                                match intent {
                                    Intent::LauncherInstances => reload_selected = true,
                                    Intent::Mutated {
                                        refresh_packages: true,
                                    } => reload_selected = true,
                                    Intent::RuntimeMutated => {
                                        refresh_registries = true;
                                    }
                                    Intent::RuntimeConfiguredForInstall => {
                                        install_runtime_after_configure = true;
                                    }
                                    Intent::AccountMutated => {
                                        refresh_accounts = true;
                                        reload_selected = true;
                                    }
                                    Intent::YggdrasilProviderMutated => {
                                        refresh_yggdrasil_providers = true;
                                    }
                                    Intent::JavaRuntimeMutated => refresh_java_runtimes = true,
                                    Intent::ServerMutated => refresh_server = true,
                                    _ => {}
                                }
                            }
                        }
                        Err(error) => {
                            if matches!(intent, Intent::Search) {
                                let message = self
                                    .tasks
                                    .get(&task_id)
                                    .and_then(|task| task.error_message.clone())
                                    .unwrap_or_else(|| error.to_string());
                                self.search_state = SearchState::Failed(message);
                            }
                            if let Some(task) = self.tasks.get_mut(&task_id) {
                                task.state = if cancelled || task.state == TaskState::Cancelled {
                                    TaskState::Cancelled
                                } else {
                                    TaskState::Failed
                                };
                                if task.error_message.is_none() {
                                    task.status_line = error.to_string();
                                    task.error_message = Some(error.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        if reload_selected {
            self.load_selected();
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
        if install_runtime_after_configure {
            self.install_runtime();
        }
        if !self.first_refresh_done && !self.runtime_instances.is_empty() {
            self.first_refresh_done = true;
        }
    }

    fn apply_result(&mut self, intent: &Intent, result: Value) -> anyhow::Result<()> {
        match intent {
            Intent::LauncherInstances => {
                let response: RuntimeInstanceList = decode(result)?;
                self.runtime_instances = response.instances;
                let selected_exists =
                    self.preferences
                        .selected_instance
                        .as_ref()
                        .is_some_and(|id| {
                            self.runtime_instances
                                .iter()
                                .any(|instance| &instance.id == id)
                        });
                if !selected_exists {
                    self.preferences.selected_instance = self
                        .runtime_instances
                        .iter()
                        .find(|instance| instance.is_default)
                        .or_else(|| self.runtime_instances.first())
                        .map(|instance| instance.id.clone());
                }
            }
            Intent::LauncherInstanceDetail => {
                let detail: RuntimeInstanceDetail = decode(result)?;
                self.runtime_edit.name = detail.instance.name.clone();
                let target = detail.installed.as_ref();
                self.runtime_edit.minecraft = target
                    .map(|installed| installed.minecraft.clone())
                    .unwrap_or_else(|| detail.desired.minecraft.clone());
                self.runtime_edit.loader = loader_index(
                    target
                        .map(|installed| installed.loader.as_str())
                        .unwrap_or(&detail.desired.loader),
                );
                self.runtime_edit.loader_version = target
                    .and_then(|installed| installed.loader_version.clone())
                    .or_else(|| detail.desired.loader_version.clone())
                    .unwrap_or_default();
                self.runtime_edit.java_policy = java_policy_index(&detail.desired.java_policy);
                self.instance_detail = Some(detail);
                self.load_runtime_metadata();
            }
            Intent::OrbitInstances => {
                self.orbit_instances = decode::<OrbitInstanceList>(result)?.instances
            }
            Intent::Packages => self.packages = decode::<PackageList>(result)?.packages,
            Intent::Search => {
                let response: SearchResults = decode(result)?;
                self.search_results = response.results;
                self.search_truncated = response.truncated;
                self.search_state = SearchState::Completed;
            }
            Intent::Outdated => {
                let response: OutdatedResults = decode(result)?;
                self.outdated = response.updates;
                self.outdated_diagnostics = response.diagnostics;
                self.outdated_warnings = response.warnings;
                self.outdated_checked = true;
                self.mod_view = 1;
            }
            Intent::Audit => self.audit = Some(wire::audit_summary(&result)),
            Intent::Accounts => self.accounts = decode::<AccountList>(result)?.accounts,
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
                if self.new_instance.minecraft.trim().is_empty() {
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
            Intent::MicrosoftBegin => self.microsoft_session = Some(result),
            Intent::EulaShow => self.eula_document = Some(result),
            Intent::Mutated { .. }
            | Intent::RuntimeMutated
            | Intent::RuntimeConfiguredForInstall
            | Intent::AccountMutated
            | Intent::YggdrasilProviderMutated
            | Intent::JavaRuntimeMutated
            | Intent::ServerMutated
            | Intent::Generic => {}
        }
        Ok(())
    }

    fn load_runtime_metadata(&mut self) {
        let minecraft = self.runtime_edit.minecraft.trim().to_string();
        let loader = self.runtime_edit.loader;
        self.request_runtime_metadata(&minecraft, loader);
    }

    fn request_runtime_metadata(&mut self, minecraft: &str, loader_index: usize) {
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
        let loaders = ["vanilla", "fabric", "forge", "neoforge", "quilt"];
        let loader = loaders[loader_index];
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

    fn show_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("navigation")
            .exact_width(188.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::sidebar())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .inner_margin(14),
            )
            .show(ctx, |ui| {
                theme::apply_ui(ui);
                ui.horizontal(|ui| {
                    theme::orbit_mark(ui, 34.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("ORBIT").size(18.0).strong());
                        ui.label(
                            RichText::new(tr!("Minecraft workspace"))
                                .size(11.0)
                                .color(theme::muted()),
                        );
                    });
                });
                ui.add_space(18.0);
                for page in Page::ALL {
                    if page == Page::Server && !self.is_server() {
                        continue;
                    }
                    if page == Page::Runtime {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(tr!("SYSTEM"))
                                .size(10.0)
                                .color(theme::muted()),
                        );
                        ui.add_space(3.0);
                    }
                    let selected = self.preferences.page == page;
                    let text = RichText::new(page.label())
                        .color(if selected {
                            theme::accent()
                        } else {
                            theme::muted()
                        })
                        .strong();
                    let button = egui::Button::new(text)
                        .fill(if selected {
                            theme::accent_soft()
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(if selected {
                            Stroke::new(1.0, theme::accent())
                        } else {
                            Stroke::NONE
                        })
                        .corner_radius(8)
                        .min_size(Vec2::new(ui.available_width(), 39.0));
                    if ui.add(button).clicked() {
                        self.preferences.page = page;
                    }
                }
            });
    }

    fn show_topbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar")
            .exact_height(58.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(egui::Margin::symmetric(20, 11)),
            )
            .show(ctx, |ui| {
                theme::apply_ui(ui);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.preferences.page.label())
                            .size(16.0)
                            .strong(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.tasks.is_empty()
                            && ui.add(theme::ghost_button("Activity")).clicked()
                        {
                            self.activity_open = !self.activity_open;
                        }
                        if ui.add(theme::ghost_button("Refresh")).clicked() {
                            self.refresh_registries();
                        }
                        let previous = self.preferences.selected_instance.clone();
                        ComboBox::from_id_salt("instance-switcher")
                            .width(240.0)
                            .selected_text(
                                self.selected_instance()
                                    .map(|instance| {
                                        format!("{} · {}", instance.name, instance.kind)
                                    })
                                    .unwrap_or_else(|| tr!("Select an instance").into_owned()),
                            )
                            .show_ui(ui, |ui| {
                                for instance in &self.runtime_instances {
                                    ui.selectable_value(
                                        &mut self.preferences.selected_instance,
                                        Some(instance.id.clone()),
                                        format!("{} · {}", instance.name, instance.kind),
                                    );
                                }
                            });
                        if previous != self.preferences.selected_instance {
                            self.load_selected();
                        }
                    });
                });
            });
    }

    fn execute_confirmation(&mut self, action: ConfirmationAction) {
        match action {
            ConfirmationAction::LogoutAccount(account) => {
                self.launcher_task_args(
                    "Logging out account",
                    Intent::AccountMutated,
                    None,
                    vec!["account".into(), "logout".into(), account],
                    None,
                );
            }
            ConfirmationAction::RemoveYggdrasilProvider(provider) => {
                self.launcher_task_args(
                    "Removing Yggdrasil provider",
                    Intent::YggdrasilProviderMutated,
                    None,
                    vec![
                        "config".into(),
                        "yggdrasil".into(),
                        "remove".into(),
                        provider,
                    ],
                    None,
                );
            }
            ConfirmationAction::UnregisterInstance(instance) => {
                self.launcher_task(
                    "Unregistering instance",
                    Intent::RuntimeMutated,
                    Some(instance),
                    ["instance", "remove"],
                    None,
                );
            }
            ConfirmationAction::RemoveJavaRuntime(runtime_id) => {
                self.launcher_task_args(
                    "Removing managed Java runtime",
                    Intent::JavaRuntimeMutated,
                    None,
                    vec!["java".into(), "remove".into(), runtime_id],
                    None,
                );
            }
            ConfirmationAction::AcceptEula(digest) => {
                if let Some(instance) = self.selected_instance().cloned() {
                    self.launcher_task_args(
                        "Accepting Minecraft EULA",
                        Intent::ServerMutated,
                        Some(instance.id),
                        vec!["server".into(), "eula".into(), "accept".into(), digest],
                        None,
                    );
                }
            }
        }
    }

    fn selected_root(&self) -> Option<PathBuf> {
        self.selected_instance()
            .map(|instance| instance.root.clone())
    }

    fn reload_packages(&mut self) {
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

    fn run_outdated(&mut self) {
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

    fn run_audit(&mut self) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_at(
                "Auditing bytecode compatibility",
                Intent::Audit,
                ["audit"],
                Some(root),
                None,
            );
        }
    }

    fn install_mods(&mut self) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                "Installing mod environment",
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec!["install".into()],
                Some(root),
                None,
            );
        }
    }

    fn initialize_orbit(&mut self) {
        let Some(detail) = self.instance_detail.clone() else {
            self.toast = Some((
                tr!("Select a Launcher instance first.").into_owned(),
                theme::warning(),
            ));
            return;
        };
        let Some(installed) = detail.installed else {
            self.toast = Some((
                tr!("Install the Minecraft runtime before initializing Orbit.").into_owned(),
                theme::warning(),
            ));
            return;
        };
        if installed.loader == "vanilla" {
            self.toast = Some((
                tr!("Orbit mod management requires Fabric, Quilt, Forge, or NeoForge.")
                    .into_owned(),
                theme::warning(),
            ));
            return;
        }
        let Some(loader_version) = installed.loader_version else {
            self.toast = Some((
                tr!("The installed runtime lock has no exact Loader version.").into_owned(),
                theme::danger(),
            ));
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
            Some(detail.instance.root),
            None,
        );
    }

    fn sync_instance(&mut self) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                "Synchronizing instance",
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec!["sync".into()],
                Some(root),
                None,
            );
        }
    }

    fn upgrade_package(&mut self, package: &str) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                &tr!("Upgrading %{package}", package = package),
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec!["upgrade".into(), package.into()],
                Some(root),
                None,
            );
        }
    }

    fn remove_package(&mut self, package: &str) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                &tr!("Removing %{package}", package = package),
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec!["remove".into(), package.into()],
                Some(root),
                None,
            );
        }
    }

    fn upgrade_all_packages(&mut self) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                "Upgrading mod environment",
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec!["upgrade".into()],
                Some(root),
                None,
            );
        }
    }

    fn set_package_environment(&mut self, package: &str, environment: &str) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                &tr!("Updating %{package} environment", package = package),
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec!["env".into(), package.into(), environment.into()],
                Some(root),
                None,
            );
        }
    }

    fn add_package_remote(&mut self, package: &str, provider: &str, locator: &str) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                &tr!(
                    "Adding %{provider} remote to %{package}",
                    provider = provider,
                    package = package
                ),
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec![
                    "remote".into(),
                    "add".into(),
                    package.into(),
                    provider.into(),
                    locator.into(),
                ],
                Some(root),
                None,
            );
        }
    }

    fn remove_package_remote(&mut self, package: &str, index: usize) {
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                &tr!("Removing remote from %{package}", package = package),
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec![
                    "remote".into(),
                    "remove".into(),
                    package.into(),
                    "--index".into(),
                    index.to_string(),
                ],
                Some(root),
                None,
            );
        }
    }

    fn search_catalog(&mut self) {
        self.search_results.clear();
        self.search_truncated = false;
        self.search_state = SearchState::Running;
        let mut command = vec!["search".into(), self.search_query.trim().into()];
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

    fn add_search_result(&mut self, result: &SearchResult) {
        let locator = match result.platform.as_str() {
            "modrinth" => format!("mr:{}", result.project_id),
            "curseforge" => format!("cf:{}", result.project_id),
            _ => result.project_id.clone(),
        };
        if let Some(root) = self.selected_root() {
            self.orbit_task_args(
                &tr!("Adding %{name}", name = result.name),
                Intent::Mutated {
                    refresh_packages: true,
                },
                vec!["add".into(), locator],
                Some(root),
                None,
            );
        }
    }

    fn install_runtime(&mut self) {
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

    fn configure_runtime_and_install(&mut self) {
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let loaders = ["vanilla", "fabric", "forge", "neoforge", "quilt"];
        let java_policies = ["auto", "managed"];
        let mut command = vec![
            "instance".into(),
            "configure".into(),
            "--minecraft".into(),
            self.runtime_edit.minecraft.trim().to_string(),
            "--loader".into(),
            loaders[self.runtime_edit.loader].into(),
            "--java-policy".into(),
            java_policies[self.runtime_edit.java_policy].into(),
        ];
        if self.runtime_edit.loader != 0 {
            command.extend([
                "--loader-version".into(),
                self.runtime_edit.loader_version.trim().to_string(),
            ]);
        }
        self.launcher_task_args(
            "Preparing runtime update",
            Intent::RuntimeConfiguredForInstall,
            Some(instance.id),
            command,
            None,
        );
    }

    fn set_default_runtime(&mut self) {
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

    fn import_runtime(&mut self) {
        self.launcher_task_args(
            "Importing runtime instance",
            Intent::RuntimeMutated,
            None,
            vec![
                "instance".into(),
                "import".into(),
                "--root".into(),
                self.runtime_edit.import_root.trim().to_string(),
            ],
            None,
        );
    }

    fn refresh_java_runtimes(&mut self, verify: bool) {
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

    fn verify_java_runtime(&mut self, runtime_id: &str) {
        self.launcher_task_args(
            "Verifying managed Java runtime",
            Intent::JavaRuntimeMutated,
            None,
            vec!["java".into(), "verify".into(), runtime_id.into()],
            None,
        );
    }

    fn create_runtime(&mut self) {
        let loaders = ["vanilla", "fabric", "forge", "neoforge", "quilt"];
        let mut command = vec![
            "install".into(),
            "--new".into(),
            self.new_instance.name.trim().into(),
            "--root".into(),
            self.new_instance.root.trim().into(),
            "--kind".into(),
            if self.new_instance.kind == 0 {
                "client"
            } else {
                "server"
            }
            .into(),
            "--minecraft".into(),
            self.new_instance.minecraft.trim().into(),
            "--loader".into(),
            loaders[self.new_instance.loader].into(),
        ];
        if self.new_instance.loader != 0 {
            command.extend([
                "--loader-version".into(),
                self.new_instance.loader_version.trim().into(),
            ]);
        }
        self.launcher_task_args(
            "Creating runtime",
            Intent::RuntimeMutated,
            None,
            command,
            None,
        );
    }
}

impl eframe::App for OrbitApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::install(
            ctx,
            self.preferences.theme_mode,
            self.preferences.accent_theme,
        );
        self.process_events(ctx);
        self.show_sidebar(ctx);
        self.show_topbar(ctx);
        self.show_activity(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(egui::Margin::symmetric(20, 16)),
            )
            .show(ctx, |ui| {
                theme::apply_ui(ui);
                match self.preferences.page {
                    Page::Library => self.show_library(ui),
                    Page::Discover => self.show_discover(ui),
                    Page::Audit => self.show_audit(ui),
                    page => {
                        let flow_step = self.runtime_flow.map(|flow| flow.step);
                        ScrollArea::vertical()
                            .id_salt((page, flow_step))
                            .auto_shrink([false, false])
                            .show(ui, |ui| match page {
                                Page::Home => self.show_home(ui),
                                Page::Runtime => self.show_runtime(ui),
                                Page::Accounts => self.show_accounts(ui),
                                Page::Server => self.show_server(ui),
                                Page::Settings => self.show_settings(ui),
                                Page::Library | Page::Discover | Page::Audit => unreachable!(),
                            });
                    }
                }
            });
        self.show_overlays(ctx);
        if self
            .tasks
            .values()
            .any(|task| task.state == TaskState::Running)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self.preferences);
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

fn decode<T: DeserializeOwned>(value: Value) -> anyhow::Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

fn push_bounded(log: &mut Vec<String>, line: String) {
    const LIMIT: usize = 500;
    if log.len() == LIMIT {
        log.remove(0);
    }
    log.push(line);
}

fn title_case(value: &str) -> String {
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
            let mut characters = other.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
                .unwrap_or_default()
        }
    }
}

fn info_chip(ui: &mut egui::Ui, label: &str, color: Color32) {
    let label = orbit_i18n::text(label);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.7)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(10.0).strong().color(color));
        });
}

fn version_badge(ui: &mut egui::Ui, label: &str, size: f32) {
    let font_size = if label.chars().count() > 5 {
        11.0
    } else if label.chars().count() > 2 {
        14.0
    } else {
        18.0
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().rect(
        rect,
        12.0,
        theme::accent_soft(),
        Stroke::new(1.0, theme::accent()),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(font_size),
        theme::text(),
    );
}

fn runtime_steps(ui: &mut egui::Ui, active: RuntimeFlowStep) {
    let active_index = match active {
        RuntimeFlowStep::Minecraft => 0,
        RuntimeFlowStep::Components => 1,
        RuntimeFlowStep::Review => 2,
    };
    ui.columns(3, |columns| {
        for (index, label) in [
            tr!("1  Minecraft"),
            tr!("2  Loader & Java"),
            tr!("3  Review"),
        ]
        .iter()
        .enumerate()
        {
            let color = if index <= active_index {
                theme::accent()
            } else {
                theme::border()
            };
            egui::Frame::new()
                .fill(if index == active_index {
                    theme::accent_soft()
                } else {
                    theme::surface()
                })
                .stroke(Stroke::new(1.0, color))
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(&mut columns[index], |ui| {
                    ui.label(RichText::new(label.as_ref()).strong().color(
                        if index <= active_index {
                            theme::text()
                        } else {
                            theme::muted()
                        },
                    ));
                });
        }
    });
}

fn selectable_runtime_row(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    selected: bool,
    tag: Option<&str>,
) -> egui::Response {
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
                ui.vertical(|ui| {
                    ui.label(RichText::new(orbit_i18n::text(title)).size(16.0).strong());
                    ui.label(
                        RichText::new(orbit_i18n::text(subtitle))
                            .size(12.0)
                            .color(theme::muted()),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(tag) = tag {
                        info_chip(ui, tag, theme::success());
                    }
                    if selected {
                        info_chip(ui, "SELECTED", theme::accent_hover());
                    }
                });
            });
        });
    response.response.interact(Sense::click())
}

fn summary_value(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(orbit_i18n::text(label))
                .size(10.0)
                .color(theme::muted()),
        );
        ui.label(RichText::new(value).size(17.0).strong());
    });
}

fn change_row(ui: &mut egui::Ui, label: &str, before: &str, after: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(orbit_i18n::text(label)).color(theme::muted()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(after).strong().color(theme::success()));
            ui.label(RichText::new(tr!("to")).size(11.0).color(theme::muted()));
            ui.label(RichText::new(before).color(theme::muted()).strikethrough());
        });
    });
}

fn onboarding_step(ui: &mut egui::Ui, number: &str, title: &str, detail: &str) {
    theme::card().show(ui, |ui| {
        version_badge(ui, number, 36.0);
        ui.add_space(5.0);
        ui.label(RichText::new(orbit_i18n::text(title)).strong());
        ui.label(
            RichText::new(orbit_i18n::text(detail))
                .size(11.0)
                .color(theme::muted()),
        );
    });
}

fn package_initials(mod_id: &str) -> String {
    let mut initials = mod_id
        .split(['-', '_', '.'])
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>();
    if initials.len() < 2 {
        initials = mod_id.chars().take(2).collect();
    }
    initials.to_ascii_uppercase()
}

fn account_provider_label(provider: &str) -> std::borrow::Cow<'static, str> {
    match provider {
        "microsoft" => tr!("Microsoft"),
        "offline" => tr!("Offline"),
        "yggdrasil" => tr!("Yggdrasil"),
        _ => tr!("External"),
    }
}

fn account_avatar(ui: &mut egui::Ui, profile_name: &str, size: f32) {
    let initial = profile_name
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_string());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), size * 0.5, theme::accent_soft());
    ui.painter()
        .circle_stroke(rect.center(), size * 0.5, Stroke::new(1.0, theme::accent()));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(size * 0.38),
        theme::accent(),
    );
}

fn login_method_card(
    ui: &mut egui::Ui,
    badge: &str,
    title: &str,
    detail: &str,
    enabled: bool,
) -> egui::Response {
    let response = theme::card().show(ui, |ui| {
        ui.set_min_height(112.0);
        version_badge(ui, badge, 40.0);
        ui.add_space(5.0);
        ui.label(
            RichText::new(orbit_i18n::text(title))
                .size(17.0)
                .strong()
                .color(if enabled {
                    theme::text()
                } else {
                    theme::muted()
                }),
        );
        ui.label(
            RichText::new(orbit_i18n::text(detail))
                .size(12.0)
                .color(theme::muted()),
        );
    });
    response.response.interact(if enabled {
        Sense::click()
    } else {
        Sense::hover()
    })
}

fn metric_card(ui: &mut egui::Ui, label: &str, value: String, hint: &str) {
    theme::card().show(ui, |ui| {
        ui.label(
            RichText::new(orbit_i18n::text(label))
                .size(12.0)
                .color(theme::muted()),
        );
        ui.label(RichText::new(value).size(28.0).strong());
        ui.label(
            RichText::new(orbit_i18n::text(hint))
                .size(11.0)
                .color(theme::muted()),
        );
    });
}

fn capability_card(ui: &mut egui::Ui, index: &str, title: &str, detail: &str) {
    theme::card().show(ui, |ui| {
        version_badge(ui, index, 38.0);
        ui.add_space(9.0);
        ui.label(RichText::new(orbit_i18n::text(title)).size(16.0).strong());
        ui.label(
            RichText::new(orbit_i18n::text(detail))
                .size(12.0)
                .color(theme::muted()),
        );
        ui.add_space(4.0);
    });
}

fn quick_action(ui: &mut egui::Ui, glyph: &str, title: &str, detail: &str) -> bool {
    let response = theme::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            version_badge(ui, glyph, 38.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(orbit_i18n::text(title)).strong());
                ui.label(
                    RichText::new(orbit_i18n::text(detail))
                        .size(11.0)
                        .color(theme::muted()),
                );
            });
        });
    });
    response.response.interact(Sense::click()).clicked()
}

fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        theme::orbit_mark(ui, 46.0);
        ui.add_space(8.0);
        ui.label(RichText::new(orbit_i18n::text(title)).size(19.0).strong());
        ui.label(RichText::new(orbit_i18n::text(detail)).color(theme::muted()));
    });
}

fn installation_required_card(ui: &mut egui::Ui, title: &str, detail: &str) -> bool {
    let mut open_installations = false;
    theme::elevated_card().show(ui, |ui| {
        ui.horizontal(|ui| {
            theme::orbit_mark(ui, 44.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(orbit_i18n::text(title)).size(19.0).strong());
                ui.label(RichText::new(orbit_i18n::text(detail)).color(theme::muted()));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                open_installations = ui
                    .add(theme::secondary_button("Open installations"))
                    .clicked();
            });
        });
    });
    open_installations
}

fn status_dot(ui: &mut egui::Ui, state: TaskState) {
    let color = match state {
        TaskState::Running => theme::accent(),
        TaskState::Succeeded => theme::success(),
        TaskState::Failed => theme::danger(),
        TaskState::Cancelled => theme::muted(),
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn risk_color(risk: u8) -> Color32 {
    match risk {
        75..=u8::MAX => theme::danger(),
        45..=74 => theme::warning(),
        _ => theme::success(),
    }
}

fn compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn loader_index(loader: &str) -> usize {
    match loader {
        "fabric" => 1,
        "forge" => 2,
        "neoforge" => 3,
        "quilt" => 4,
        _ => 0,
    }
}

fn java_policy_index(policy: &str) -> usize {
    match policy {
        "managed" => 1,
        _ => 0,
    }
}

fn minecraft_type_matches(version: &MinecraftVersion, filter: usize) -> bool {
    match filter {
        0 => version.version_type == "release",
        1 => version.version_type == "snapshot",
        2 => matches!(version.version_type.as_str(), "old_alpha" | "old_beta"),
        _ => true,
    }
}

fn loader_version_tags(version: &LoaderVersion) -> String {
    let mut tags = Vec::new();
    if version.recommended {
        tags.push(tr!("recommended").into_owned());
    } else if version.stable {
        tags.push(tr!("stable").into_owned());
    }
    if version.latest {
        tags.push(tr!("latest").into_owned());
    }
    if let Some(major) = version.minimum_java_major {
        tags.push(format!("Java {major}+"));
    }
    tags.join(" · ")
}

fn java_requirement_label(requirement: Option<&JavaRequirement>) -> String {
    match requirement {
        Some(requirement) if requirement.required => {
            let component = requirement
                .component
                .clone()
                .unwrap_or_else(|| tr!("official component").into_owned());
            tr!(
                "Java %{major} · %{component} · downloaded automatically",
                major = requirement.major.unwrap_or_default(),
                component = component
            )
        }
        Some(_) => tr!("No authoritative managed Java component published").into_owned(),
        None => tr!("Loading official Minecraft metadata…").into_owned(),
    }
}

fn render_interaction_data(ui: &mut egui::Ui, data: &Value) {
    let Some(changes) = data.get("changes").and_then(Value::as_array) else {
        return;
    };
    if changes.is_empty() {
        return;
    }
    ui.add_space(8.0);
    for item in changes {
        let different = item
            .get("different")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let change = item.get("change").unwrap_or(item);
        let package = change
            .get("package")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let kind = change
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("change");
        let current = change
            .get("current_version")
            .and_then(Value::as_str)
            .unwrap_or("not installed");
        let selected = change
            .get("selected_version")
            .and_then(Value::as_str)
            .unwrap_or("removed");
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if different { "◆" } else { "•" }).color(if different {
                    theme::accent()
                } else {
                    theme::muted()
                }),
            );
            ui.label(RichText::new(package).strong());
            ui.label(
                RichText::new(format!("{kind}: {current} → {selected}"))
                    .size(12.0)
                    .color(if different {
                        theme::text()
                    } else {
                        theme::muted()
                    }),
            );
        });
    }
}
