pub mod add;
pub mod audit;
pub mod cache;
pub mod check;
pub mod export;
pub mod import;
pub mod info;
pub mod init;
pub mod install;
pub mod instances;
pub mod list;
pub mod outdated;
pub mod purge;
pub mod remote;
pub mod remove;
pub mod search;
pub mod sync;
pub mod upgrade;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// 全局 CLI 上下文，传递给所有命令 handler。
#[derive(Debug, Clone)]
pub struct CliContext {
    pub verbose: bool,
    pub quiet: bool,
    pub yes: bool,
    pub dry_run: bool,
    pub instance: Option<String>,
    pub runtime: orbit_core::RuntimeContext,
    /// 输出格式与进度协议，由全局 `--format` / `--progress-format` 决定。
    pub output: crate::cli::output::OutputCfg,
}

impl CliContext {
    pub fn instance_dir(&self) -> Result<PathBuf> {
        if let Some(name) = &self.instance {
            let registry =
                orbit_core::InstancesRegistry::load(self.runtime.paths().instances_file())
                    .context("failed to load instances registry")?;
            let entry = registry.find(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown instance '{name}'. Run 'orbit instances list' to see registered instances."
                )
            })?;
            let path = PathBuf::from(&entry.path);
            if self.verbose && !self.quiet {
                eprintln!("Using instance '{name}' at {}", path.display());
            }
            return Ok(path);
        }

        let path = std::env::current_dir().context("failed to get current directory")?;
        if path.join("orbit.toml").exists() {
            if self.verbose && !self.quiet {
                eprintln!("Using current directory as instance: {}", path.display());
            }
            return Ok(path);
        }

        let registry = orbit_core::InstancesRegistry::load(self.runtime.paths().instances_file())
            .context("failed to load instances registry")?;
        if let Some(instance) = registry.default_instance() {
            let default_path = PathBuf::from(&instance.path);
            if self.verbose && !self.quiet {
                eprintln!(
                    "Using default instance '{}' at {}",
                    instance.name,
                    default_path.display()
                );
            }
            return Ok(default_path);
        }

        if self.verbose && !self.quiet {
            eprintln!(
                "No project or default instance found; using {}",
                path.display()
            );
        }
        Ok(path)
    }

    /// Prevent an instance-mutating command from silently using the global
    /// default while the user is standing in an unrelated directory.
    pub fn require_explicit_mutation_target(&self) -> Result<()> {
        if self.instance.is_some() {
            return Ok(());
        }
        let current_dir = std::env::current_dir().context("failed to get current directory")?;
        if current_dir.join("orbit.toml").exists() {
            return Ok(());
        }
        let registry = orbit_core::InstancesRegistry::load(self.runtime.paths().instances_file())
            .context("failed to load instances registry")?;
        if let Some(instance) = registry.default_instance() {
            anyhow::bail!(
                "refusing to modify the default instance '{}' from outside its project \
                 directory; pass --instance '{}' or change to {}",
                instance.name,
                instance.name,
                instance.path
            );
        }
        Ok(())
    }
}

pub trait CommandHandler {
    async fn execute(self, ctx: &CliContext) -> Result<()>;
}

pub use add::handle as handle_add;
pub use audit::handle as handle_audit;
pub use check::handle as handle_check;
pub use export::handle as handle_export;
pub use import::handle as handle_import;
pub use info::handle as handle_info;
pub use init::handle as handle_init;
pub use install::handle as handle_install;
pub use list::handle as handle_list;
pub use outdated::handle as handle_outdated;
pub use purge::handle as handle_purge;
pub use remote::handle as handle_remote;
pub use remove::handle as handle_remove;
pub use search::handle as handle_search;
pub use sync::handle as handle_sync;
pub use upgrade::handle as handle_upgrade;

pub fn install_interaction(ctx: &CliContext) -> orbit_core::InstallInteraction {
    orbit_core::InstallInteraction {
        select_package: package_selector(ctx.yes),
        select_resolution: resolution_selector(ctx.dry_run, ctx.yes),
        confirm_install: (!ctx.dry_run).then(|| {
            let yes = ctx.yes;
            Box::new(move |report: &orbit_core::InstallReport| prompt_install_report(report, yes))
                as orbit_core::InstallPrompt
        }),
        progress: operation_progress(ctx),
    }
}

pub fn operation_progress(ctx: &CliContext) -> Option<orbit_core::ProgressReporter> {
    if ctx.output.ndjson_progress() {
        return Some(crate::cli::output::ndjson_progress_reporter());
    }
    crate::cli::progress::reporter(ctx.quiet, &ctx.runtime.config().ui.progress_bar)
}

