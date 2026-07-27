use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "orbit-launcher", version, about = "Minecraft runtime launcher")]
pub struct Cli {
    /// Select a registered instance by stable ID or name.
    #[arg(long, global = true)]
    pub instance: Option<String>,

    /// Final output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    pub format: OutputFormat,

    /// Progress event protocol written to stderr.
    #[arg(long, value_enum, default_value_t = ProgressFormat::None, global = true)]
    pub progress_format: ProgressFormat,

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
    None,
    Ndjson,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
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
        matches!(self, Self::Show | Self::Rename { .. } | Self::Remove)
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
}
