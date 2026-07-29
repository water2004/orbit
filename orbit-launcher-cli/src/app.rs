use std::fs::OpenOptions;
use std::io::BufRead;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use orbit_launcher_core::{
    AccountMetadata, AccountRepository, ConfigKey, ContextIntent, CreateInstanceRequest,
    EulaAcceptanceMethod, EulaDocument, ExternalYggdrasilLoginRequest, InstallProgressEvent,
    InstanceKind, InstanceRegistry, LaunchPreparationEvent, LaunchProcessEvent, LauncherError,
    ManifestFile, MicrosoftLoginProgressEvent, RepositoryMoveEvent, ResolvedInstance,
    RuntimeContext, ServerInstallPlan, SupervisorControl, SupervisorEvent, YggdrasilProviderConfig,
    accept_shown_eula, add_yggdrasil_provider, apply_install_plan, begin_microsoft_device_login,
    complete_microsoft_device_login, configure_instance, create_instance, create_offline_account,
    get_config, import_instance, list_config, login_external_yggdrasil, move_minecraft_directory,
    native_secret_store, prepare_install, prepare_launch, remove_instance,
    remove_yggdrasil_provider, rename_instance, resolve_directory, resolve_instance,
    resolve_launch_identity, rollback_created_instance, run_launch, set_config,
    set_default_instance, show_current_eula, supervise_server, unset_config,
};
use tokio::sync::{mpsc, watch};
use zeroize::Zeroizing;

use crate::cli::{
    AccountCommands, AccountLoginCommands, Commands, ConfigCommands, DefaultCommands, EulaCommands,
    InstanceCommands, JavaCommands, MicrosoftLoginCommands, MinecraftCommands, ServerCommands,
    VersionCommands, YggdrasilProviderCommands,
};
use crate::output::{
    AccountListView, AccountLoginView, AccountLogoutView, AccountSelectionView, AccountView,
    ConfigEntryView, ConfigListView, ConfigMutationAction, ConfigMutationView, ConfigPathView,
    DefaultView, EulaAcceptanceView, EulaDocumentView, InstallView, InstanceDetailView,
    InstanceListView, InstanceMutationAction, InstanceMutationView, InstanceView,
    JavaRequirementView, JavaRuntimeListView, JavaRuntimeMutationView, LaunchPlanView,
    LaunchResultView, LoaderVersionCatalogView, MicrosoftDeviceSessionView,
    MinecraftDirectoryMoveView, MinecraftDirectoryView, MinecraftVersionCatalogView, RenameView,
    ServerControlView, ServerStartView, ServerStatusView, SupervisorResultView,
    YggdrasilProviderListView, YggdrasilProviderMutationView, YggdrasilProviderView,
};
use crate::supervisor_ipc::{
    IpcRequest, IpcServer, SupervisorLock, SupervisorState, request as supervisor_request,
};

#[derive(Debug)]
pub enum CommandOutput {
    ConfigPath(ConfigPathView),
    ConfigList(ConfigListView),
    ConfigEntry(ConfigEntryView),
    ConfigMutation(ConfigMutationView),
    EulaDocument(EulaDocumentView),
    EulaAcceptance(EulaAcceptanceView),
    Install(InstallView),
    LaunchPlan(LaunchPlanView),
    LaunchResult(LaunchResultView),
    ServerStart(ServerStartView),
    ServerStatus(ServerStatusView),
    ServerControl(ServerControlView),
    SupervisorResult(SupervisorResultView),
    InstanceList(InstanceListView),
    InstanceDetail(InstanceDetailView),
    InstanceMutation(InstanceMutationView),
    Rename(RenameView),
    InstanceConfigured(InstanceDetailView),
    Default(DefaultView),
    AccountList(AccountListView),
    AccountDetail(AccountView),
    AccountRefresh(AccountView),
    AccountLogin(AccountLoginView),
    AccountSelection(AccountSelectionView),
    AccountLogout(AccountLogoutView),
    MicrosoftDeviceSession(MicrosoftDeviceSessionView),
    YggdrasilProviderList(YggdrasilProviderListView),
    YggdrasilProviderMutation(YggdrasilProviderMutationView),
    JavaRuntimeList(JavaRuntimeListView),
    JavaRuntimeMutation(JavaRuntimeMutationView),
    MinecraftVersions(MinecraftVersionCatalogView),
    LoaderVersions(LoaderVersionCatalogView),
    JavaRequirement(JavaRequirementView),
    MinecraftDirectory(MinecraftDirectoryView),
    MinecraftDirectoryMove(MinecraftDirectoryMoveView),
}

impl CommandOutput {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::ConfigPath(_) => "config.path",
            Self::ConfigList(_) => "config.list",
            Self::ConfigEntry(_) => "config.get",
            Self::ConfigMutation(view) => view.action.command_name(),
            Self::EulaDocument(_) => "server.eula.show",
            Self::EulaAcceptance(_) => "server.eula.accept",
            Self::Install(_) => "install",
            Self::LaunchPlan(view) => {
                if view.kind == "client" {
                    "launch"
                } else {
                    "server.run"
                }
            }
            Self::LaunchResult(view) => {
                if view.kind == "client" {
                    "launch"
                } else {
                    "server.run"
                }
            }
            Self::ServerStart(_) => "server.start",
            Self::ServerStatus(_) => "server.status",
            Self::ServerControl(view) => match view.action {
                "stop" => "server.stop",
                _ => "server.command",
            },
            Self::SupervisorResult(_) => "server.supervisor",
            Self::InstanceList(_) => "instance.list",
            Self::InstanceDetail(_) => "instance.show",
            Self::InstanceMutation(view) => view.action.command_name(),
            Self::Rename(_) => "instance.rename",
            Self::InstanceConfigured(_) => "instance.configure",
            Self::Default(_) => "instance.default",
            Self::AccountList(_) => "account.list",
            Self::AccountDetail(_) => "account.show",
            Self::AccountRefresh(_) => "account.refresh",
            Self::AccountLogin(view) => match view.method {
                "offline" => "account.login.offline",
                "microsoft" => "account.login.microsoft.complete",
                _ => "account.login.yggdrasil",
            },
            Self::AccountSelection(_) => "account.select",
            Self::AccountLogout(_) => "account.logout",
            Self::MicrosoftDeviceSession(_) => "account.login.microsoft.begin",
            Self::YggdrasilProviderList(_) => "config.yggdrasil.list",
            Self::YggdrasilProviderMutation(view) => match view.action {
                "added" => "config.yggdrasil.add",
                _ => "config.yggdrasil.remove",
            },
            Self::JavaRuntimeList(_) => "java.list",
            Self::JavaRuntimeMutation(view) => match view.action {
                "verified" => "java.verify",
                _ => "java.remove",
            },
            Self::MinecraftVersions(_) => "versions.minecraft",
            Self::LoaderVersions(_) => "versions.loader",
            Self::JavaRequirement(_) => "versions.java",
            Self::MinecraftDirectory(_) => "minecraft.directory",
            Self::MinecraftDirectoryMove(_) => "minecraft.move",
        }
    }

    pub const fn process_succeeded(&self) -> bool {
        match self {
            Self::LaunchResult(view) => view.success,
            Self::SupervisorResult(view) => {
                !view.restart_limit_reached && (view.stopped_by_request || view.final_success)
            }
            _ => true,
        }
    }
}

