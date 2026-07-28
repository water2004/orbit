pub mod commands;
pub mod output;
mod progress;
use crate::cli::commands::CommandHandler;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use orbit_i18n::LanguageMode;

pub use output::{OutputFormat, ProgressFormat};

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "The Modern, Non-intrusive Package Manager for Minecraft Mods.", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Presentation language: system / en / zh-CN (system by default).
    #[arg(long, global = true, default_value_t = LanguageMode::System)]
    pub language: LanguageMode,

    /// Select an instance by name.
    #[arg(short = 'i', long, global = true)]
    pub instance: Option<String>,

    /// Exact global configuration file path.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Exact global JAR cache directory.
    #[arg(long, global = true)]
    pub cache_dir: Option<PathBuf>,

    /// Default path layout: system / executable.
    #[arg(long, global = true)]
    pub data_layout: Option<orbit_core::PathLayout>,

    /// Output format: text / json.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Progress protocol: none / ndjson (stderr only).
    #[arg(long, global = true, value_enum, default_value_t = ProgressFormat::None)]
    pub progress_format: ProgressFormat,

    /// Show detailed logs.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Quiet mode; only print errors.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Skip write confirmation; package identity and resolution choices still require a decision.
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

    /// Simulate the operation without changing files.
    #[arg(long, global = true)]
    pub dry_run: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize the current directory as an Orbit project.
    Init {
        /// Instance name.
        name: String,
        /// Minecraft version.
        #[arg(long)]
        mc_version: Option<String>,
        /// Mod loader.
        #[arg(long)]
        modloader: Option<String>,
        /// Loader version.
        #[arg(long)]
        modloader_version: Option<String>,
    },

    /// Manage instances.
    Instances {
        #[command(subcommand)]
        command: InstanceCommands,
    },

    /// Install the exact package realization recorded by orbit.lock.
    Install {
        /// Target environment: client / server / both (default).
        #[arg(long)]
        target: Option<String>,
        /// Install only the selected group.
        #[arg(long)]
        group: Option<String>,
        /// Skip optional dependencies.
        #[arg(long)]
        no_optional: bool,
    },

    /// Resolve and repair the package set declared by orbit.toml.
    Fix,

    /// Add a mod.
    Add {
        /// Mod name; supports mr:name, cf:name, and file:path prefixes.
        mod_name: String,
        /// Select a provider.
        #[arg(long)]
        platform: Option<String>,
        /// Version requirement.
        #[arg(long)]
        version: Option<String>,
        /// Environment filter: client / server / both.
        #[arg(long)]
        env: Option<String>,
        /// Mark as an optional dependency.
        #[arg(long)]
        optional: bool,
        /// Do not install transitive dependencies.
        #[arg(long)]
        no_deps: bool,
    },

    /// Set a root package environment filter; auto follows the selected JAR declaration.
    Env {
        /// mod_id declared by JAR metadata.
        package: String,
        /// client / server / both / auto.
        environment: String,
    },

    /// Remove a mod.
    Remove {
        /// Mod name.
        mod_name: String,
    },

    /// Remove a mod and its configuration files.
    Purge {
        /// Mod name.
        mod_name: String,
    },

    /// Reconcile local state in both directions.
    Sync,

    /// Check for outdated mods (read-only).
    Outdated {
        /// Optional mod name.
        mod_name: Option<String>,
    },

    /// Upgrade mods.
    Upgrade {
        /// Optional mod name; omit to upgrade all.
        mod_name: Option<String>,
    },

    /// Search for mods.
    Search {
        /// Search query.
        query: String,
        /// Select a provider.
        #[arg(long)]
        platform: Option<String>,
        /// Result limit.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Filter by Minecraft version.
        #[arg(long)]
        mc_version: Option<String>,
        /// Filter by mod loader (Fabric, Forge, Quilt, etc.).
        #[arg(long)]
        modloader: Option<String>,
    },

    /// Show mod details.
    Info {
        /// Mod name.
        mod_name: String,
        /// Select a provider.
        #[arg(long)]
        platform: Option<String>,
    },

    /// List installed mods.
    List {
        /// Show dependencies as a tree.
        #[arg(long)]
        tree: bool,
        /// Filter by environment.
        #[arg(long)]
        target: Option<String>,
    },

    /// Import an external mod manifest.
    Import {
        /// File path (.toml or .zip).
        file: String,
        /// Merge strategy.
        #[arg(long)]
        merge_strategy: Option<String>,
    },

    /// Export the current instance as an archive.
    Export {
        /// Output file path.
        file: Option<String>,
        /// Target environment filter.
        #[arg(long)]
        target: Option<String>,
        /// Export format: zip / mrpack.
        #[arg(long, default_value = "zip")]
        format: String,
    },

    /// Plan or export a package migration into an installed target runtime.
    Migrate {
        #[command(subcommand)]
        command: MigrateCommands,
    },

    /// Statically analyze bytecode compatibility risks in the current instance (read-only).
    Audit {
        /// Show only findings at or above this risk score (0-100).
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=100))]
        min_risk: u8,
        /// Return non-zero when a finding reaches this risk score (0-100).
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
        fail_on_risk: Option<u8>,
        /// Show only findings involving this mod (ID, file name, or display name).
        #[arg(long = "mod")]
        mod_filter: Option<String>,
        /// Write the complete untruncated structured report to a JSON file.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Maximum high-ranked findings shown in text mode.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Clean the global download cache.
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },

    /// Inspect or change global configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Manage candidate sources for a package.
    Remote {
        #[command(subcommand)]
        command: RemoteCommands,
    },
}

