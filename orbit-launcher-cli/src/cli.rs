use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use orbit_i18n::LanguageMode;

#[derive(Debug, Parser)]
#[command(name = "orbit-launcher", version, about = "Minecraft runtime launcher")]
pub struct Cli {
    /// Presentation language: system / en / zh-CN (system by default).
    #[arg(long, global = true, default_value_t = LanguageMode::System)]
    pub language: LanguageMode,

    /// Select a registered instance by stable ID or name.
    #[arg(long, global = true)]
    pub instance: Option<String>,

    /// Final output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    pub format: OutputFormat,

    /// Progress event protocol written to stderr.
    #[arg(long, value_enum, default_value_t = ProgressFormat::Text, global = true)]
    pub progress_format: ProgressFormat,

    /// Disable all prompts; missing decisions become interaction_required errors.
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Exact launcher configuration directory.
    #[arg(long, global = true)]
    pub config_dir: Option<PathBuf>,

    /// Exact launcher data directory.
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,

    /// Exact launcher cache directory.
    #[arg(long, global = true)]
    pub cache_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProgressFormat {
    Text,
    None,
    Ndjson,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Resolve, download, verify, and atomically install an existing or new instance.
    Install {
        /// Create and install a new named instance in one command.
        #[arg(long)]
        new: Option<String>,
        /// New instance root; only valid with --new and defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// New instance kind; required with --new.
        #[arg(long, value_enum)]
        kind: Option<InstanceKindArg>,
        /// Minecraft requirement for a new instance; required with --new.
        #[arg(long)]
        minecraft: Option<String>,
        /// Loader for a new instance; defaults to Vanilla.
        #[arg(long, value_enum)]
        loader: Option<LoaderKindArg>,
        /// Loader requirement for a new non-Vanilla instance.
        #[arg(long)]
        loader_version: Option<String>,
    },

    /// Verify and launch a client instance with its selected account.
    Launch {
        /// Verify and print a token-redacted command without starting Java.
        #[arg(long)]
        dry_run: bool,
    },

    /// Inspect and change launcher-wide configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Create, import, inspect, and register launcher instances.
    Instance {
        #[command(subcommand)]
        command: InstanceCommands,
    },

    /// Manage a dedicated server and its legal/runtime state.
    Server {
        #[command(subcommand)]
        command: ServerCommands,
    },

    /// Log in once, select accounts, and manage persisted sessions.
    Account {
        #[command(subcommand)]
        command: AccountCommands,
    },

    /// Browse authoritative Minecraft, Loader, and Java runtime metadata.
    Versions {
        #[command(subcommand)]
        command: VersionCommands,
    },

    /// Inspect, verify, and remove managed Java runtimes.
    Java {
        #[command(subcommand)]
        command: JavaCommands,
    },

    /// Internal detached supervisor entrypoint.
    #[command(name = "__supervisor", hide = true)]
    Supervisor,
}

#[derive(Debug, Subcommand)]
pub enum AccountCommands {
    /// Create or renew an account session.
    Login {
        #[command(subcommand)]
        command: AccountLoginCommands,
    },
    /// List non-secret account metadata.
    List,
    /// Show one account; defaults to the global selection.
    Show { account: Option<String> },
    /// Select an account for this client instance, or globally with --global.
    Select {
        account: String,
        #[arg(long)]
        global: bool,
    },
    /// Clear this client instance's account, or the global selection with --global.
    Clear {
        #[arg(long)]
        global: bool,
    },
    /// Delete a persisted local account session and its non-secret metadata.
    Logout { account: String },
}

#[derive(Debug, Subcommand)]
pub enum JavaCommands {
    /// List installed managed runtimes; optionally verify every file.
    List {
        #[arg(long)]
        verify: bool,
    },
    /// Verify one installed runtime by its stable ID.
    Verify { runtime_id: String },
    /// Remove an unreferenced runtime; instance locks prevent unsafe removal.
    Remove { runtime_id: String },
}

#[derive(Debug, Subcommand)]
pub enum VersionCommands {
    /// List Mojang Minecraft versions in official manifest order.
    Minecraft,
    /// List official Loader versions compatible with one exact Minecraft version.
    Loader {
        #[arg(long, value_enum)]
        loader: LoaderKindArg,
        #[arg(long)]
        minecraft: String,
    },
    /// Show the authoritative Java component required by one exact Minecraft version.
    Java {
        #[arg(long)]
        minecraft: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccountLoginCommands {
    /// Create a deterministic offline profile; this does not authenticate with Microsoft.
    Offline { profile_name: String },
    /// Use Microsoft's OAuth device authorization flow.
    Microsoft {
        #[command(subcommand)]
        command: MicrosoftLoginCommands,
    },
    /// Authenticate against a configured standard External Yggdrasil provider.
    Yggdrasil {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        username: String,
        /// Required when the account owns more than one Minecraft profile.
        #[arg(long)]
        profile: Option<String>,
        /// Read exactly one password line from stdin, including in JSON/non-interactive mode.
        #[arg(long)]
        password_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MicrosoftLoginCommands {
    /// Start device authorization and return a public user code.
    Begin,
    /// Poll and finish a previously started device authorization.
    Complete { login_session_id: uuid::Uuid },
}

#[derive(Debug, Subcommand)]
pub enum ServerCommands {
    /// Run a server in the foreground.
    Run {
        /// Verify and print the command without starting Java.
        #[arg(long)]
        dry_run: bool,
    },
    /// Start a managed supervisor in the background for this login session.
    Start,
    /// Request a graceful stop through the managed supervisor.
    Stop,
    /// Query the managed supervisor without inspecting or guessing process IDs.
    Status,
    /// Send one Minecraft console command through the managed supervisor.
    Command {
        #[arg(required = true, trailing_var_arg = true)]
        value: Vec<String>,
    },
    /// Display or accept the current official Minecraft EULA.
    Eula {
        #[command(subcommand)]
        command: EulaCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum EulaCommands {
    /// Fetch and output the complete current official EULA.
    Show,
    /// Accept exactly the digest returned by the latest show for this instance.
    Accept { digest: String },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Print the exact global configuration file path.
    Path,
    /// List all supported scalar settings and their effective values.
    List,
    /// Read one setting by its canonical key.
    Get { key: String },
    /// Set one setting after typed validation.
    Set { key: String, value: String },
    /// Remove an explicit setting and restore its default value.
    Unset { key: String },
    /// Configure standard External Yggdrasil endpoints.
    Yggdrasil {
        #[command(subcommand)]
        command: YggdrasilProviderCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum YggdrasilProviderCommands {
    /// List configured providers.
    List,
    /// Discover and add a provider from a website or exact API root.
    Add {
        id: String,
        api_root: String,
        /// Explicitly permit an HTTP endpoint; credentials can be intercepted.
        #[arg(long)]
        allow_insecure_http: bool,
    },
    /// Remove a provider; existing account metadata is preserved but cannot launch.
    Remove { id: String },
}

#[derive(Debug, Subcommand)]
pub enum InstanceCommands {
    /// Create and register an instance manifest without installing runtime artifacts.
    Create {
        #[arg(long)]
        name: String,
        /// Instance root; defaults to the exact current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long, value_enum)]
        kind: InstanceKindArg,
        #[arg(long)]
        minecraft: String,
        #[arg(long, value_enum, default_value_t = LoaderKindArg::Vanilla)]
        loader: LoaderKindArg,
        /// Required for non-Vanilla loaders; for example `stable` or an exact version.
        #[arg(long)]
        loader_version: Option<String>,
    },

    /// Register an existing orbit-launcher.toml, including after moving it.
    Import {
        /// Instance root; defaults to the exact current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// List globally registered instances.
    List,

    /// Show the explicit, local, or default instance.
    Show,

    /// Rename the explicit or local instance.
    Rename { new_name: String },

    /// Change desired Minecraft, loader, or Java policy before installing.
    Configure {
        #[arg(long)]
        minecraft: Option<String>,
        #[arg(long, value_enum)]
        loader: Option<LoaderKindArg>,
        #[arg(long)]
        loader_version: Option<String>,
        #[arg(long, value_enum)]
        java_policy: Option<JavaPolicyArg>,
    },

    /// Unregister the explicit or local instance without deleting its files.
    Remove,

    /// Manage the read-only global default context.
    Default {
        #[command(subcommand)]
        command: DefaultCommands,
    },
}

impl InstanceCommands {
    pub const fn accepts_instance_context(&self) -> bool {
        matches!(
            self,
            Self::Show | Self::Rename { .. } | Self::Configure { .. } | Self::Remove
        )
    }
}

#[derive(Debug, Subcommand)]
pub enum DefaultCommands {
    /// Select a registered instance as the read-only default.
    Set { instance: String },
    /// Clear the read-only default.
    Clear,
    /// Show the current default.
    Show,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InstanceKindArg {
    Client,
    Server,
}

impl From<InstanceKindArg> for orbit_launcher_core::InstanceKind {
    fn from(value: InstanceKindArg) -> Self {
        match value {
            InstanceKindArg::Client => Self::Client,
            InstanceKindArg::Server => Self::Server,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LoaderKindArg {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    Neoforge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum JavaPolicyArg {
    Auto,
    Managed,
}

impl From<JavaPolicyArg> for orbit_launcher_core::JavaPolicy {
    fn from(value: JavaPolicyArg) -> Self {
        match value {
            JavaPolicyArg::Auto => Self::Auto,
            JavaPolicyArg::Managed => Self::Managed,
        }
    }
}

impl From<LoaderKindArg> for orbit_launcher_core::LoaderKind {
    fn from(value: LoaderKindArg) -> Self {
        match value {
            LoaderKindArg::Vanilla => Self::Vanilla,
            LoaderKindArg::Fabric => Self::Fabric,
            LoaderKindArg::Quilt => Self::Quilt,
            LoaderKindArg::Forge => Self::Forge,
            LoaderKindArg::Neoforge => Self::Neoforge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_accepts_global_options_after_subcommands_for_gui_callers() {
        let cli = Cli::try_parse_from([
            "orbit-launcher",
            "instance",
            "create",
            "--name",
            "server",
            "--kind",
            "server",
            "--minecraft",
            "1.21.1",
            "--loader",
            "fabric",
            "--loader-version",
            "stable",
            "--format",
            "json",
        ])
        .unwrap();
        assert_eq!(cli.format, OutputFormat::Json);
    }

    #[test]
    fn remove_has_no_positional_path_or_instance_selector() {
        let cli = Cli::try_parse_from([
            "orbit-launcher",
            "--instance",
            "server-id",
            "instance",
            "remove",
        ])
        .unwrap();
        assert_eq!(cli.instance.as_deref(), Some("server-id"));
    }

    #[test]
    fn config_keys_remain_protocol_strings_instead_of_clap_enums() {
        let cli = Cli::try_parse_from([
            "orbit-launcher",
            "config",
            "set",
            "cache.max-size",
            "8 GiB",
            "--format",
            "json",
        ])
        .unwrap();
        let Commands::Config {
            command: ConfigCommands::Set { key, value },
        } = cli.command
        else {
            panic!("unexpected command");
        };
        assert_eq!(key, "cache.max-size");
        assert_eq!(value, "8 GiB");
    }

    #[test]
    fn install_new_keeps_creation_paths_at_the_bootstrap_boundary() {
        let cli = Cli::try_parse_from([
            "orbit-launcher",
            "install",
            "--new",
            "server",
            "--root",
            "./server",
            "--kind",
            "server",
            "--minecraft",
            "latest-release",
        ])
        .unwrap();
        let Commands::Install {
            new,
            root,
            kind,
            minecraft,
            ..
        } = cli.command
        else {
            panic!("unexpected command");
        };
        assert_eq!(new.as_deref(), Some("server"));
        assert_eq!(root.as_deref(), Some(std::path::Path::new("./server")));
        assert_eq!(kind, Some(InstanceKindArg::Server));
        assert_eq!(minecraft.as_deref(), Some("latest-release"));
        assert_eq!(cli.progress_format, ProgressFormat::Text);
    }

    #[test]
    fn launch_surfaces_have_explicit_dry_run_modes() {
        let client = Cli::try_parse_from(["orbit-launcher", "launch", "--dry-run"]).unwrap();
        assert!(matches!(client.command, Commands::Launch { dry_run: true }));

        let server = Cli::try_parse_from(["orbit-launcher", "server", "run", "--dry-run"]).unwrap();
        assert!(matches!(
            server.command,
            Commands::Server {
                command: ServerCommands::Run { dry_run: true }
            }
        ));
    }

    #[test]
    fn server_supervisor_commands_have_distinct_cli_shapes() {
        for name in ["start", "stop", "status"] {
            Cli::try_parse_from(["orbit-launcher", "server", name]).unwrap();
        }
        let command =
            Cli::try_parse_from(["orbit-launcher", "server", "command", "say", "hello world"])
                .unwrap();
        let Commands::Server {
            command: ServerCommands::Command { value },
        } = command.command
        else {
            panic!("unexpected command");
        };
        assert_eq!(value, ["say", "hello world"]);
    }

    #[test]
    fn instance_configure_accepts_runtime_policy_updates() {
        let cli = Cli::try_parse_from([
            "orbit-launcher",
            "--instance",
            "client",
            "instance",
            "configure",
            "--minecraft",
            "latest-release",
            "--loader",
            "fabric",
            "--loader-version",
            "stable",
            "--java-policy",
            "managed",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Instance {
                command: InstanceCommands::Configure {
                    loader: Some(LoaderKindArg::Fabric),
                    java_policy: Some(JavaPolicyArg::Managed),
                    ..
                }
            }
        ));
    }

    #[test]
    fn java_management_commands_are_global_and_explicit() {
        let list = Cli::try_parse_from(["orbit-launcher", "java", "list", "--verify"]).unwrap();
        assert!(matches!(
            list.command,
            Commands::Java {
                command: JavaCommands::List { verify: true }
            }
        ));
        let remove =
            Cli::try_parse_from(["orbit-launcher", "java", "remove", "runtime-21"]).unwrap();
        assert!(matches!(
            remove.command,
            Commands::Java {
                command: JavaCommands::Remove { .. }
            }
        ));
    }

    #[test]
    fn version_catalog_commands_require_explicit_compatibility_axes() {
        let minecraft = Cli::try_parse_from(["orbit-launcher", "versions", "minecraft"]).unwrap();
        assert!(matches!(
            minecraft.command,
            Commands::Versions {
                command: VersionCommands::Minecraft
            }
        ));
        let loader = Cli::try_parse_from([
            "orbit-launcher",
            "versions",
            "loader",
            "--loader",
            "fabric",
            "--minecraft",
            "1.21.1",
        ])
        .unwrap();
        assert!(matches!(
            loader.command,
            Commands::Versions {
                command: VersionCommands::Loader {
                    loader: LoaderKindArg::Fabric,
                    ..
                }
            }
        ));
    }

    #[test]
    fn internal_supervisor_entrypoint_is_parseable_but_hidden() {
        let cli = Cli::try_parse_from(["orbit-launcher", "__supervisor"]).unwrap();
        assert!(matches!(cli.command, Commands::Supervisor));
        let help = Cli::try_parse_from(["orbit-launcher", "--help"])
            .unwrap_err()
            .to_string();
        assert!(!help.contains("__supervisor"));
    }
}
