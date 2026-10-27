use super::commands::{self, CommandHandler};
use super::*;
use anyhow::Result;

impl CommandHandler for Commands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::*;
        let mutates_instance = self.mutates_instance();
        self.validate_invocation(ctx.instance.is_some(), ctx.dry_run)?;
        if mutates_instance && !matches!(&self, Self::Init { .. }) {
            ctx.require_explicit_mutation_target()?;
        }
        let _workspace_locks = if mutates_instance {
            let source = if matches!(&self, Self::Init { .. }) {
                std::env::current_dir()?
            } else {
                ctx.instance_dir()?
            };
            let mut targets = vec![source];
            if let Commands::Migrate {
                command: MigrateCommands::Export { target, .. },
            } = &self
            {
                targets.push(target.clone());
            }
            Some(orbit_core::WorkspaceMutationLocks::acquire(targets)?)
        } else {
            None
        };
        match self {
            Commands::Update {
                command: UpdateCommands::Check { current_version },
            } => commands::update::handle(current_version, ctx).await,
            Commands::Init {
                name,
                mc_version,
                modloader,
                modloader_version,
            } => handle_init(name, mc_version, modloader, modloader_version, ctx).await,
            Commands::Instance { command } => command.execute(ctx).await,
            Commands::Install {
                target,
                group,
                no_optional,
            } => handle_install(target, group, no_optional, ctx).await,
            Commands::Fix => handle_fix(ctx).await,
            Commands::Add {
                mod_name,
                platform,
                version,
                string,
                env,
                optional,
            } => handle_add(mod_name, platform, version, string, env, optional, ctx).await,
            Commands::Package { package, command } => command.execute(package, ctx).await,
            Commands::Remove { mod_name } => handle_remove(mod_name, ctx).await,
            Commands::Launch {
                launcher,
                launcher_instance,
                server,
                runtime_agent,
            } => handle_launch(launcher, runtime_agent, launcher_instance, server, ctx).await,
            Commands::Sync => handle_sync(ctx).await,
            Commands::Outdated { mod_name } => handle_outdated(mod_name, ctx).await,
            Commands::Upgrade { mod_name } => handle_upgrade(mod_name, ctx).await,
            Commands::Search {
                query,
                platform,
                limit,
                mc_version,
                modloader,
            } => handle_search(query, platform, limit, mc_version, modloader, ctx).await,
            Commands::Info { mod_name, platform } => handle_info(mod_name, platform, ctx).await,
            Commands::List { tree, target } => handle_list(tree, target, ctx).await,
            Commands::Import {
                file,
                merge_strategy,
                optional_files,
                all_optional,
            } => handle_import(file, merge_strategy, optional_files, all_optional, ctx).await,
            Commands::Export {
                file,
                target,
                archive_format,
                content,
            } => handle_export(file, target, archive_format, content, ctx).await,
            Commands::Migrate { command } => command.execute(ctx).await,
            Commands::Capabilities => handle_capabilities(ctx).await,
            Commands::Audit {
                min_risk,
                fail_on_risk,
                mod_filter,
                report,
                limit,
            } => handle_audit(min_risk, fail_on_risk, mod_filter, report, limit, ctx).await,
            Commands::Cache { command } => command.execute(ctx).await,
            Commands::Config { scope, command } => handle_config(scope, command, ctx).await,
        }
    }
}