fn package_selector(yes: bool) -> Option<orbit_core::PackageSelector> {
    (!yes).then(|| Box::new(prompt_package) as orbit_core::PackageSelector)
}

fn prompt_package(packages: &[String]) -> usize {
    eprintln!("\nThe provider project contains multiple feasible JAR-declared packages:");
    for (index, package) in packages.iter().enumerate() {
        eprintln!("  {}. {package}", index + 1);
    }
    loop {
        eprint!(
            "\nChoose the package to add [1-{}] (default 1): ",
            packages.len()
        );
        use std::io::Write;
        std::io::stderr().flush().ok();
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() || input.trim().is_empty() {
            return 0;
        }
        if let Ok(choice) = input.trim().parse::<usize>()
            && (1..=packages.len()).contains(&choice)
        {
            return choice - 1;
        }
        eprintln!("Please enter a number from 1 to {}.", packages.len());
    }
}

pub fn resolution_selector(_dry_run: bool, yes: bool) -> Option<orbit_core::ResolutionSelector> {
    (!yes).then(|| Box::new(prompt_resolution) as orbit_core::ResolutionSelector)
}

fn prompt_resolution(alternatives: &[orbit_core::ResolutionReport]) -> usize {
    eprintln!("\nMultiple dependency solutions are available:");
    eprintln!("{}", crate::cli::output::resolution_choices(alternatives));

    loop {
        eprint!(
            "\nChoose a dependency solution [1-{}] (default 1): ",
            alternatives.len()
        );
        use std::io::Write;
        std::io::stderr().flush().ok();
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() || input.trim().is_empty() {
            return 0;
        }
        if let Ok(choice) = input.trim().parse::<usize>()
            && (1..=alternatives.len()).contains(&choice)
        {
            return choice - 1;
        }
        eprintln!("Please enter a number from 1 to {}.", alternatives.len());
    }
}