pub trait Frontend: Send {
    fn progress(&mut self, event: InstallProgressEvent);

    fn confirm_eula(&mut self, document: &EulaDocument) -> Result<bool, LauncherError>;

    fn read_password(
        &mut self,
        prompt: &str,
        stdin: bool,
    ) -> Result<Zeroizing<String>, LauncherError>;

    fn microsoft_login_progress(&mut self, event: MicrosoftLoginProgressEvent);

    fn launch_preparation(&mut self, command: &'static str, event: LaunchPreparationEvent);

    fn launch_process(&mut self, command: &'static str, event: LaunchProcessEvent);

    fn supervisor_event(&mut self, command: &'static str, event: SupervisorEvent);

    fn repository_move(&mut self, event: RepositoryMoveEvent);
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Core(#[from] orbit_launcher_core::LauncherError),
    #[error("invalid command usage: {0}")]
    Argument(String),
}

impl AppError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Core(error) => error.code(),
            Self::Argument(_) => "argument",
        }
    }
}

pub async fn execute(
    command: Commands,
    instance_selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    match command {
        Commands::Install {
            new,
            server_directory,
            kind,
            minecraft,
            loader,
            loader_version,
        } => {
            execute_install(
                instance_selector,
                current_dir,
                runtime,
                frontend,
                InstallCommandRequest {
                    new_name: new,
                    server_directory,
                    kind,
                    minecraft,
                    loader,
                    loader_version,
                },
            )
            .await
        }
        Commands::Launch { dry_run } => {
            execute_launch(
                instance_selector,
                current_dir,
                runtime,
                frontend,
                InstanceKind::Client,
                dry_run,
            )
            .await
        }
        Commands::Config { command } => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for configuration commands".to_string(),
                ));
            }
            execute_config(command, runtime).await
        }
        Commands::Instance { command } => {
            if instance_selector.is_some() && !command.accepts_instance_context() {
                return Err(AppError::Argument(
                    "--instance is not valid for this instance subcommand".to_string(),
                ));
            }
            execute_instance(command, instance_selector, current_dir, runtime)
        }
        Commands::Server { command } => {
            execute_server(command, instance_selector, current_dir, runtime, frontend).await
        }
        Commands::Account { command } => {
            execute_account(command, instance_selector, current_dir, runtime, frontend).await
        }
        Commands::Java { command } => execute_java(command, instance_selector, runtime),
        Commands::Minecraft { command } => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for Minecraft repository commands".to_string(),
                ));
            }
            execute_minecraft(command, current_dir, runtime, frontend)
        }
        Commands::Versions { command } => {
            execute_versions(command, instance_selector, runtime).await
        }
        Commands::Supervisor => {
            execute_internal_supervisor(instance_selector, current_dir, runtime, frontend).await
        }
    }
}

async fn execute_versions(
    command: VersionCommands,
    instance_selector: Option<&str>,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    if instance_selector.is_some() {
        return Err(AppError::Argument(
            "--instance is not valid for global version catalogs".to_string(),
        ));
    }
    let client = runtime.config().http_client()?;
    match command {
        VersionCommands::Minecraft => Ok(CommandOutput::MinecraftVersions(
            orbit_launcher_core::MojangClient::new(client)
                .list_versions()
                .await?
                .into(),
        )),
        VersionCommands::Loader { loader, minecraft } => {
            let loader: orbit_launcher_core::LoaderKind = loader.into();
            let versions =
                orbit_launcher_core::list_loader_versions(&client, loader, &minecraft).await?;
            Ok(CommandOutput::LoaderVersions(LoaderVersionCatalogView {
                loader: loader.as_str().to_string(),
                minecraft,
                versions: versions.into_iter().map(Into::into).collect(),
            }))
        }
        VersionCommands::Java { minecraft } => {
            let requirement = orbit_launcher_core::MojangClient::new(client)
                .resolve_java_requirement(&minecraft)
                .await?;
            Ok(CommandOutput::JavaRequirement(JavaRequirementView {
                minecraft,
                required: requirement.is_some(),
                component: requirement.as_ref().map(|value| value.component.clone()),
                major: requirement.map(|value| value.major),
            }))
        }
    }
}

fn execute_java(
    command: JavaCommands,
    instance_selector: Option<&str>,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    if instance_selector.is_some() {
        return Err(AppError::Argument(
            "--instance is not valid for global Java runtime management".to_string(),
        ));
    }
    match command {
        JavaCommands::List { verify } => Ok(CommandOutput::JavaRuntimeList(JavaRuntimeListView {
            verification_requested: verify,
            runtimes: orbit_launcher_core::list_managed_java_runtimes(runtime.paths(), verify)?
                .into_iter()
                .map(Into::into)
                .collect(),
        })),
        JavaCommands::Verify { runtime_id } => Ok(CommandOutput::JavaRuntimeMutation(
            JavaRuntimeMutationView {
                action: "verified",
                runtime: orbit_launcher_core::verify_managed_java_runtime(
                    runtime.paths(),
                    &runtime_id,
                )?
                .into(),
            },
        )),
        JavaCommands::Remove { runtime_id } => Ok(CommandOutput::JavaRuntimeMutation(
            JavaRuntimeMutationView {
                action: "removed",
                runtime: orbit_launcher_core::remove_managed_java_runtime(
                    runtime.paths(),
                    &runtime_id,
                )?
                .into(),
            },
        )),
    }
}