impl Commands {
    pub fn validate_invocation(&self, has_instance: bool, dry_run: bool) -> Result<()> {
        if has_instance
            && matches!(
                self,
                Self::Init { .. }
                    | Self::Instance { .. }
                    | Self::Cache { .. }
                    | Self::Update { .. }
                    | Self::Config {
                        scope: ConfigScope::Global,
                        ..
                    }
            )
        {
            anyhow::bail!("{}", tr!("This command does not accept --instance"));
        }
        if dry_run
            && matches!(
                self,
                Self::Instance {
                    command: InstanceCommands::Register { .. }
                        | InstanceCommands::Unregister { .. }
                }
            )
        {
            anyhow::bail!(
                "{}",
                tr!("Instance registration commands do not support --dry-run")
            );
        }
        Ok(())
    }

    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Update { .. } => "update.check",
            Self::Init { .. } => "init",
            Self::Install { .. } => "install",
            Self::Fix => "fix",
            Self::Add { .. } => "add",
            Self::Remove { .. } => "remove",
            Self::Launch { .. } => "launch",
            Self::Sync => "sync",
            Self::Outdated { .. } => "outdated",
            Self::Upgrade { .. } => "upgrade",
            Self::Search { .. } => "search",
            Self::Info { .. } => "info",
            Self::List { .. } => "list",
            Self::Import { .. } => "import",
            Self::Export { .. } => "export",
            Self::Migrate { command } => match command {
                MigrateCommands::Check { .. } => "migrate.check",
                MigrateCommands::Export { .. } => "migrate.export",
            },
            Self::Capabilities => "capabilities",
            Self::Audit { .. } => "audit",
            Self::Cache {
                command: CacheCommands::Clean,
            } => "cache.clean",
            Self::Config { command, .. } => match command {
                ConfigCommands::Path => "config.path",
                ConfigCommands::List => "config.list",
                ConfigCommands::Get { .. } => "config.get",
                ConfigCommands::Set { .. } => "config.set",
                ConfigCommands::Unset { .. } => "config.unset",
            },
            Self::Instance { command } => match command {
                InstanceCommands::List => "instance.list",
                InstanceCommands::Register { .. } => "instance.register",
                InstanceCommands::Unregister { .. } => "instance.unregister",
            },
            Self::Package { command, .. } => command.command_name(),
        }
    }
    pub fn mutates_instance(&self) -> bool {
        match self {
            Self::Package { command, .. } => command.mutates_instance(),
            Self::Init { .. }
            | Self::Install { .. }
            | Self::Fix
            | Self::Add { .. }
            | Self::Remove { .. }
            | Self::Launch { .. }
            | Self::Sync
            | Self::Upgrade { .. }
            | Self::Import { .. } => true,
            Self::Config { scope, command } => {
                *scope == ConfigScope::Instance
                    && matches!(
                        command,
                        ConfigCommands::Set { .. } | ConfigCommands::Unset { .. }
                    )
            }
            Self::Migrate { command } => matches!(command, MigrateCommands::Export { .. }),
            Self::Instance { .. }
            | Self::Update { .. }
            | Self::Outdated { .. }
            | Self::Search { .. }
            | Self::Info { .. }
            | Self::List { .. }
            | Self::Export { .. }
            | Self::Capabilities
            | Self::Audit { .. }
            | Self::Cache { .. } => false,
        }
    }
}

impl CommandHandler for InstanceCommands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::instances::*;
        match self {
            InstanceCommands::List => handle_list(ctx).await,
            InstanceCommands::Register { name, path } => handle_register(name, path, ctx).await,
            InstanceCommands::Unregister { name } => handle_remove(name, ctx).await,
        }
    }
}

impl CommandHandler for CacheCommands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::cache::clean;
        match self {
            CacheCommands::Clean => clean::handle(ctx).await,
        }
    }
}

impl CommandHandler for MigrateCommands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        match self {
            Self::Check {
                target,
                source_pack,
                allow_removals,
            } => commands::migrate::handle_check(target, source_pack, allow_removals, ctx).await,
            Self::Export {
                target,
                source_pack,
                consume_source_pack,
                allow_removals,
            } => {
                commands::migrate::handle_export(
                    target,
                    source_pack,
                    consume_source_pack,
                    allow_removals,
                    ctx,
                )
                .await
            }
        }
    }
}

impl PackageCommands {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Constraint {
                command: ConstraintCommands::Show,
            } => "package.constraint.show",
            Self::Constraint {
                command: ConstraintCommands::Set { .. },
            } => "package.constraint.set",
            Self::Versions => "package.versions",
            Self::Files => "package.files",
            Self::Env { .. } => "package.env",
            Self::Enable => "package.enable",
            Self::Disable => "package.disable",
            Self::Reset => "package.reset",
            Self::Purge => "package.purge",
            Self::Remote { command } => match command {
                RemoteCommands::Add { .. } => "package.remote.add",
                RemoteCommands::Remove { .. } => "package.remote.remove",
                RemoteCommands::List => "package.remote.list",
            },
        }
    }
    pub fn mutates_instance(&self) -> bool {
        !matches!(
            self,
            Self::Versions
                | Self::Files
                | Self::Constraint {
                    command: ConstraintCommands::Show
                }
                | Self::Remote {
                    command: RemoteCommands::List
                }
        )
    }
    async fn execute(self, package: String, ctx: &commands::CliContext) -> Result<()> {
        use super::commands::*;
        match self {
            Self::Constraint { command } => handle_constraint(package, command, ctx).await,
            Self::Versions => handle_versions(package, ctx).await,
            Self::Files => handle_ownership(package, ctx),
            Self::Env { environment } => handle_env(package, environment, ctx),
            Self::Enable => handle_activation(package, true, ctx),
            Self::Disable => handle_activation(package, false, ctx),
            Self::Reset => handle_reset(package, ctx).await,
            Self::Purge => handle_purge(package, ctx).await,
            Self::Remote { command } => handle_remote(package, command, ctx).await,
        }
    }
}
