use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use orbit_i18n::LanguageMode;

pub use super::output::{OutputFormat, ProgressFormat};

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "The Modern, Non-intrusive Package Manager for Minecraft Mods.", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Override the configured presentation language: system / en / zh-CN.
    #[arg(long, global = true)]
    pub language: Option<LanguageMode>,

    /// Select an instance by name.
    #[arg(short = 'i', long, global = true)]
    pub instance: Option<String>,

    /// Exact global configuration file path.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Exact global JAR cache directory.
    #[arg(long, global = true)]
    pub cache_dir: Option<PathBuf>,

    /// Exact global version repository directory.
    #[arg(long, global = true)]
    pub repository_dir: Option<PathBuf>,

    /// Default path layout: system / executable.
    #[arg(long, global = true)]
    pub data_layout: Option<orbit_core::PathLayout>,

    /// Output format: text / json.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    pub output_format: OutputFormat,

    /// Progress protocol: none / ndjson (stderr only).
    #[arg(long, global = true, value_enum, default_value_t = ProgressFormat::None)]
    pub progress_format: ProgressFormat,

    /// Show resolved runtime and instance context.
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
    Instance {
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
        /// Ordered set rule over the complete JAR-declared version string.
        #[arg(long)]
        string: Option<String>,
        /// Environment filter: client / server / both.
        #[arg(long)]
        env: Option<String>,
        /// Mark as an optional dependency.
        #[arg(long)]
        optional: bool,
    },

    /// Remove a mod.
    Remove {
        /// Mod name.
        mod_name: String,
    },

    /// Launch Minecraft through Orbit Launcher with runtime data observation.
    Launch {
        /// Exact Orbit Launcher executable; defaults to the one beside Orbit.
        #[arg(long)]
        launcher: Option<PathBuf>,
        /// Exact Launcher instance ID or name.
        #[arg(long)]
        launcher_instance: Option<String>,
        /// Start the selected dedicated server instead of a client.
        #[arg(long)]
        server: bool,
        /// Exact Orbit Runtime Agent JAR (packaging/testing override).
        #[arg(long, hide = true)]
        runtime_agent: Option<PathBuf>,
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

    /// Import an Orbit bundle, an official mrpack, or an Orbit manifest.
    Import {
        /// File path (.toml, .orbitbundle, or .mrpack).
        file: String,
        /// Merge strategy.
        #[arg(long)]
        merge_strategy: Option<String>,
        /// Include one exact optional mrpack path; repeat for multiple files.
        #[arg(long = "optional")]
        optional_files: Vec<String>,
        /// Include every mrpack entry optional for the selected side.
        #[arg(long)]
        all_optional: bool,
    },

    /// Export the current instance as an archive.
    Export {
        /// Output file path.
        file: Option<String>,
        /// Target environment filter.
        #[arg(long)]
        target: Option<String>,
        /// Export format: orbit / mrpack.
        #[arg(long = "format", default_value = "orbit")]
        archive_format: String,
        /// Export content: mods / mods-and-data.
        #[arg(long, default_value = "mods-and-data")]
        content: String,
    },

    /// Plan or export a package migration into an installed target runtime.
    Migrate {
        #[command(subcommand)]
        command: MigrateCommands,
    },

    /// Show optional audit and runtime-observation support for this instance.
    Capabilities,

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

    /// Inspect or change global or instance configuration.
    Config {
        /// Configuration scope: global settings or this instance's orbit.toml.
        #[arg(long, value_enum, default_value = "global", global = true)]
        scope: ConfigScope,
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Inspect and manage one JAR-declared package.
    Package {
        /// JAR-declared mod_id, not a provider slug.
        package: String,
        #[command(subcommand)]
        command: PackageCommands,
    },
}

#[derive(Subcommand)]
pub enum InstanceCommands {
    /// List all managed Minecraft instances.
    List,
    /// Register an existing, complete Orbit workspace.
    Register { name: String, path: PathBuf },
    /// Stop tracking an instance.
    Unregister { name: String },
}

#[derive(Subcommand)]
pub enum CacheCommands {
    /// Clean the download cache.
    Clean,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ConfigScope {
    Global,
    Instance,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Show the exact configuration file path for this scope.
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
        /// file / modrinth / curseforge.
        provider: String,
        /// File path, Modrinth project ID, or numeric CurseForge project ID.
        locator: String,
    },
    /// Remove a source; the final source cannot be removed.
    Remove {
        /// file / modrinth / curseforge (omit when using --index)
        #[arg(required_unless_present = "index", requires = "locator")]
        provider: Option<String>,
        /// Source locator (omit when using --index)
        #[arg(required_unless_present = "index", requires = "provider")]
        locator: Option<String>,
        /// One-based index shown by `orbit package <mod_id> remote list`
        #[arg(long, conflicts_with_all = ["provider", "locator"])]
        index: Option<usize>,
    },
    /// List every source for a package.
    List,
}

#[derive(Subcommand)]
pub enum ConstraintCommands {
    /// Show the configured policy and current selected version.
    Show,
    /// Apply a structured version policy and immediately converge the instance.
    Set {
        /// Ordered set rule over the complete JAR-declared version string.
        #[arg(long, global = true)]
        string: Option<String>,
        #[command(subcommand)]
        policy: ConstraintPolicyCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConstraintPolicyCommands {
    /// Allow every compatible version.
    Any,
    /// Match one dotted numeric core.
    Exact { version: String },
    /// Require a numeric core strictly newer than the selected boundary.
    GreaterThan { version: String },
    /// Require the boundary numeric core or anything newer.
    AtLeast { version: String },
    /// Require a numeric core strictly older than the selected boundary.
    LessThan { version: String },
    /// Require the boundary numeric core or anything older.
    AtMost { version: String },
    /// Restrict the package to a bounded interval.
    Range {
        lower: String,
        upper: String,
        #[arg(long, value_enum, default_value_t = BoundInclusion::Inclusive)]
        lower_bound: BoundInclusion,
        #[arg(long, value_enum, default_value_t = BoundInclusion::Inclusive)]
        upper_bound: BoundInclusion,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BoundInclusion {
    Inclusive,
    Exclusive,
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
        /// Permit a Pareto-minimal package-removing plan without prompting.
        #[arg(long)]
        allow_removals: bool,
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
        /// Permit a Pareto-minimal package-removing plan without prompting.
        #[arg(long)]
        allow_removals: bool,
    },
}

#[derive(Subcommand)]
pub enum PackageCommands {
    /// Show the configured version policy and locked selection without downloading candidates.
    Constraint {
        #[command(subcommand)]
        command: ConstraintCommands,
    },
    /// Enumerate and parse remote versions for this package.
    Versions,
    /// Show the installed JAR and owned files or directory trees.
    Files,
    /// Set an environment filter; auto follows JAR metadata.
    Env { environment: String },
    /// Allow the loader to discover this package's JAR.
    Enable,
    /// Retain this package but rename its JAR so the loader skips it.
    Disable,
    /// Delete owned data while retaining the package.
    Reset,
    /// Remove this package and its exclusive runtime data.
    Purge,
    /// Manage sources without changing the JAR-declared package identity.
    Remote {
        #[command(subcommand)]
        command: RemoteCommands,
    },
}