fn execute_minecraft(
    command: MinecraftCommands,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    match command {
        MinecraftCommands::Directory => {
            Ok(CommandOutput::MinecraftDirectory(MinecraftDirectoryView {
                directory: runtime.minecraft_directory(),
                explicit: runtime.config().minecraft.directory.is_some(),
            }))
        }
        MinecraftCommands::Move { destination } => {
            let destination = resolve_directory(current_dir, Some(&destination))?;
            let moved = move_minecraft_directory(
                runtime.paths(),
                runtime.config(),
                &destination,
                |event| frontend.repository_move(event),
            )?;
            Ok(CommandOutput::MinecraftDirectoryMove(moved.into()))
        }
    }
}

async fn execute_launch(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
    expected_kind: InstanceKind,
    dry_run: bool,
) -> Result<CommandOutput, AppError> {
    let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
    let resolved = resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
    if resolved.manifest.kind != expected_kind {
        return Err(AppError::Argument(format!(
            "instance '{}' is a {}; this command requires a {} instance",
            resolved.entry.name,
            resolved.manifest.kind.as_str(),
            expected_kind.as_str()
        )));
    }
    let command_name = if expected_kind == InstanceKind::Client {
        "launch"
    } else {
        "server.run"
    };
    let identity = if expected_kind == InstanceKind::Client {
        let client = runtime.config().http_client()?;
        let secrets = native_secret_store(runtime.paths())?;
        Some(
            resolve_launch_identity(
                runtime.paths(),
                runtime.config(),
                &client,
                secrets.as_ref(),
                resolved.manifest.launch.account,
            )
            .await?,
        )
    } else {
        None
    };
    let plan = prepare_launch(
        &resolved.entry.location,
        runtime.paths(),
        runtime.config(),
        identity,
        |event| frontend.launch_preparation(command_name, event),
    )?;
    if dry_run {
        return Ok(CommandOutput::LaunchPlan(plan.summary().into()));
    }
    let result = run_launch(plan, |event| frontend.launch_process(command_name, event)).await?;
    Ok(CommandOutput::LaunchResult(result.into()))
}

async fn execute_account(
    command: AccountCommands,
    instance_selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    let secrets = native_secret_store(runtime.paths())?;
    let backend = secrets.backend_name().to_string();
    match command {
        AccountCommands::Login { command } => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for account login".to_string(),
                ));
            }
            let (account, method) = match command {
                AccountLoginCommands::Offline { profile_name } => (
                    create_offline_account(runtime.paths(), &profile_name)?,
                    "offline",
                ),
                AccountLoginCommands::Microsoft { command } => match command {
                    MicrosoftLoginCommands::Begin => {
                        let client = runtime.config().http_client()?;
                        let session = begin_microsoft_device_login(
                            runtime.paths(),
                            &client,
                            secrets.as_ref(),
                        )
                        .await?;
                        return Ok(CommandOutput::MicrosoftDeviceSession(session.into()));
                    }
                    MicrosoftLoginCommands::Complete { login_session_id } => {
                        let client = runtime.config().http_client()?;
                        let account = complete_microsoft_device_login(
                            runtime.paths(),
                            &client,
                            secrets.as_ref(),
                            login_session_id,
                            |event| frontend.microsoft_login_progress(event),
                        )
                        .await?;
                        (account, "microsoft")
                    }
                },
                AccountLoginCommands::Yggdrasil {
                    provider,
                    username,
                    profile,
                    password_stdin,
                } => {
                    let password =
                        frontend.read_password("External Yggdrasil password: ", password_stdin)?;
                    let client = runtime.config().http_client()?;
                    let account = login_external_yggdrasil(
                        runtime.paths(),
                        runtime.config(),
                        &client,
                        secrets.as_ref(),
                        ExternalYggdrasilLoginRequest {
                            provider_id: &provider,
                            username: &username,
                            password: &password,
                            profile_selector: profile.as_deref(),
                        },
                    )
                    .await?;
                    (account, "external-yggdrasil")
                }
            };
            let repository = AccountRepository::load(runtime.paths())?;
            Ok(CommandOutput::AccountLogin(AccountLoginView {
                method,
                account: AccountView::new(
                    &account,
                    repository.default_account(),
                    &backend,
                    runtime.paths(),
                ),
            }))
        }
        AccountCommands::List => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for account list".to_string(),
                ));
            }
            let repository = AccountRepository::load(runtime.paths())?;
            let accounts = repository.accounts().to_vec();
            ensure_account_presentations(runtime, &accounts).await;
            let repository = AccountRepository::load(runtime.paths())?;
            Ok(CommandOutput::AccountList(AccountListView {
                accounts: repository
                    .accounts()
                    .iter()
                    .map(|account| {
                        AccountView::new(
                            account,
                            repository.default_account(),
                            &backend,
                            runtime.paths(),
                        )
                    })
                    .collect(),
            }))
        }
        AccountCommands::Show { account } => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for account show".to_string(),
                ));
            }
            let repository = AccountRepository::load(runtime.paths())?;
            let account = match account {
                Some(selector) => repository.get(&selector)?,
                None => repository.selected(None)?,
            }
            .clone();
            ensure_account_presentations(runtime, std::slice::from_ref(&account)).await;
            let repository = AccountRepository::load(runtime.paths())?;
            let account = repository.get(&account.id.to_string())?;
            Ok(CommandOutput::AccountDetail(AccountView::new(
                account,
                repository.default_account(),
                &backend,
                runtime.paths(),
            )))
        }
        AccountCommands::Refresh { account } => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for account refresh".to_string(),
                ));
            }
            let account_id = AccountRepository::load(runtime.paths())?.get(&account)?.id;
            let client = runtime.config().http_client()?;
            drop(
                resolve_launch_identity(
                    runtime.paths(),
                    runtime.config(),
                    &client,
                    secrets.as_ref(),
                    Some(account_id),
                )
                .await?,
            );
            let repository = AccountRepository::load(runtime.paths())?;
            let account = repository.get(&account_id.to_string())?.clone();
            ensure_account_presentations(runtime, std::slice::from_ref(&account)).await;
            let repository = AccountRepository::load(runtime.paths())?;
            let account = repository.get(&account_id.to_string())?;
            Ok(CommandOutput::AccountRefresh(AccountView::new(
                account,
                repository.default_account(),
                &backend,
                runtime.paths(),
            )))
        }
        AccountCommands::Select { account, global } => {
            let mut repository = AccountRepository::load(runtime.paths())?;
            let selected = repository.get(&account)?.clone();
            if global {
                if instance_selector.is_some() {
                    return Err(AppError::Argument(
                        "--instance cannot be combined with account select --global".to_string(),
                    ));
                }
                repository.set_default(Some(selected.id))?;
                Ok(CommandOutput::AccountSelection(AccountSelectionView {
                    scope: "global",
                    account: Some(AccountView::new(
                        &selected,
                        repository.default_account(),
                        &backend,
                        runtime.paths(),
                    )),
                }))
            } else {
                set_instance_account(instance_selector, current_dir, runtime, Some(selected.id))?;
                Ok(CommandOutput::AccountSelection(AccountSelectionView {
                    scope: "instance",
                    account: Some(AccountView::new(
                        &selected,
                        repository.default_account(),
                        &backend,
                        runtime.paths(),
                    )),
                }))
            }
        }
        AccountCommands::Clear { global } => {
            if global {
                if instance_selector.is_some() {
                    return Err(AppError::Argument(
                        "--instance cannot be combined with account clear --global".to_string(),
                    ));
                }
                AccountRepository::load(runtime.paths())?.set_default(None)?;
                Ok(CommandOutput::AccountSelection(AccountSelectionView {
                    scope: "global",
                    account: None,
                }))
            } else {
                set_instance_account(instance_selector, current_dir, runtime, None)?;
                Ok(CommandOutput::AccountSelection(AccountSelectionView {
                    scope: "instance",
                    account: None,
                }))
            }
        }
        AccountCommands::Logout { account } => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for account logout".to_string(),
                ));
            }
            let mut repository = AccountRepository::load(runtime.paths())?;
            let account = repository.get(&account)?.clone();
            let view = AccountView::new(
                &account,
                repository.default_account(),
                &backend,
                runtime.paths(),
            );
            repository.remove(account.id, secrets.as_ref()).await?;
            orbit_launcher_core::account::remove_account_avatars(runtime.paths(), account.id)?;
            Ok(CommandOutput::AccountLogout(AccountLogoutView {
                account: view,
                local_secret_deleted: true,
            }))
        }
    }
}