#[derive(Subcommand)]
pub enum InstanceCommands {
    /// List all managed Minecraft instances.
    List,
    /// Set an instance as the global default.
    Default { name: String },
    /// Stop tracking an instance.
    Remove { name: String },
}

#[derive(Subcommand)]
pub enum CacheCommands {
    /// Clean the download cache.
    Clean,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Show the exact global configuration file path in use.
    Path,
    /// List file-level values for every supported field.
    List,
    /// Read a configuration field.
    Get {
        /// Configuration key, for example cache.capacity-mib.
        key: String,
    },
    /// Set a configuration field after typed validation.
    Set {
        /// Configuration key, for example cache.capacity-mib.
        key: String,
        /// New value.
        value: String,
    },
    /// Clear an optional field or restore a required field to its default.
    Unset {
        /// Configuration key, for example network.proxy.
        key: String,
    },
}

#[derive(Subcommand)]
pub enum RemoteCommands {
    /// Validate and add a source.
    Add {
        package: String,
        /// file / modrinth / curseforge.
        provider: String,
        /// File path, Modrinth project ID, or numeric CurseForge project ID.
        locator: String,
    },
    /// Remove a source; the final source cannot be removed.
    Remove {
        package: String,
        /// file / modrinth / curseforge (omit when using --index)
        provider: Option<String>,
        /// Source locator (omit when using --index)
        locator: Option<String>,
        /// One-based index shown by `orbit remote list`
        #[arg(long, conflicts_with_all = ["provider", "locator"])]
        index: Option<usize>,
    },
    /// List every source for a package.
    List { package: String },
}

impl CommandHandler for Commands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::*;
        if self.mutates_instance() {
            ctx.require_explicit_mutation_target()?;
        }
        match self {
            Commands::Init {
                name,
                mc_version,
                modloader,
                modloader_version,
            } => handle_init(name, mc_version, modloader, modloader_version, ctx).await,
            Commands::Instances { command } => command.execute(ctx).await,
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
                env,
                optional,
                no_deps,
            } => handle_add(mod_name, platform, version, env, optional, no_deps, ctx).await,
            Commands::Env {
                package,
                environment,
            } => handle_env(package, environment, ctx),
            Commands::Remove { mod_name } => handle_remove(mod_name, ctx).await,
            Commands::Purge { mod_name } => handle_purge(mod_name, ctx).await,
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
            } => handle_import(file, merge_strategy, ctx).await,
            Commands::Export {
                file,
                target,
                format,
            } => handle_export(file, target, format, ctx).await,
            Commands::Migrate { command } => command.execute(ctx).await,
            Commands::Audit {
                min_risk,
                fail_on_risk,
                mod_filter,
                report,
                limit,
            } => handle_audit(min_risk, fail_on_risk, mod_filter, report, limit, ctx).await,
            Commands::Cache { command } => command.execute(ctx).await,
            Commands::Config { command } => handle_config(command, ctx).await,
            Commands::Remote { command } => handle_remote(command, ctx).await,
        }
    }
}

impl Commands {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Init { .. } => "init",
            Self::Instances { .. } => "instances",
            Self::Install { .. } => "install",
            Self::Fix => "fix",
            Self::Add { .. } => "add",
            Self::Env { .. } => "env",
            Self::Remove { .. } => "remove",
            Self::Purge { .. } => "purge",
            Self::Sync => "sync",
            Self::Outdated { .. } => "outdated",
            Self::Upgrade { .. } => "upgrade",
            Self::Search { .. } => "search",
            Self::Info { .. } => "info",
            Self::List { .. } => "list",
            Self::Import { .. } => "import",
            Self::Export { .. } => "export",
            Self::Migrate { .. } => "migrate",
            Self::Audit { .. } => "audit",
            Self::Cache { .. } => "cache",
            Self::Config { .. } => "config",
            Self::Remote { .. } => "remote",
        }
    }

    fn mutates_instance(&self) -> bool {
        matches!(
            self,
            Self::Install { .. }
                | Self::Fix
                | Self::Add { .. }
                | Self::Env { .. }
                | Self::Remove { .. }
                | Self::Purge { .. }
                | Self::Sync
                | Self::Upgrade { .. }
                | Self::Import { .. }
                | Self::Migrate {
                    command: MigrateCommands::Export { .. }
                }
                | Self::Remote {
                    command: RemoteCommands::Add { .. } | RemoteCommands::Remove { .. }
                }
        )
    }
}

