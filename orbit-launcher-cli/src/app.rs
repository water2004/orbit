use std::fs::OpenOptions;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use orbit_launcher_core::{
    AccountMetadata, AccountRepository, ConfigKey, ContextIntent, EulaAcceptanceMethod,
    EulaDocument, ExternalYggdrasilLoginRequest, InstallProgressEvent, InstanceKind,
    InstanceRegistry, LaunchPreparationEvent, LaunchProcessEvent, LauncherError,
    MicrosoftLoginProgressEvent, RepositoryMoveEvent, ResolvedInstance, RuntimeContext,
    ServerInstallPlan, StateArchiveProgressEvent, SupervisorControl, SupervisorEvent,
    YggdrasilProviderConfig, accept_shown_eula, add_yggdrasil_provider, apply_install_plan,
    begin_microsoft_device_login, complete_microsoft_device_login, configure_instance,
    create_offline_account, export_launcher_state_with_base, get_config, import_instance,
    inspect_install_pack, list_config, login_external_yggdrasil, move_minecraft_directory,
    native_secret_store, prepare_install, prepare_launch, remove_instance,
    remove_yggdrasil_provider, rename_instance, resolve_directory, resolve_instance,
    resolve_launch_identity, restore_launcher_state, run_launch, set_client_resolution, set_config,
    set_default_instance, show_current_eula, supervise_server, unset_config,
};
use tokio::sync::{mpsc, watch};
use zeroize::Zeroizing;

use crate::cli::{
    AccountCommands, AccountDefaultCommands, AccountLoginCommands, Commands, ConfigCommands,
    DefaultCommands, EulaCommands, InstanceAccountCommands, InstanceCommands, JavaCommands,
    MicrosoftLoginCommands, MinecraftCommands, PackageCommands, ResolutionCommands, ServerCommands,
    VersionCommands, YggdrasilProviderCommands,
};
use crate::output::{
    AccountListView, AccountLoginView, AccountLogoutView, AccountSelectionView, AccountView,
    ConfigEntryView, ConfigListView, ConfigMutationAction, ConfigMutationView, ConfigPathView,
    DefaultView, EulaAcceptanceView, EulaDocumentView, InstallView, InstanceDetailView,
    InstanceListView, InstanceMutationAction, InstanceMutationView, InstanceView,
    JavaRequirementView, JavaRuntimeListView, JavaRuntimeMutationView, LaunchPlanView,
    LaunchResultView, LauncherStateExportView, LoaderVersionCatalogView,
    MicrosoftDeviceSessionView, MinecraftDirectoryMoveView, MinecraftDirectoryView,
    MinecraftVersionCatalogView, PackageRequirementView, RenameView, ServerControlView,
    ServerStartView, ServerStatusView, SupervisorResultView, YggdrasilEndpointView,
    YggdrasilProviderListView, YggdrasilProviderMutationView, YggdrasilProviderView,
};
use crate::supervisor_ipc::{
    IpcRequest, IpcServer, SupervisorLock, SupervisorState, request as supervisor_request,
};

#[derive(Debug)]
pub enum CommandOutput {
    UpdateCheck(orbit_launcher_core::updates::UpdateCheck),
    ConfigPath(ConfigPathView),
    ConfigList(ConfigListView),
    ConfigEntry(ConfigEntryView),
    ConfigMutation(ConfigMutationView),
    EulaDocument(EulaDocumentView),
    EulaAcceptance(EulaAcceptanceView),
    Install(InstallView),
    LaunchPlan(LaunchPlanView),
    LaunchResult(LaunchResultView),
    LaunchReport(Option<orbit_launcher_core::launch_record::LaunchRecord>),
    ServerStart(ServerStartView),
    ServerStatus(ServerStatusView),
    ServerControl(ServerControlView),
    SupervisorResult(SupervisorResultView),
    InstanceList(InstanceListView),
    InstanceDetail(InstanceDetailView),
    InstanceMutation(InstanceMutationView),
    InstanceSources(orbit_launcher_core::ExternalInstanceCatalog),
    InstanceImportPreview(orbit_launcher_core::ExternalImportPlan),
    InstanceImported(crate::output::InstanceImportView),
    InstanceDeletion(orbit_launcher_core::InstanceDeletionPlan),
    Rename(RenameView),
    InstanceConfigured(InstanceDetailView),
    InstanceResolution(InstanceDetailView),
    InstanceMemory(crate::output::InstanceMemoryView),
    InstanceIcon(InstanceDetailView),
    InstanceJava(InstanceDetailView),
    LauncherStateExport(LauncherStateExportView),
    Default(DefaultView),
    AccountList(AccountListView),
    AccountDetail(AccountView),
    AccountRefresh(AccountView),
    AccountLogin(AccountLoginView),
    AccountSelection(AccountSelectionView),
    AccountLogout(AccountLogoutView),
    MicrosoftDeviceSession(MicrosoftDeviceSessionView),
    YggdrasilProviderList(YggdrasilProviderListView),
    YggdrasilEndpoint(YggdrasilEndpointView),
    YggdrasilProviderMutation(YggdrasilProviderMutationView),
    JavaRuntimeList(JavaRuntimeListView),
    JavaRuntimeMutation(JavaRuntimeMutationView),
    MinecraftVersions(MinecraftVersionCatalogView),
    LoaderVersions(LoaderVersionCatalogView),
    JavaRequirement(JavaRequirementView),
    MinecraftDirectory(MinecraftDirectoryView),
    MinecraftDirectoryMove(MinecraftDirectoryMoveView),
    PackageRequirement(PackageRequirementView),
}