async fn ensure_account_presentations(runtime: &RuntimeContext, accounts: &[AccountMetadata]) {
    let Ok(client) = runtime.config().http_client() else {
        return;
    };
    for account in accounts {
        let _ = orbit_launcher_core::account::ensure_account_presentation(
            runtime.paths(),
            runtime.config(),
            &client,
            account,
        )
        .await;
    }
}

fn set_instance_account(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    account: Option<uuid::Uuid>,
) -> Result<(), AppError> {
    let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
    let resolved = resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
    if resolved.manifest.kind != InstanceKind::Client {
        return Err(AppError::Argument(
            "server instances do not use client accounts".to_string(),
        ));
    }
    let mut manifest = ManifestFile::open(resolved.entry.instance_directory())?;
    manifest.inner.launch.account = account;
    manifest.save()?;
    Ok(())
}

struct InstallCommandRequest {
    new_name: Option<String>,
    server_directory: Option<std::path::PathBuf>,
    kind: Option<crate::cli::InstanceKindArg>,
    minecraft: Option<String>,
    loader: Option<crate::cli::LoaderKindArg>,
    loader_version: Option<String>,
}

async fn execute_install(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
    request: InstallCommandRequest,
) -> Result<CommandOutput, AppError> {
    let created = if let Some(name) = request.new_name {
        if selector.is_some() {
            return Err(AppError::Argument(
                "--instance cannot be combined with install --new".to_string(),
            ));
        }
        let kind: InstanceKind = request
            .kind
            .ok_or_else(|| AppError::Argument("install --new requires --kind".to_string()))?
            .into();
        let minecraft = request
            .minecraft
            .ok_or_else(|| AppError::Argument("install --new requires --minecraft".to_string()))?;
        let directory = match kind {
            InstanceKind::Client => {
                if request.server_directory.is_some() {
                    return Err(AppError::Argument(
                        "--server-directory is invalid for client instances; use 'minecraft move' to relocate the managed repository"
                            .to_string(),
                    ));
                }
                runtime.minecraft_directory()
            }
            InstanceKind::Server => {
                resolve_directory(current_dir, request.server_directory.as_deref())?
            }
        };
        Some(create_instance(
            runtime.paths(),
            CreateInstanceRequest {
                directory,
                name,
                kind,
                minecraft_requirement: minecraft,
                loader_kind: request
                    .loader
                    .unwrap_or(crate::cli::LoaderKindArg::Vanilla)
                    .into(),
                loader_requirement: request.loader_version,
            },
        )?)
    } else {
        if request.server_directory.is_some()
            || request.kind.is_some()
            || request.minecraft.is_some()
            || request.loader.is_some()
            || request.loader_version.is_some()
        {
            return Err(AppError::Argument(
                "--server-directory, --kind, --minecraft, --loader, and --loader-version require install --new"
                    .to_string(),
            ));
        }
        None
    };
    let created_selector = created.as_ref().map(|created| created.entry.id.to_string());
    let selector = created_selector.as_deref().or(selector);
    let result = execute_existing_install(selector, current_dir, runtime, frontend).await;
    if result.is_err()
        && let Some(created) = created
    {
        rollback_created_instance(runtime.paths(), &created.entry.id.to_string()).map_err(
            |rollback| {
                AppError::Core(LauncherError::Transaction(format!(
                    "bootstrap failed and its provisional instance could not be rolled back: {rollback}"
                )))
            },
        )?;
    }
    result
}