pub fn prompt_install_report(report: &orbit_core::InstallReport, yes: bool) -> bool {
    if report.installed.is_empty() && report.removed.is_empty() && report.changes.is_empty() {
        return true;
    }
    if !report.changes.is_empty() {
        eprintln!("\nPlanned package transaction:");
        eprintln!(
            "{}",
            crate::cli::output::package_changes_table(&report.changes)
        );
    }
    if !report.installed.is_empty() {
        eprintln!("\nSelected package contents:");
        for m in &report.installed {
            eprintln!("  {} v{}", m.mod_id, m.version);
            for expression in &m.dependencies {
                for dependency in expression.relations() {
                    eprintln!(
                        "      ↳ {} {} ({:?})",
                        dependency.id, dependency.requirement, dependency.kind
                    );
                }
            }
            for bundled in &m.bundled {
                print_bundled_mod(bundled, 1);
            }
        }
    }
    if report.changes.is_empty() && !report.removed.is_empty() {
        eprintln!("\nThe following unselected package versions will be removed:");
        eprintln!(
            "{}",
            crate::cli::output::removed_packages_table(&report.removed)
        );
    }
    if !report.already_satisfied.is_empty() {
        eprintln!(
            "\nAlready satisfied: {}",
            report.already_satisfied.join(", ")
        );
    }
    if yes {
        return true;
    }
    eprint!("\nDo you want to continue? [Y/n] ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}

fn print_bundled_mod(bundled: &orbit_core::BundledMod, depth: usize) {
    let indent = "    ".repeat(depth);
    eprintln!(
        "      ↳ {indent}[bundled] {} {}",
        bundled.mod_id, bundled.version
    );
    for child in &bundled.bundled {
        print_bundled_mod(child, depth + 1);
    }
}

pub fn print_resolution_diagnostics(
    diagnostics: &[orbit_core::resolver::types::CandidateDiagnostic],
) {
    if !diagnostics.is_empty() {
        eprintln!("\nUpgrade diagnostics:");
        eprintln!("{}", crate::cli::output::diagnostics_table(diagnostics));
    }
}

pub fn print_resolution_warnings(warnings: &[String]) {
    if !warnings.is_empty() {
        eprintln!("\nDependency ordering warnings:");
        for warning in warnings {
            eprintln!("  • {warning}");
        }
    }
}

/// Print a transaction-style result (used by `add`, `install`, `upgrade`)
/// honoring the configured output format.
pub fn print_transaction_result(
    command: &'static str,
    report: &orbit_core::InstallReport,
    ctx: &CliContext,
) {
    use crate::cli::output::OutputFormat;
    match ctx.output.format {
        OutputFormat::Text => {
            print_resolution_diagnostics(&report.diagnostics);
            print_resolution_warnings(&report.warnings);
            if ctx.dry_run {
                println!("\n{command} preview:");
                println!(
                    "{}",
                    crate::cli::output::package_changes_table(&report.changes)
                );
                return;
            }
            if report.installed.is_empty() && report.removed.is_empty() {
                println!("No new mods were installed.");
            } else {
                println!(
                    "\nApplied {} selected package version(s) and removed {} unselected package version(s).",
                    report.installed.len(),
                    report.removed.len()
                );
            }
        }
        OutputFormat::Json => {
            let view = crate::cli::output::transaction_view(report, ctx.dry_run);
            crate::cli::output::print_json(command, &view);
        }
    }
}

pub fn create_instance_providers(
    instance_dir: &Path,
    platform: Option<&str>,
    runtime: &orbit_core::RuntimeContext,
) -> Result<Vec<Box<dyn orbit_core::ModProvider>>> {
    let mut catalogs = if let Some(platform) = platform {
        vec![normalize_platform(platform).to_string()]
    } else {
        match orbit_core::ManifestFile::open(instance_dir) {
            Ok(manifest) => manifest.inner.resolver.catalogs,
            Err(orbit_core::OrbitError::ManifestNotFound) => vec!["modrinth".to_string()],
            Err(error) => return Err(error).context("failed to read orbit.toml"),
        }
    };
    if platform.is_none()
        && let Ok(lockfile) = orbit_core::Lockfile::open(instance_dir)
    {
        for entry in &lockfile.inner.packages {
            for remote in &entry.remotes {
                let provider = remote.provider();
                if provider != "file" && !catalogs.iter().any(|item| item == provider) {
                    catalogs.push(provider.to_string());
                }
            }
            for source in &entry.artifact_sources {
                let provider = source.provider();
                if provider != "file" && !catalogs.iter().any(|item| item == provider) {
                    catalogs.push(provider.to_string());
                }
            }
        }
    }
    if platform.is_none()
        && let Ok(manifest) = orbit_core::ManifestFile::open(instance_dir)
    {
        for remote in manifest
            .inner
            .dependencies
            .values()
            .flat_map(|dependency| dependency.remotes.iter())
        {
            let provider = remote.provider();
            if provider != "file" && !catalogs.iter().any(|item| item == provider) {
                catalogs.push(provider.to_string());
            }
        }
    }
    orbit_core::providers::create_providers(&catalogs, &runtime.config().auth)
        .context("failed to create providers")
}

pub fn resolve_platform_target<'a>(
    input: &'a str,
    requested_platform: Option<&str>,
) -> Result<(Option<String>, &'a str)> {
    let (prefixed_platform, target) = if let Some(target) = input.strip_prefix("mr:") {
        (Some("modrinth"), target)
    } else if let Some(target) = input.strip_prefix("cf:") {
        (Some("curseforge"), target)
    } else {
        (None, input)
    };
    if target.is_empty() {
        anyhow::bail!("platform prefix must be followed by a project slug or ID");
    }

    let requested_platform = requested_platform.map(normalize_platform);
    if let (Some(prefixed), Some(requested)) = (prefixed_platform, requested_platform)
        && prefixed != requested
    {
        anyhow::bail!(
            "'{input}' selects {prefixed}, but --platform selects {requested}; use one platform"
        );
    }
    Ok((
        prefixed_platform
            .or(requested_platform)
            .map(ToString::to_string),
        target,
    ))
}

fn normalize_platform(platform: &str) -> &str {
    match platform {
        "mr" => "modrinth",
        "cf" => "curseforge",
        other => other,
    }
}

pub fn parse_package_remote(provider: &str, locator: &str) -> Result<orbit_core::PackageRemote> {
    match normalize_platform(provider) {
        "file" => Ok(orbit_core::PackageRemote::File {
            path: locator.to_string(),
        }),
        "modrinth" => Ok(orbit_core::PackageRemote::Modrinth {
            project_id: locator.to_string(),
        }),
        "curseforge" => Ok(orbit_core::PackageRemote::Curseforge {
            project_id: locator.parse().map_err(|_| {
                anyhow::anyhow!("CurseForge remotes require a numeric project ID, got '{locator}'")
            })?,
        }),
        other => anyhow::bail!("unsupported package remote '{other}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_platform_target;

    #[test]
    fn platform_prefix_selects_one_provider() {
        assert_eq!(
            resolve_platform_target("cf:238222", None).unwrap(),
            (Some("curseforge".to_string()), "238222")
        );
        assert_eq!(
            resolve_platform_target("mr:sodium", Some("modrinth")).unwrap(),
            (Some("modrinth".to_string()), "sodium")
        );
    }

    #[test]
    fn conflicting_platform_selectors_are_rejected() {
        let error = resolve_platform_target("cf:238222", Some("modrinth")).unwrap_err();
        assert!(error.to_string().contains("selects curseforge"));
    }
}