impl CommandOutput {
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
    fn external_import_progress(&mut self, event: orbit_launcher_core::ExternalImportEvent) {
        if let orbit_launcher_core::ExternalImportEvent::Install(event) = event {
            self.progress(event);
        }
    }

    fn confirm_instance_deletion(
        &mut self,
        _plan: &orbit_launcher_core::InstanceDeletionPlan,
    ) -> Result<bool, LauncherError> {
        Err(LauncherError::InteractionRequired(
            "complete instance deletion requires explicit confirmation (--yes)".into(),
        ))
    }
    fn progress(&mut self, event: InstallProgressEvent);

    fn confirm_eula(&mut self, document: &EulaDocument) -> Result<bool, LauncherError>;

    fn read_password(
        &mut self,
        prompt: &str,
        stdin: bool,
    ) -> Result<Zeroizing<String>, LauncherError>;

    fn microsoft_login_progress(&mut self, event: MicrosoftLoginProgressEvent);

    fn launch_preparation(&mut self, event: LaunchPreparationEvent);

    fn launch_process(&mut self, event: LaunchProcessEvent);

    fn supervisor_event(&mut self, event: SupervisorEvent);

    fn repository_move(&mut self, event: RepositoryMoveEvent);

    fn state_archive_progress(&mut self, event: StateArchiveProgressEvent);
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
    if instance_selector.is_some() && !command.accepts_instance_context() {
        return Err(AppError::Argument(
            "--instance is not valid for this command".into(),
        ));
    }
    match command {
        Commands::Update {
            command: crate::cli::UpdateCommands::Check { current_version },
        } => Ok(CommandOutput::UpdateCheck(
            orbit_launcher_core::updates::check(
                runtime.config(),
                current_version
                    .as_deref()
                    .unwrap_or(env!("CARGO_PKG_VERSION")),
            )
            .await?,
        )),
        Commands::Install {
            new,
            server_directory,
            kind,
            minecraft,
            loader,
            loader_version,
            from,
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
                    from,
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
        Commands::Export { output, base } => {
            let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
            let resolved = resolve_instance(
                &registry,
                instance_selector,
                current_dir,
                ContextIntent::ReadOnly,
            )?;
            let report = export_launcher_state_with_base(
                resolved.entry.instance_directory(),
                base.as_deref(),
                &output,
                |event| frontend.state_archive_progress(event),
            )?;
            Ok(CommandOutput::LauncherStateExport(report.into()))
        }
        Commands::Config { command } => execute_config(command, runtime).await,
        Commands::Instance { command } => match command {
            InstanceCommands::Import {
                directory,
                source,
                name,
                game_directory,
                dry_run,
            } => {
                let directory = resolve_directory(current_dir, directory.as_deref())?;
                let catalog = orbit_launcher_core::inspect_external_instances(&directory)?;
                if catalog.instances.len() == 1 && catalog.instances[0].format == "orbit" {
                    if source.is_some() || name.is_some() || game_directory.is_some() || dry_run {
                        return Err(AppError::Argument("Orbit instances are registered in place; external import options do not apply".into()));
                    }
                    let imported = import_instance(runtime.paths(), &directory)?;
                    return Ok(CommandOutput::InstanceImported(
                        crate::output::InstanceImportView {
                            instance: InstanceView::from_entry(&imported.entry, None),
                            copied: false,
                            source_directory: directory,
                            files: 0,
                            bytes: 0,
                            warnings: Vec::new(),
                        },
                    ));
                }
                let plan = orbit_launcher_core::plan_external_import(
                    runtime,
                    orbit_launcher_core::ExternalImportRequest {
                        directory,
                        source,
                        name,
                        game_directory: game_directory
                            .map(|p| resolve_directory(current_dir, Some(&p)))
                            .transpose()?,
                    },
                )?;
                if dry_run {
                    return Ok(CommandOutput::InstanceImportPreview(plan));
                }
                let result =
                    orbit_launcher_core::import_external_instance(runtime, plan, |event| {
                        frontend.external_import_progress(event)
                    })
                    .await?;
                Ok(CommandOutput::InstanceImported(result.into()))
            }
            InstanceCommands::Delete { yes, dry_run } => {
                let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
                let resolved = resolve_instance(
                    &registry,
                    instance_selector,
                    current_dir,
                    ContextIntent::Sensitive,
                )?;
                let plan = orbit_launcher_core::plan_instance_deletion(
                    runtime.paths(),
                    &resolved.entry.id.to_string(),
                )?;
                if dry_run {
                    return Ok(CommandOutput::InstanceDeletion(plan));
                }
                if !yes && !frontend.confirm_instance_deletion(&plan)? {
                    return Err(LauncherError::InteractionRequired(
                        "instance deletion was cancelled".into(),
                    )
                    .into());
                }
                let entry = orbit_launcher_core::delete_instance(runtime.paths(), &plan)?;
                Ok(CommandOutput::InstanceMutation(InstanceMutationView {
                    instance: InstanceView::from_entry(&entry, None),
                    action: InstanceMutationAction::Deleted,
                    files_deleted: true,
                }))
            }
            other => execute_instance(other, instance_selector, current_dir, runtime),
        },
        Commands::Server { command } => {
            execute_server(command, instance_selector, current_dir, runtime, frontend).await
        }
        Commands::Account { command } => execute_account(command, runtime, frontend).await,
        Commands::Java { command } => execute_java(command, runtime),
        Commands::Minecraft { command } => {
            execute_minecraft(command, current_dir, runtime, frontend)
        }
        Commands::Versions { command } => execute_versions(command, runtime).await,
        Commands::Package { command } => match command {
            PackageCommands::Inspect { source } => Ok(CommandOutput::PackageRequirement(
                inspect_install_pack(&source)?.into(),
            )),
        },
        Commands::Supervisor => {
            execute_internal_supervisor(instance_selector, current_dir, runtime, frontend).await
        }
    }
}

async fn execute_versions(
    command: VersionCommands,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    let client = runtime.config().download_client()?;
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
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    match command {
        JavaCommands::List { verify } => Ok(CommandOutput::JavaRuntimeList(JavaRuntimeListView {
            sources: ["mojang", "system"],
            verification_requested: verify,
            runtimes: orbit_launcher_core::list_managed_java_runtimes(runtime.paths(), verify)?
                .into_iter()
                .map(Into::into)
                .chain(
                    orbit_launcher_core::discover_system_java()
                        .into_iter()
                        .map(Into::into),
                )
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
        |event| frontend.launch_preparation(event),
    )?;
    if dry_run {
        return Ok(CommandOutput::LaunchPlan(plan.summary().into()));
    }
    let result = run_launch(plan, |event| frontend.launch_process(event)).await?;
    Ok(CommandOutput::LaunchResult(result.into()))
}

async fn execute_account(
    command: AccountCommands,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    let secrets = native_secret_store(runtime.paths())?;
    let backend = secrets.backend_name().to_string();
    match command {
        AccountCommands::Login { command } => {
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
            let repository = AccountRepository::load(runtime.paths())?;
            let account = repository.get(&account)?.clone();
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
        AccountCommands::Default { command } => {
            let mut repository = AccountRepository::load(runtime.paths())?;
            match command {
                AccountDefaultCommands::Show => {}
                AccountDefaultCommands::Set { account } => {
                    let id = repository.get(&account)?.id;
                    repository.set_default(Some(id))?;
                }
                AccountDefaultCommands::Clear => repository.set_default(None)?,
            }
            Ok(CommandOutput::AccountSelection(account_selection_view(
                runtime,
                &repository,
                None,
                "global",
            )))
        }
        AccountCommands::Logout { account } => {
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

fn account_selection_view(
    runtime: &RuntimeContext,
    repository: &AccountRepository,
    explicit: Option<uuid::Uuid>,
    scope: &'static str,
) -> AccountSelectionView {
    AccountSelectionView {
        scope,
        selection: repository.selection_summary(runtime.paths(), explicit),
    }
}

struct InstallCommandRequest {
    new_name: Option<String>,
    server_directory: Option<std::path::PathBuf>,
    kind: Option<crate::cli::InstanceKindArg>,
    minecraft: Option<String>,
    loader: Option<crate::cli::LoaderKindArg>,
    loader_version: Option<String>,
    from: Option<PathBuf>,
}

async fn execute_install(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
    request: InstallCommandRequest,
) -> Result<CommandOutput, AppError> {
    if request.from.is_some() && request.new_name.is_none() {
        return Err(AppError::Argument(
            "install --from requires install --new and cannot restore into an existing instance"
                .to_string(),
        ));
    }
    let created = if let Some(name) = request.new_name {
        Some(orbit_launcher_core::prepare_new_install(
            runtime,
            current_dir,
            orbit_launcher_core::NewInstallRequest {
                name,
                server_directory: request.server_directory,
                kind: request.kind.map(Into::into),
                minecraft: request.minecraft,
                loader: request.loader.map(Into::into),
                loader_version: request.loader_version,
                from: request.from,
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
    let created_selector = created
        .as_ref()
        .map(|created| created.created.entry.id.to_string());
    let selector = created_selector.as_deref().or(selector);
    let result = execute_existing_install(
        selector,
        current_dir,
        runtime,
        frontend,
        created
            .as_ref()
            .and_then(|prepared| prepared.state_archive.as_deref()),
    )
    .await;
    if result.is_err()
        && let Some(created) = created
    {
        created.rollback(runtime.paths()).map_err(|error| {
            LauncherError::Transaction(format!(
                "bootstrap failed and its provisional instance could not be rolled back: {error}"
            ))
        })?;
    }
    result
}

async fn execute_existing_install(
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
    state_archive: Option<&Path>,
) -> Result<CommandOutput, AppError> {
    let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
    let resolved = resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
    let package = state_archive.map(inspect_install_pack).transpose()?;
    if let Some(package) = &package
        && !package.targets.contains(&resolved.manifest.kind)
    {
        return Err(AppError::Argument(format!(
            "package cannot install into a {} instance",
            resolved.manifest.kind.as_str()
        )));
    }
    let client = runtime.config().download_client()?;
    let plan = prepare_install(
        &resolved.entry.location,
        &client,
        &runtime.config().java.selection,
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
    let state = state_archive
        .filter(|_| {
            package
                .as_ref()
                .is_some_and(|package| package.launcher_state)
        })
        .map(|source| {
            restore_launcher_state(resolved.entry.instance_directory(), source, |event| {
                frontend.state_archive_progress(event)
            })
            .map(Into::into)
        })
        .transpose()?;
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
        source_pinned_artifacts: result.source_pinned_artifacts,
        state,
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
        ServerCommands::Start => start_detached_supervisor(&resolved, runtime, frontend).await,
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
) -> Result<orbit_launcher_core::LaunchPlan, AppError> {
    prepare_launch(
        &resolved.entry.location,
        runtime.paths(),
        runtime.config(),
        None,
        |event| frontend.launch_preparation(event),
    )
    .map_err(AppError::from)
}

async fn execute_foreground_supervisor(
    resolved: &ResolvedInstance,
    runtime: &RuntimeContext,
    frontend: &mut dyn Frontend,
) -> Result<CommandOutput, AppError> {
    let plan = prepare_server_plan(resolved, runtime, frontend)?;
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
        frontend.supervisor_event(event)
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
    let plan = prepare_server_plan(&resolved, runtime, frontend)?;
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
        frontend.supervisor_event(event);
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
    frontend: &mut dyn Frontend,
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
        .arg("--output-format")
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
            if let Some(pid) = response.state.child_pid {
                frontend.supervisor_event(SupervisorEvent::Spawned {
                    pid,
                    generation: response.state.generation,
                });
                return Ok(CommandOutput::ServerStart(ServerStartView {
                    state: response.state,
                    stdout_log,
                    stderr_log,
                }));
            }
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
            YggdrasilProviderCommands::Inspect {
                address,
                allow_insecure_http,
            } => {
                let client = runtime.config().http_client()?;
                Ok(CommandOutput::YggdrasilEndpoint(
                    orbit_launcher_core::inspect_yggdrasil_provider(
                        &client,
                        &runtime.config().yggdrasil.providers,
                        &address,
                        allow_insecure_http,
                    )
                    .await?
                    .into(),
                ))
            }
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
                let endpoint = orbit_launcher_core::inspect_yggdrasil_provider(
                    &client,
                    &runtime.config().yggdrasil.providers,
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
                                api_root: endpoint.api_root,
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
        InstanceCommands::Inspect { directory } => Ok(CommandOutput::InstanceSources(
            orbit_launcher_core::inspect_external_instances(&resolve_directory(
                current_dir,
                Some(&directory),
            )?)?,
        )),
        InstanceCommands::Import { .. } | InstanceCommands::Delete { .. } => {
            unreachable!("async instance operations are dispatched by execute")
        }
        InstanceCommands::List => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let accounts = AccountRepository::load(runtime.paths())?;
            let instances = registry
                .instances
                .iter()
                .map(|entry| {
                    let resolved = resolve_instance(
                        &registry,
                        Some(&entry.id.to_string()),
                        current_dir,
                        ContextIntent::ReadOnly,
                    )?;
                    let installed =
                        orbit_launcher_core::LockFile::open_optional(entry.instance_directory())?;
                    Ok(InstanceView::from_snapshot(
                        entry,
                        &resolved.manifest,
                        installed.as_ref().map(|f| &f.inner),
                        registry.default_instance,
                        accounts.instance_selection_summary(runtime.paths(), &resolved.manifest),
                    ))
                })
                .collect::<Result<Vec<_>, LauncherError>>()?;
            Ok(CommandOutput::InstanceList(InstanceListView { instances }))
        }
        InstanceCommands::LaunchReport => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::ReadOnly)?;
            Ok(CommandOutput::LaunchReport(
                orbit_launcher_core::launch_record::latest_launch(
                    resolved.entry.instance_directory(),
                )?,
            ))
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
                AccountRepository::load(runtime.paths())?
                    .instance_selection_summary(runtime.paths(), &resolved.manifest),
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
        } => {
            if minecraft.is_none() && loader.is_none() && loader_version.is_none() {
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
                AccountRepository::load(runtime.paths())?
                    .instance_selection_summary(runtime.paths(), &configured.manifest),
            )))
        }
        InstanceCommands::Memory { command } => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let memory = match command {
                crate::cli::MemoryCommands::Show => None,
                crate::cli::MemoryCommands::Clear => {
                    Some(orbit_launcher_core::instance::MemoryConfig::default())
                }
                crate::cli::MemoryCommands::Set { min_mib, max_mib } => {
                    Some(orbit_launcher_core::instance::MemoryConfig { min_mib, max_mib })
                }
            };
            let selected_memory = memory.unwrap_or(resolved.manifest.launch.memory);
            let capabilities =
                orbit_launcher_core::platform::memory::MemoryCapabilities::native(selected_memory)?;
            memory
                .map(|memory| {
                    orbit_launcher_core::operations::set_instance_memory(
                        runtime.paths(),
                        &resolved.entry.id.to_string(),
                        memory,
                    )
                })
                .transpose()?;
            Ok(CommandOutput::InstanceMemory(
                crate::output::InstanceMemoryView {
                    instance_id: resolved.entry.id.to_string(),
                    name: resolved.entry.name,
                    memory: selected_memory,
                    capabilities,
                },
            ))
        }
        InstanceCommands::Resolution { command } => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let resolution = match command {
                ResolutionCommands::Set { width, height } => {
                    Some(orbit_launcher_core::ClientResolution { width, height })
                }
                ResolutionCommands::Clear => None,
            };
            let configured =
                set_client_resolution(runtime.paths(), &resolved.entry.id.to_string(), resolution)?;
            let installed = orbit_launcher_core::LockFile::open_optional(
                configured.entry.instance_directory(),
            )?;
            Ok(CommandOutput::InstanceResolution(InstanceDetailView::new(
                &configured.entry,
                &configured.manifest,
                installed.as_ref().map(|lock| &lock.inner),
                registry.default_instance,
                resolved.source,
                AccountRepository::load(runtime.paths())?
                    .instance_selection_summary(runtime.paths(), &configured.manifest),
            )))
        }
        InstanceCommands::Icon { command } => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let source = match &command {
                crate::cli::IconCommands::Set { image } => Some(image.as_path()),
                crate::cli::IconCommands::Clear => None,
            };
            let configured = orbit_launcher_core::presentation::set_instance_icon(
                runtime.paths(),
                &resolved.entry.id.to_string(),
                source,
            )?;
            let installed = orbit_launcher_core::LockFile::open_optional(
                configured.entry.instance_directory(),
            )?;
            Ok(CommandOutput::InstanceIcon(InstanceDetailView::new(
                &configured.entry,
                &configured.manifest,
                installed.as_ref().map(|f| &f.inner),
                registry.default_instance,
                resolved.source,
                AccountRepository::load(runtime.paths())?
                    .instance_selection_summary(runtime.paths(), &configured.manifest),
            )))
        }
        InstanceCommands::Java { command } => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let selection = match command {
                crate::cli::InstanceJavaCommands::Set { selection } => Some(selection.parse()?),
                crate::cli::InstanceJavaCommands::Clear => None,
            };
            let configured = orbit_launcher_core::operations::set_instance_java(
                runtime.paths(),
                &resolved.entry.id.to_string(),
                selection,
            )?;
            let installed = orbit_launcher_core::LockFile::open_optional(
                configured.entry.instance_directory(),
            )?;
            Ok(CommandOutput::InstanceJava(InstanceDetailView::new(
                &configured.entry,
                &configured.manifest,
                installed.as_ref().map(|f| &f.inner),
                registry.default_instance,
                resolved.source,
                AccountRepository::load(runtime.paths())?
                    .instance_selection_summary(runtime.paths(), &configured.manifest),
            )))
        }
        InstanceCommands::Account { command } => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let intent = if matches!(command, InstanceAccountCommands::Show) {
                ContextIntent::ReadOnly
            } else {
                ContextIntent::Sensitive
            };
            let resolved = resolve_instance(&registry, selector, current_dir, intent)?;
            let accounts = AccountRepository::load(runtime.paths())?;
            let manifest = match command {
                InstanceAccountCommands::Show => resolved.manifest,
                InstanceAccountCommands::Set { account } => {
                    let id = accounts.get(&account)?.id;
                    orbit_launcher_core::set_instance_account(
                        runtime.paths(),
                        &resolved.entry.id.to_string(),
                        Some(id),
                    )?
                    .manifest
                }
                InstanceAccountCommands::Clear => {
                    orbit_launcher_core::set_instance_account(
                        runtime.paths(),
                        &resolved.entry.id.to_string(),
                        None,
                    )?
                    .manifest
                }
            };
            if manifest.kind != InstanceKind::Client {
                return Err(LauncherError::InvalidManifest(
                    "server instances do not use client accounts".into(),
                )
                .into());
            }
            Ok(CommandOutput::AccountSelection(account_selection_view(
                runtime,
                &accounts,
                manifest.launch.account,
                "instance",
            )))
        }
        InstanceCommands::Unregister => {
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
    use orbit_launcher_core::{CreateInstanceRequest, create_instance};

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

        fn launch_preparation(&mut self, _event: LaunchPreparationEvent) {}

        fn launch_process(&mut self, _event: LaunchProcessEvent) {}

        fn supervisor_event(&mut self, _event: SupervisorEvent) {}

        fn repository_move(&mut self, _event: RepositoryMoveEvent) {}

        fn state_archive_progress(&mut self, _event: StateArchiveProgressEvent) {}
    }

    struct Fixture {
        instance: InstanceView,
    }
    fn create_fixture(
        request: CreateInstanceRequest,
        runtime: &RuntimeContext,
    ) -> Result<Fixture, AppError> {
        let created = create_instance(runtime.paths(), request)?;
        Ok(Fixture {
            instance: InstanceView::from_entry(&created.entry, None),
        })
    }

    fn runtime(directory: &Path) -> RuntimeContext {
        RuntimeContext::load(RuntimePathOptions {
            config_dir: Some(directory.join("config")),
            data_dir: Some(directory.join("data")),
            cache_dir: Some(directory.join("cache")),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn default_and_instance_account_commands_are_separate_and_inherit_after_clear() {
        use orbit_launcher_core::account::{AccountSelectionSource, AccountSelectionSummary};
        let temp = tempfile::tempdir().unwrap();
        let runtime = runtime(temp.path());
        let fallback = create_offline_account(runtime.paths(), "Fallback").unwrap();
        let explicit = create_offline_account(runtime.paths(), "Specific").unwrap();
        let fixture = create_fixture(
            CreateInstanceRequest {
                directory: runtime.minecraft_directory(),
                name: "test".into(),
                kind: InstanceKind::Client,
                minecraft_requirement: "1.21.1".into(),
                loader_kind: orbit_launcher_core::LoaderKind::Vanilla,
                loader_requirement: None,
            },
            &runtime,
        )
        .unwrap();
        let selected = execute_instance(
            InstanceCommands::Account {
                command: InstanceAccountCommands::Set {
                    account: explicit.id.to_string(),
                },
            },
            Some(&fixture.instance.id),
            temp.path(),
            &runtime,
        )
        .unwrap();
        let CommandOutput::AccountSelection(selected) = selected else {
            panic!("wrong output")
        };
        assert!(
            matches!(selected.selection, AccountSelectionSummary::Selected { account, source: AccountSelectionSource::Instance } if account.id == explicit.id)
        );
        let cleared = execute_instance(
            InstanceCommands::Account {
                command: InstanceAccountCommands::Clear,
            },
            Some(&fixture.instance.id),
            temp.path(),
            &runtime,
        )
        .unwrap();
        let CommandOutput::AccountSelection(cleared) = cleared else {
            panic!("wrong output")
        };
        assert!(
            matches!(cleared.selection, AccountSelectionSummary::Selected { account, source: AccountSelectionSource::Global } if account.id == fallback.id)
        );
        assert_eq!(
            AccountRepository::load(runtime.paths())
                .unwrap()
                .default_account(),
            Some(fallback.id)
        );
        let mut frontend = NoopFrontend;
        let error = execute(
            Commands::Account {
                command: AccountCommands::Default {
                    command: AccountDefaultCommands::Clear,
                },
            },
            Some(&fixture.instance.id),
            temp.path(),
            &runtime,
            &mut frontend,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), "argument");
        let CommandOutput::AccountSelection(empty) = execute(
            Commands::Account {
                command: AccountCommands::Default {
                    command: AccountDefaultCommands::Clear,
                },
            },
            None,
            temp.path(),
            &runtime,
            &mut frontend,
        )
        .await
        .unwrap() else {
            panic!("wrong output")
        };
        assert!(matches!(
            empty.selection,
            AccountSelectionSummary::NotConfigured
        ));
    }

    #[test]
    fn memory_management_uses_the_same_instance_api_for_clients_and_servers() {
        for kind in [InstanceKindArg::Client, InstanceKindArg::Server] {
            let directory = tempfile::tempdir().unwrap();
            let game = directory.path().join("game");
            std::fs::create_dir_all(&game).unwrap();
            let game = dunce::canonicalize(game).unwrap();
            let runtime = runtime(directory.path());
            let created = create_fixture(
                CreateInstanceRequest {
                    name: "memory-test".into(),
                    kind: kind.into(),
                    directory: if kind == InstanceKindArg::Client {
                        runtime.minecraft_directory()
                    } else {
                        game.clone()
                    },
                    minecraft_requirement: "26.2".into(),
                    loader_kind: LoaderKindArg::Vanilla.into(),
                    loader_requirement: None,
                },
                &runtime,
            )
            .unwrap();
            let selector = Some(created.instance.id.as_str());
            let output = execute_instance(
                InstanceCommands::Memory {
                    command: crate::cli::MemoryCommands::Set {
                        min_mib: 1024,
                        max_mib: 8192,
                    },
                },
                selector,
                &game,
                &runtime,
            )
            .unwrap();
            let CommandOutput::InstanceMemory(memory) = output else {
                panic!("unexpected memory output")
            };
            assert_eq!(memory.memory.max_mib, 8192);
            assert!(memory.capabilities.physical_memory_mib > 0);
            let path = created
                .instance
                .directory
                .join(orbit_launcher_core::instance::INSTANCE_MANIFEST_FILE);
            let original = std::fs::read(&path).unwrap();
            assert!(
                execute_instance(
                    InstanceCommands::Memory {
                        command: crate::cli::MemoryCommands::Set {
                            min_mib: 0,
                            max_mib: 8192
                        }
                    },
                    selector,
                    &game,
                    &runtime
                )
                .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), original);
            let CommandOutput::InstanceMemory(shown) = execute_instance(
                InstanceCommands::Memory {
                    command: crate::cli::MemoryCommands::Show,
                },
                selector,
                &game,
                &runtime,
            )
            .unwrap() else {
                panic!("unexpected show output")
            };
            assert_eq!(shown.memory, memory.memory);
            let CommandOutput::InstanceMemory(cleared) = execute_instance(
                InstanceCommands::Memory {
                    command: crate::cli::MemoryCommands::Clear,
                },
                selector,
                &game,
                &runtime,
            )
            .unwrap() else {
                panic!("unexpected clear output")
            };
            assert_eq!(
                cleared.memory,
                orbit_launcher_core::instance::MemoryConfig::default()
            );
        }
    }

    #[test]
    fn instance_inventory_exposes_the_effective_account_without_authenticating() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = runtime(directory.path());
        let default = create_offline_account(runtime.paths(), "DefaultPlayer").unwrap();
        let explicit = create_offline_account(runtime.paths(), "InstancePlayer").unwrap();
        for (name, kind, account, expected_source, expected_account) in [
            (
                "inherited",
                InstanceKindArg::Client,
                None,
                "global",
                default.id,
            ),
            (
                "specific",
                InstanceKindArg::Client,
                Some(explicit.id),
                "instance",
                explicit.id,
            ),
            ("server", InstanceKindArg::Server, None, "", default.id),
        ] {
            let game = directory.path().join(name);
            std::fs::create_dir_all(&game).unwrap();
            let game = dunce::canonicalize(game).unwrap();
            let created = create_fixture(
                CreateInstanceRequest {
                    name: name.into(),
                    kind: kind.into(),
                    directory: if kind == InstanceKindArg::Client {
                        runtime.minecraft_directory()
                    } else {
                        game.clone()
                    },
                    minecraft_requirement: "26.2".into(),
                    loader_kind: LoaderKindArg::Vanilla.into(),
                    loader_requirement: None,
                },
                &runtime,
            )
            .unwrap();
            if let Some(account) = account {
                orbit_launcher_core::set_instance_account(
                    runtime.paths(),
                    &created.instance.id,
                    Some(account),
                )
                .unwrap();
            }
            let CommandOutput::InstanceList(list) =
                execute_instance(InstanceCommands::List, None, &game, &runtime).unwrap()
            else {
                panic!("unexpected list output");
            };
            let listed = list
                .instances
                .iter()
                .find(|instance| instance.id == created.instance.id)
                .unwrap();
            let CommandOutput::InstanceDetail(shown) = execute_instance(
                InstanceCommands::Show,
                Some(&created.instance.id),
                &game,
                &runtime,
            )
            .unwrap() else {
                panic!("unexpected show output");
            };
            let list_json = serde_json::to_value(listed).unwrap();
            let show_json = serde_json::to_value(shown).unwrap();
            assert_eq!(list_json["launch_account"], show_json["launch_account"]);
            if name == "server" {
                assert!(list_json["launch_account"].is_null());
            } else {
                assert_eq!(list_json["launch_account"]["source"], expected_source);
                assert_eq!(
                    list_json["launch_account"]["account"]["id"],
                    expected_account.to_string()
                );
                assert_eq!(
                    list_json["launch_account"]["account"]["provider"]["kind"],
                    "offline"
                );
                assert!(!list_json.to_string().contains("token"));
            }
        }
    }
    #[test]
    fn current_directory_and_explicit_global_id_resolve_the_same_instance() {
        let directory = tempfile::tempdir().unwrap();
        let instance_root = directory.path().join("instance");
        std::fs::create_dir_all(&instance_root).unwrap();
        let instance_root = dunce::canonicalize(instance_root).unwrap();
        let runtime = runtime(directory.path());

        let created = create_fixture(
            CreateInstanceRequest {
                name: "server".to_string(),
                kind: InstanceKindArg::Server.into(),
                directory: if InstanceKindArg::Server == InstanceKindArg::Client {
                    runtime.minecraft_directory()
                } else {
                    instance_root.clone()
                },
                minecraft_requirement: "1.21.1".to_string(),
                loader_kind: LoaderKindArg::Fabric.into(),
                loader_requirement: Some("stable".to_string()),
            },
            &runtime,
        )
        .unwrap();

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
        create_fixture(
            CreateInstanceRequest {
                name: "client".to_string(),
                kind: InstanceKindArg::Client.into(),
                directory: if InstanceKindArg::Client == InstanceKindArg::Client {
                    runtime.minecraft_directory()
                } else {
                    instance_root.clone()
                },
                minecraft_requirement: "latest-release".to_string(),
                loader_kind: LoaderKindArg::Vanilla.into(),
                loader_requirement: None,
            },
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