async fn execute_existing_install(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
    let resolved = resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
    let client = runtime.config().http_client()?;
    let plan = prepare_install(
        &resolved.entry.location,
        &client,
        runtime.config().java.default_provider,
        |event| frontend.progress(event),
    )
    .await?;
    if let Some(server) = plan.server() {
        ensure_server_eula_accepted(resolved.entry.instance_directory(), server, frontend)?;
    }
    let result = apply_install_plan(
        &resolved.entry.location,
        runtime.paths(),
        &client,
        plan,
        usize::from(runtime.config().network.concurrency),
        runtime.config().installer.timeout_seconds,
        |event| frontend.progress(event),
    )
    .await?;
    let java = result.lock.java.as_ref().ok_or_else(|| {
        LauncherError::InvalidLock("completed install did not lock a Java runtime".to_string())
    })?;
    Ok(CommandOutput::Install(InstallView {
        instance_id: resolved.entry.id.to_string(),
        kind: resolved.manifest.kind.as_str().to_string(),
        minecraft_version: result.lock.minecraft.version.clone(),
        loader: result.lock.loader.kind.as_str().to_string(),
        java_runtime_id: java.runtime_id.clone(),
        java_version: java.version.clone(),
        eula_digest_sha256: result
            .lock
            .eula
            .as_ref()
            .map(|acceptance| acceptance.digest_sha256.clone()),
        downloaded_artifacts: result.downloaded_artifacts,
        cached_artifacts: result.cached_artifacts,
    }))
}

fn ensure_server_eula_accepted(
    instance_root: &Path,
    plan: &ServerInstallPlan,
    frontend: &mut dyn Frontend,
) -> Result<(), AppError> {
    if plan.eula_is_accepted() {
        return Ok(());
    }
    if !frontend.confirm_eula(plan.eula())? {
        return Err(LauncherError::EulaRequired(
            "the user did not accept the current Minecraft EULA".to_string(),
        )
        .into());
    }
    accept_shown_eula(
        instance_root,
        &plan.eula().digest_sha256,
        EulaAcceptanceMethod::InteractivePrompt,
    )?;
    Ok(())
}

async fn execute_server(
    command: ServerCommands,
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    let resolved = resolve_server_instance(selector, current_dir, runtime)?;
    match command {
        ServerCommands::Run { dry_run: true } => {
            execute_launch(
                selector,
                current_dir,
                runtime,
                frontend,
                InstanceKind::Server,
                true,
            )
            .await
        }
        ServerCommands::Run { dry_run: false } => {
            execute_foreground_supervisor(&resolved, runtime, frontend).await
        }
        ServerCommands::Start => start_detached_supervisor(&resolved, runtime).await,
        ServerCommands::Stop => {
            execute_supervisor_control(&resolved, runtime, IpcRequest::Stop, "stop").await
        }
        ServerCommands::Status => {
            let response = supervisor_request(
                runtime.paths().data_dir(),
                resolved.entry.id,
                IpcRequest::Status,
            )
            .await?;
            Ok(CommandOutput::ServerStatus(ServerStatusView {
                running: response.is_some(),
                state: response.map(|response| response.state),
            }))
        }
        ServerCommands::Command { value } => {
            let command = value.join(" ");
            validate_console_command(&command)?;
            execute_supervisor_control(
                &resolved,
                runtime,
                IpcRequest::SendCommand { value: command },
                "command",
            )
            .await
        }
        ServerCommands::Eula { command } => match command {
            EulaCommands::Show => {
                let client = runtime.config().http_client()?;
                let document =
                    show_current_eula(resolved.entry.instance_directory(), &client).await?;
                Ok(CommandOutput::EulaDocument(EulaDocumentView {
                    instance_id: resolved.entry.id.to_string(),
                    url: document.url,
                    digest_sha256: document.digest_sha256,
                    fetched_at_unix_seconds: document.fetched_at_unix_seconds,
                    text: document.text,
                }))
            }
            EulaCommands::Accept { digest } => {
                let acceptance = accept_shown_eula(
                    resolved.entry.instance_directory(),
                    &digest,
                    EulaAcceptanceMethod::DigestCommand,
                )?;
                Ok(CommandOutput::EulaAcceptance(EulaAcceptanceView {
                    instance_id: resolved.entry.id.to_string(),
                    url: acceptance.url,
                    digest_sha256: acceptance.digest_sha256,
                    accepted_at_unix_seconds: acceptance.accepted_at_unix_seconds,
                    method: acceptance.method.as_str(),
                }))
            }
        },
    }
}

fn resolve_server_instance(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
) -> Result<ResolvedInstance, AppError> {
    let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
    let resolved = resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
    if resolved.manifest.kind != InstanceKind::Server {
        return Err(AppError::Argument(format!(
            "instance '{}' is a client; server commands require a server instance",
            resolved.entry.name
        )));
    }
    Ok(resolved)
}

fn prepare_server_plan(
    resolved: &ResolvedInstance,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
    command: &'static str,
) -> Result<orbit_launcher_core::LaunchPlan, AppError> {
    prepare_launch(
        &resolved.entry.location,
        runtime.paths(),
        runtime.config(),
        None,
        |event| frontend.launch_preparation(command, event),
    )
    .map_err(AppError::from)
}

