use std::path::Path;

use orbit_launcher_core::{
    AccountRepository, ConfigKey, ContextIntent, CreateInstanceRequest, EulaAcceptanceMethod,
    EulaDocument, ExternalYggdrasilLoginRequest, InstallProgressEvent, InstanceKind,
    InstanceRegistry, LaunchPreparationEvent, LaunchProcessEvent, LauncherError, ManifestFile,
    MicrosoftLoginProgressEvent, RuntimeContext, ServerInstallPlan, YggdrasilProviderConfig,
    accept_shown_eula, add_yggdrasil_provider, apply_install_plan, begin_microsoft_device_login,
    complete_microsoft_device_login, create_instance, create_offline_account, get_config,
    import_instance, list_config, login_external_yggdrasil, native_secret_store, prepare_install,
    prepare_launch, remove_instance, remove_yggdrasil_provider, rename_instance, resolve_instance,
    resolve_instance_root, resolve_launch_identity, rollback_created_instance, run_launch,
    set_config, set_default_instance, show_current_eula, unset_config,
};
use zeroize::Zeroizing;

use crate::cli::{
    AccountCommands, AccountLoginCommands, Commands, ConfigCommands, DefaultCommands, EulaCommands,
    InstanceCommands, MicrosoftLoginCommands, ServerCommands, YggdrasilProviderCommands,
};
use crate::output::{
    AccountListView, AccountLoginView, AccountLogoutView, AccountSelectionView, AccountView,
    ConfigEntryView, ConfigListView, ConfigMutationAction, ConfigMutationView, ConfigPathView,
    DefaultView, EulaAcceptanceView, EulaDocumentView, InstallView, InstanceDetailView,
    InstanceListView, InstanceMutationAction, InstanceMutationView, InstanceView, LaunchPlanView,
    LaunchResultView, MicrosoftDeviceSessionView, RenameView, YggdrasilProviderListView,
    YggdrasilProviderMutationView, YggdrasilProviderView,
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
    InstanceList(InstanceListView),
    InstanceDetail(InstanceDetailView),
    InstanceMutation(InstanceMutationView),
    Rename(RenameView),
    Default(DefaultView),
    AccountList(AccountListView),
    AccountDetail(AccountView),
    AccountLogin(AccountLoginView),
    AccountSelection(AccountSelectionView),
    AccountLogout(AccountLogoutView),
    MicrosoftDeviceSession(MicrosoftDeviceSessionView),
    YggdrasilProviderList(YggdrasilProviderListView),
    YggdrasilProviderMutation(YggdrasilProviderMutationView),
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
            Self::InstanceList(_) => "instance.list",
            Self::InstanceDetail(_) => "instance.show",
            Self::InstanceMutation(view) => view.action.command_name(),
            Self::Rename(_) => "instance.rename",
            Self::Default(_) => "instance.default",
            Self::AccountList(_) => "account.list",
            Self::AccountDetail(_) => "account.show",
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
        }
    }

    pub const fn process_succeeded(&self) -> bool {
        match self {
            Self::LaunchResult(view) => view.success,
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
            root,
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
                    root,
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
            execute_config(command, runtime)
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
    let plan = prepare_launch(&resolved.entry.root, runtime.paths(), identity, |event| {
        frontend.launch_preparation(command_name, event)
    })?;
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
                            runtime.config(),
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
                account: AccountView::new(&account, repository.default_account(), &backend),
            }))
        }
        AccountCommands::List => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for account list".to_string(),
                ));
            }
            let repository = AccountRepository::load(runtime.paths())?;
            Ok(CommandOutput::AccountList(AccountListView {
                accounts: repository
                    .accounts()
                    .iter()
                    .map(|account| {
                        AccountView::new(account, repository.default_account(), &backend)
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
            };
            Ok(CommandOutput::AccountDetail(AccountView::new(
                account,
                repository.default_account(),
                &backend,
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
            let view = AccountView::new(&account, repository.default_account(), &backend);
            repository.remove(account.id, secrets.as_ref()).await?;
            Ok(CommandOutput::AccountLogout(AccountLogoutView {
                account: view,
                local_secret_deleted: true,
            }))
        }
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
    let mut manifest = ManifestFile::open(&resolved.entry.root)?;
    manifest.inner.launch.account = account;
    manifest.save()?;
    Ok(())
}

struct InstallCommandRequest {
    new_name: Option<String>,
    root: Option<std::path::PathBuf>,
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
        let kind = request
            .kind
            .ok_or_else(|| AppError::Argument("install --new requires --kind".to_string()))?;
        let minecraft = request
            .minecraft
            .ok_or_else(|| AppError::Argument("install --new requires --minecraft".to_string()))?;
        let root = resolve_instance_root(current_dir, request.root.as_deref())?;
        Some(create_instance(
            runtime.paths(),
            CreateInstanceRequest {
                root,
                name,
                kind: kind.into(),
                minecraft_requirement: minecraft,
                loader_kind: request
                    .loader
                    .unwrap_or(crate::cli::LoaderKindArg::Vanilla)
                    .into(),
                loader_requirement: request.loader_version,
            },
        )?)
    } else {
        if request.root.is_some()
            || request.kind.is_some()
            || request.minecraft.is_some()
            || request.loader.is_some()
            || request.loader_version.is_some()
        {
            return Err(AppError::Argument(
                "--root, --kind, --minecraft, --loader, and --loader-version require install --new"
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
        &resolved.entry.root,
        &client,
        runtime.config().java.default_provider,
        |event| frontend.progress(event),
    )
    .await?;
    if let Some(server) = plan.server() {
        ensure_server_eula_accepted(&resolved.entry.root, server, frontend)?;
    }
    let result = apply_install_plan(
        &resolved.entry.root,
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
    let registry = InstanceRegistry::load(&runtime.paths().instances_file())?;
    let resolved = resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
    if resolved.manifest.kind != InstanceKind::Server {
        return Err(AppError::Argument(format!(
            "instance '{}' is a client; server commands require a server instance",
            resolved.entry.name
        )));
    }
    match command {
        ServerCommands::Run { dry_run } => {
            execute_launch(
                selector,
                current_dir,
                runtime,
                frontend,
                InstanceKind::Server,
                dry_run,
            )
            .await
        }
        ServerCommands::Eula { command } => match command {
            EulaCommands::Show => {
                let client = runtime.config().http_client()?;
                let document = show_current_eula(&resolved.entry.root, &client).await?;
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
                    &resolved.entry.root,
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

fn execute_config(
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
            } => Ok(CommandOutput::YggdrasilProviderMutation(
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
            )),
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
            root,
            kind,
            minecraft,
            loader,
            loader_version,
        } => {
            let root = resolve_instance_root(current_dir, root.as_deref())?;
            let created = create_instance(
                runtime.paths(),
                CreateInstanceRequest {
                    root,
                    name,
                    kind: kind.into(),
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
        InstanceCommands::Import { root } => {
            let root = resolve_instance_root(current_dir, root.as_deref())?;
            let imported = import_instance(runtime.paths(), &root)?;
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
            Ok(CommandOutput::InstanceDetail(InstanceDetailView::new(
                &resolved.entry,
                &resolved.manifest,
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
                root: None,
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
                root: None,
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