#[derive(Subcommand)]
pub enum MigrateCommands {
    /// Resolve the complete package graph for an installed target runtime.
    Check {
        /// Exact target game-instance directory created by a launcher.
        target: PathBuf,
        /// Portable Orbit ZIP captured before the target instance was created.
        #[arg(long)]
        source_pack: Option<PathBuf>,
    },
    /// Write target orbit.toml, orbit.lock, and configuration into that runtime.
    Export {
        /// Exact target game-instance directory created by a launcher.
        target: PathBuf,
        /// Portable Orbit ZIP captured before the target instance was created.
        #[arg(long)]
        source_pack: Option<PathBuf>,
        /// Remove --source-pack after a successful, confirmed export.
        #[arg(long, requires = "source_pack")]
        consume_source_pack: bool,
    },
}

impl CommandHandler for InstanceCommands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::instances::*;
        match self {
            InstanceCommands::List => handle_list(ctx).await,
            InstanceCommands::Default { name } => handle_default(name, ctx).await,
            InstanceCommands::Remove { name } => handle_remove(name, ctx).await,
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
            } => commands::migrate::handle_check(target, source_pack, ctx).await,
            Self::Export {
                target,
                source_pack,
                consume_source_pack,
            } => {
                commands::migrate::handle_export(target, source_pack, consume_source_pack, ctx)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands, ConfigCommands, MigrateCommands, PathBuf, RemoteCommands};

    #[test]
    fn audit_defaults_do_not_request_a_report_file() {
        let cli = Cli::try_parse_from(["orbit", "audit"]).unwrap();
        let Commands::Audit { report, limit, .. } = cli.command else {
            panic!("audit command was not parsed");
        };

        assert!(report.is_none());
        assert_eq!(limit, 20);
    }

    #[test]
    fn classifies_instance_mutations_for_default_fallback_safety() {
        assert!(
            Commands::Install {
                target: None,
                group: None,
                no_optional: false,
            }
            .mutates_instance()
        );
        assert!(
            Commands::Import {
                file: "pack.zip".to_string(),
                merge_strategy: None,
            }
            .mutates_instance()
        );
        assert!(!Commands::Outdated { mod_name: None }.mutates_instance());
        assert!(
            !Commands::Audit {
                min_risk: 0,
                fail_on_risk: None,
                mod_filter: None,
                report: None,
                limit: 20,
            }
            .mutates_instance()
        );
        assert!(
            !Commands::Export {
                file: None,
                target: None,
                format: "zip".to_string(),
            }
            .mutates_instance()
        );
    }

    #[test]
    fn remote_removal_accepts_a_human_visible_list_index() {
        let cli =
            Cli::try_parse_from(["orbit", "remote", "remove", "sodium", "--index", "2"]).unwrap();
        let Commands::Remote {
            command:
                RemoteCommands::Remove {
                    package,
                    provider,
                    locator,
                    index,
                },
        } = cli.command
        else {
            panic!("remote remove command was not parsed");
        };

        assert_eq!(package, "sodium");
        assert_eq!(index, Some(2));
        assert!(provider.is_none());
        assert!(locator.is_none());
    }

    #[test]
    fn env_command_accepts_explicit_and_auto_values_for_core_validation() {
        let cli = Cli::try_parse_from(["orbit", "env", "sodium", "auto"]).unwrap();
        let Commands::Env {
            package,
            environment,
        } = cli.command
        else {
            panic!("env command was not parsed");
        };

        assert_eq!(package, "sodium");
        assert_eq!(environment, "auto");
    }

    #[test]
    fn config_set_accepts_canonical_typed_key_syntax() {
        let cli =
            Cli::try_parse_from(["orbit", "config", "set", "cache.capacity-mib", "2048"]).unwrap();
        let Commands::Config {
            command: ConfigCommands::Set { key, value },
        } = cli.command
        else {
            panic!("config set command was not parsed");
        };

        assert_eq!(key, "cache.capacity-mib");
        assert_eq!(value, "2048");
    }

    #[test]
    fn migration_is_namespaced_and_only_export_mutates() {
        let check = Cli::try_parse_from(["orbit", "migrate", "check", "target"]).unwrap();
        let Commands::Migrate {
            command:
                MigrateCommands::Check {
                    target,
                    source_pack,
                },
        } = check.command
        else {
            panic!("migrate check was not parsed");
        };
        assert_eq!(target, PathBuf::from("target"));
        assert!(source_pack.is_none());

        let export = Cli::try_parse_from([
            "orbit",
            "migrate",
            "export",
            "target",
            "--source-pack",
            "source.zip",
            "--consume-source-pack",
        ])
        .unwrap();
        assert!(export.command.mutates_instance());
        assert!(Cli::try_parse_from(["orbit", "check", "1.21"]).is_err());
    }
}