async fn execute_foreground_supervisor(
    resolved: &ResolvedInstance,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    let plan = prepare_server_plan(resolved, runtime, frontend, "server.run")?;
    let config =
        resolved.manifest.server.as_ref().ok_or_else(|| {
            LauncherError::InvalidManifest("server configuration is missing".into())
        })?;
    let (controls, mut receiver) = mpsc::unbounded_channel();
    let stdin_controls = controls.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let command = line.trim();
            if command.is_empty() {
                continue;
            }
            let control = if command == "stop" {
                SupervisorControl::Stop
            } else {
                SupervisorControl::Command(command.to_string())
            };
            if stdin_controls.send(control).is_err() {
                break;
            }
        }
    });
    let signal_controls = controls.clone();
    let signal_task = tokio::spawn(async move {
        if wait_for_shutdown_signal().await.is_ok() {
            let _ = signal_controls.send(SupervisorControl::Stop);
        }
    });
    drop(controls);
    let result = supervise_server(plan, config, &mut receiver, |event| {
        frontend.supervisor_event("server.run", event)
    })
    .await;
    signal_task.abort();
    Ok(CommandOutput::SupervisorResult(result?.into()))
}

async fn execute_internal_supervisor(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    let resolved = resolve_server_instance(selector, current_dir, runtime)?;
    let _lock = SupervisorLock::acquire(resolved.entry.instance_directory())?;
    let plan = prepare_server_plan(&resolved, runtime, frontend, "server.supervisor")?;
    let config =
        resolved.manifest.server.clone().ok_or_else(|| {
            LauncherError::InvalidManifest("server configuration is missing".into())
        })?;
    let state = Arc::new(RwLock::new(SupervisorState::starting(resolved.entry.id)?));
    let server = IpcServer::bind(runtime.paths().data_dir(), resolved.entry.id).await?;
    let (controls, mut receiver) = mpsc::unbounded_channel();
    let signal_controls = controls.clone();
    let signal_task = tokio::spawn(async move {
        if wait_for_shutdown_signal().await.is_ok() {
            let _ = signal_controls.send(SupervisorControl::Stop);
        }
    });
    let (shutdown, shutdown_receiver) = watch::channel(false);
    let ipc_task = tokio::spawn(server.serve(controls, Arc::clone(&state), shutdown_receiver));
    let result = supervise_server(plan, &config, &mut receiver, |event| {
        if let Ok(mut current) = state.write() {
            current.apply(&event);
        }
        frontend.supervisor_event("server.supervisor", event);
    })
    .await;
    signal_task.abort();
    let _ = shutdown.send(true);
    ipc_task
        .await
        .map_err(|error| LauncherError::Launch(format!("supervisor IPC task failed: {error}")))??;
    Ok(CommandOutput::SupervisorResult(result?.into()))
}

#[cfg(windows)]
async fn wait_for_shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

async fn start_detached_supervisor(
    resolved: &ResolvedInstance,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    if let Some(response) = supervisor_request(
        runtime.paths().data_dir(),
        resolved.entry.id,
        IpcRequest::Status,
    )
    .await?
    {
        return Err(LauncherError::Launch(format!(
            "server supervisor is already running as process {}",
            response.state.supervisor_pid
        ))
        .into());
    }

    let log_directory = resolved.entry.instance_directory().join(".orbit-launcher");
    std::fs::create_dir_all(&log_directory).map_err(LauncherError::from)?;
    let stdout_log = log_directory.join("supervisor.stdout.log");
    let stderr_log = log_directory.join("supervisor.stderr.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_log)
        .map_err(LauncherError::from)?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_log)
        .map_err(LauncherError::from)?;
    let executable = std::env::current_exe().map_err(LauncherError::from)?;
    let mut command = std::process::Command::new(executable);
    command
        .arg("--instance")
        .arg(resolved.entry.id.to_string())
        .arg("--format")
        .arg("json")
        .arg("--progress-format")
        .arg("ndjson")
        .arg("--non-interactive")
        .arg("--config-dir")
        .arg(runtime.paths().config_dir())
        .arg("--data-dir")
        .arg(runtime.paths().data_dir())
        .arg("--cache-dir")
        .arg(runtime.paths().cache_dir())
        .arg("__supervisor")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|error| {
        LauncherError::Launch(format!("failed to start server supervisor: {error}"))
    })?;
    let expected_pid = child.id();
    for _ in 0..150 {
        if let Some(response) = supervisor_request(
            runtime.paths().data_dir(),
            resolved.entry.id,
            IpcRequest::Status,
        )
        .await?
        {
            if response.state.supervisor_pid != expected_pid {
                return Err(LauncherError::Launch(format!(
                    "another supervisor won the startup race as process {}",
                    response.state.supervisor_pid
                ))
                .into());
            }
            return Ok(CommandOutput::ServerStart(ServerStartView {
                state: response.state,
                stdout_log,
                stderr_log,
            }));
        }
        if let Some(status) = child.try_wait().map_err(LauncherError::from)? {
            return Err(LauncherError::Launch(format!(
                "server supervisor exited before becoming ready ({status}); inspect '{}' and '{}'",
                stdout_log.display(),
                stderr_log.display()
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(LauncherError::Launch(format!(
        "server supervisor did not become ready within 30 seconds; inspect '{}' and '{}'",
        stdout_log.display(),
        stderr_log.display()
    ))
    .into())
}

async fn execute_supervisor_control(
    resolved: &ResolvedInstance,
    runtime: &RuntimeContext,
    request: IpcRequest,
    action: &'static str,
) -> Result<CommandOutput, AppError> {
    let response = supervisor_request(runtime.paths().data_dir(), resolved.entry.id, request)
        .await?
        .ok_or_else(|| LauncherError::Launch("server supervisor is not running".to_string()))?;
    if !response.accepted {
        return Err(LauncherError::Launch(response.message).into());
    }
    Ok(CommandOutput::ServerControl(ServerControlView {
        action,
        accepted: response.accepted,
        message: response.message,
        state: response.state,
    }))
}

fn validate_console_command(command: &str) -> Result<(), AppError> {
    if command.is_empty() {
        return Err(AppError::Argument(
            "server command must not be empty".to_string(),
        ));
    }
    if command.len() > 32 * 1024 || command.chars().any(char::is_control) {
        return Err(AppError::Argument(
            "server command must be at most 32 KiB and contain no control characters".to_string(),
        ));
    }
    Ok(())
}

async fn execute_config(
    command: ConfigCommands,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    let path = runtime.paths().config_file();
    match command {
        ConfigCommands::Path => Ok(CommandOutput::ConfigPath(ConfigPathView { path })),
        ConfigCommands::List => Ok(CommandOutput::ConfigList(ConfigListView {
            settings: list_config(&path)?
                .into_iter()
                .map(ConfigEntryView::from)
                .collect(),
        })),
        ConfigCommands::Get { key } => {
            let key = key.parse::<ConfigKey>()?;
            Ok(CommandOutput::ConfigEntry(get_config(&path, key)?.into()))
        }
        ConfigCommands::Set { key, value } => {
            let key = key.parse::<ConfigKey>()?;
            Ok(CommandOutput::ConfigMutation(ConfigMutationView::new(
                set_config(&path, key, &value)?,
                ConfigMutationAction::Set,
            )))
        }
        ConfigCommands::Unset { key } => {
            let key = key.parse::<ConfigKey>()?;
            Ok(CommandOutput::ConfigMutation(ConfigMutationView::new(
                unset_config(&path, key)?,
                ConfigMutationAction::Unset,
            )))
        }
        ConfigCommands::Yggdrasil { command } => match command {
            YggdrasilProviderCommands::List => {
                let config = orbit_launcher_core::GlobalConfig::load(&path)?;
                Ok(CommandOutput::YggdrasilProviderList(
                    YggdrasilProviderListView {
                        providers: config
                            .yggdrasil
                            .providers
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                    },
                ))
            }
            YggdrasilProviderCommands::Add {
                id,
                api_root,
                allow_insecure_http,
            } => {
                let client = runtime.config().http_client()?;
                let api_root = orbit_launcher_core::discover_yggdrasil_api_root(
                    &client,
                    &api_root,
                    allow_insecure_http,
                )
                .await?;
                Ok(CommandOutput::YggdrasilProviderMutation(
                    YggdrasilProviderMutationView {
                        action: "added",
                        provider: YggdrasilProviderView::from(add_yggdrasil_provider(
                            &path,
                            YggdrasilProviderConfig {
                                id,
                                api_root,
                                allow_insecure_http,
                            },
                        )?),
                    },
                ))
            }
            YggdrasilProviderCommands::Remove { id } => Ok(
                CommandOutput::YggdrasilProviderMutation(YggdrasilProviderMutationView {
                    action: "removed",
                    provider: remove_yggdrasil_provider(&path, &id)?.into(),
                }),
            ),
        },
    }
}

fn execute_instance(
    command: InstanceCommands,
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    let registry_path = runtime.paths().instances_file();
    match command {
        InstanceCommands::Create {
            name,
            server_directory,
            kind,
            minecraft,
            loader,
            loader_version,
        } => {
            let kind: InstanceKind = kind.into();
            let directory = match kind {
                InstanceKind::Client => {
                    if server_directory.is_some() {
                        return Err(AppError::Argument(
                            "--server-directory is invalid for client instances; use 'minecraft move' to relocate the managed repository"
                                .to_string(),
                        ));
                    }
                    runtime.minecraft_directory()
                }
                InstanceKind::Server => {
                    resolve_directory(current_dir, server_directory.as_deref())?
                }
            };
            let created = create_instance(
                runtime.paths(),
                CreateInstanceRequest {
                    directory,
                    name,
                    kind,
                    minecraft_requirement: minecraft,
                    loader_kind: loader.into(),
                    loader_requirement: loader_version,
                },
            )?;
            Ok(CommandOutput::InstanceMutation(InstanceMutationView {
                instance: InstanceView::from_entry(&created.entry, None),
                action: InstanceMutationAction::Created,
                files_deleted: false,
            }))
        }
        InstanceCommands::Import { directory } => {
            let directory = resolve_directory(current_dir, directory.as_deref())?;
            let imported = import_instance(runtime.paths(), &directory)?;
            let registry = InstanceRegistry::load(&registry_path)?;
            Ok(CommandOutput::InstanceMutation(InstanceMutationView {
                instance: InstanceView::from_entry(&imported.entry, registry.default_instance),
                action: InstanceMutationAction::Imported,
                files_deleted: false,
            }))
        }
        InstanceCommands::List => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let instances = registry
                .instances
                .iter()
                .map(|entry| InstanceView::from_entry(entry, registry.default_instance))
                .collect();
            Ok(CommandOutput::InstanceList(InstanceListView { instances }))
        }
        InstanceCommands::Show => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::ReadOnly)?;
            let installed =
                orbit_launcher_core::LockFile::open_optional(resolved.entry.instance_directory())?;
            Ok(CommandOutput::InstanceDetail(InstanceDetailView::new(
                &resolved.entry,
                &resolved.manifest,
                installed.as_ref().map(|lock| &lock.inner),
                registry.default_instance,
                resolved.source,
            )))
        }
        InstanceCommands::Rename { new_name } => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let renamed =
                rename_instance(runtime.paths(), &resolved.entry.id.to_string(), &new_name)?;
            Ok(CommandOutput::Rename(RenameView {
                id: renamed.id.to_string(),
                old_name: renamed.old_name,
                new_name: renamed.new_name,
            }))
        }
        InstanceCommands::Configure {
            minecraft,
            loader,
            loader_version,
            java_policy,
        } => {
            if minecraft.is_none()
                && loader.is_none()
                && loader_version.is_none()
                && java_policy.is_none()
            {
                return Err(AppError::Argument(
                    "instance configure requires at least one desired runtime option".to_string(),
                ));
            }
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let configured = configure_instance(
                runtime.paths(),
                &resolved.entry.id.to_string(),
                orbit_launcher_core::ConfigureInstanceRequest {
                    minecraft_requirement: minecraft,
                    loader_kind: loader.map(Into::into),
                    loader_requirement: loader_version,
                    java_policy: java_policy.map(Into::into),
                },
            )?;
            let installed = orbit_launcher_core::LockFile::open_optional(
                configured.entry.instance_directory(),
            )?;
            Ok(CommandOutput::InstanceConfigured(InstanceDetailView::new(
                &configured.entry,
                &configured.manifest,
                installed.as_ref().map(|lock| &lock.inner),
                registry.default_instance,
                resolved.source,
            )))
        }
        InstanceCommands::Remove => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let removed = remove_instance(runtime.paths(), &resolved.entry.id.to_string())?;
            Ok(CommandOutput::InstanceMutation(InstanceMutationView {
                instance: InstanceView::from_entry(&removed.entry, None),
                action: InstanceMutationAction::Removed,
                files_deleted: false,
            }))
        }
        InstanceCommands::Default { command } => {
            let selected = match command {
                DefaultCommands::Set { instance } => {
                    set_default_instance(runtime.paths(), Some(&instance))?
                }
                DefaultCommands::Clear => set_default_instance(runtime.paths(), None)?,
                DefaultCommands::Show => InstanceRegistry::load(&registry_path)?
                    .default_entry()
                    .cloned(),
            };
            Ok(CommandOutput::Default(DefaultView {
                instance: selected
                    .as_ref()
                    .map(|entry| InstanceView::from_entry(entry, Some(entry.id))),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{DefaultCommands, InstanceKindArg, LoaderKindArg};
    use orbit_launcher_core::{ContextSource, RuntimePathOptions};

    struct NoopFrontend;

    impl Frontend for NoopFrontend {
        fn progress(&mut self, _event: InstallProgressEvent) {}

        fn confirm_eula(&mut self, _document: &EulaDocument) -> Result<bool, LauncherError> {
            Err(LauncherError::InteractionRequired(
                "test frontend does not prompt".to_string(),
            ))
        }

        fn read_password(
            &mut self,
            _prompt: &str,
            _stdin: bool,
        ) -> Result<Zeroizing<String>, LauncherError> {
            Err(LauncherError::InteractionRequired(
                "test frontend does not read passwords".to_string(),
            ))
        }

        fn microsoft_login_progress(&mut self, _event: MicrosoftLoginProgressEvent) {}

        fn launch_preparation(&mut self, _command: &'static str, _event: LaunchPreparationEvent) {}

        fn launch_process(&mut self, _command: &'static str, _event: LaunchProcessEvent) {}

        fn supervisor_event(&mut self, _command: &'static str, _event: SupervisorEvent) {}

        fn repository_move(&mut self, _event: RepositoryMoveEvent) {}
    }

    fn runtime(directory: &Path) -> RuntimeContext {
        RuntimeContext::load(RuntimePathOptions {
            config_dir: Some(directory.join("config")),
            data_dir: Some(directory.join("data")),
            cache_dir: Some(directory.join("cache")),
        })
        .unwrap()
    }

    #[test]
    fn current_directory_and_explicit_global_id_resolve_the_same_instance() {
        let directory = tempfile::tempdir().unwrap();
        let instance_root = directory.path().join("instance");
        std::fs::create_dir_all(&instance_root).unwrap();
        let instance_root = dunce::canonicalize(instance_root).unwrap();
        let runtime = runtime(directory.path());

        let created = execute_instance(
            InstanceCommands::Create {
                name: "server".to_string(),
                server_directory: None,
                kind: InstanceKindArg::Server,
                minecraft: "1.21.1".to_string(),
                loader: LoaderKindArg::Fabric,
                loader_version: Some("stable".to_string()),
            },
            None,
            &instance_root,
            &runtime,
        )
        .unwrap();
        let CommandOutput::InstanceMutation(created) = created else {
            panic!("unexpected create output");
        };

        let local =
            execute_instance(InstanceCommands::Show, None, &instance_root, &runtime).unwrap();
        let CommandOutput::InstanceDetail(local) = local else {
            panic!("unexpected local show output");
        };
        assert_eq!(local.context, ContextSource::CurrentDirectory);

        let unrelated = directory.path().join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();
        let global = execute_instance(
            InstanceCommands::Show,
            Some(&created.instance.id),
            &unrelated,
            &runtime,
        )
        .unwrap();
        let CommandOutput::InstanceDetail(global) = global else {
            panic!("unexpected global show output");
        };
        assert_eq!(global.context, ContextSource::Explicit);
        assert_eq!(global.instance.id, created.instance.id);
    }

    #[test]
    fn default_is_read_only_and_cannot_target_a_rename() {
        let directory = tempfile::tempdir().unwrap();
        let instance_root = directory.path().join("instance");
        std::fs::create_dir_all(&instance_root).unwrap();
        let instance_root = dunce::canonicalize(instance_root).unwrap();
        let runtime = runtime(directory.path());
        execute_instance(
            InstanceCommands::Create {
                name: "client".to_string(),
                server_directory: None,
                kind: InstanceKindArg::Client,
                minecraft: "latest-release".to_string(),
                loader: LoaderKindArg::Vanilla,
                loader_version: None,
            },
            None,
            &instance_root,
            &runtime,
        )
        .unwrap();
        execute_instance(
            InstanceCommands::Default {
                command: DefaultCommands::Set {
                    instance: "client".to_string(),
                },
            },
            None,
            &instance_root,
            &runtime,
        )
        .unwrap();

        let unrelated = directory.path().join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();
        let error = execute_instance(
            InstanceCommands::Rename {
                new_name: "renamed".to_string(),
            },
            None,
            &unrelated,
            &runtime,
        )
        .unwrap_err();
        assert_eq!(error.code(), "explicit_instance_required");
    }

    #[tokio::test]
    async fn config_commands_use_typed_core_mutations() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = runtime(directory.path());
        let mut frontend = NoopFrontend;
        let output = execute(
            Commands::Config {
                command: ConfigCommands::Set {
                    key: "cache.max-size".to_string(),
                    value: "4 GiB".to_string(),
                },
            },
            None,
            directory.path(),
            &runtime,
            &mut frontend,
        )
        .await
        .unwrap();
        let CommandOutput::ConfigMutation(view) = output else {
            panic!("unexpected config output");
        };
        assert_eq!(view.key, "cache.max-size");
        assert_eq!(view.current.as_deref(), Some("4 GiB"));
        assert!(view.explicit);

        let error = execute(
            Commands::Config {
                command: ConfigCommands::Get {
                    key: "unknown".to_string(),
                },
            },
            Some("server"),
            directory.path(),
            &runtime,
            &mut frontend,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), "argument");
    }
}
