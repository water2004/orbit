pub mod activation;
pub mod add;
pub mod audit;
pub mod cache;
pub mod config;
pub mod constraint;
pub mod env;
pub mod export;
pub mod fix;
pub mod import;
pub mod info;
pub mod init;
pub mod install;
pub mod instances;
pub mod list;
pub mod migrate;
pub mod outdated;
pub mod purge;
pub mod remote;
pub mod remove;
pub mod search;
pub mod sync;
pub mod upgrade;
pub mod versions;

pub use constraint::handle as handle_constraint;
pub use env::handle as handle_env;
pub use versions::handle as handle_versions;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// 全局 CLI 上下文，传递给所有命令 handler。
#[derive(Debug, Clone)]
pub struct CliContext {
    pub command: &'static str,
    pub machine_sequence: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub verbose: bool,
    pub quiet: bool,
    pub yes: bool,
    pub dry_run: bool,
    pub instance: Option<String>,
    pub runtime: orbit_core::RuntimeContext,
    /// 输出格式与进度协议，由全局 `--output-format` / `--progress-format` 决定。
    pub output: crate::cli::output::OutputCfg,
}

impl CliContext {
    pub fn print_result(&self, arguments: std::fmt::Arguments<'_>) {
        if !self.quiet {
            print!("{arguments}");
        }
    }

    pub fn print_result_line(&self, arguments: std::fmt::Arguments<'_>) {
        if !self.quiet {
            println!("{arguments}");
        }
    }

    pub fn print_information_line(&self, arguments: std::fmt::Arguments<'_>) {
        if !self.quiet {
            eprintln!("{arguments}");
        }
    }

    pub fn print_json<T: serde::Serialize>(&self, command: &'static str, view: &T) {
        if !self.quiet {
            crate::cli::output::print_json(command, view);
        }
    }

    pub fn print_verbose_runtime(&self) {
        if !self.verbose || self.quiet {
            return;
        }
        let config = self.runtime.config();
        eprintln!(
            "{}",
            tr!(
                "Runtime context: config %{config}; JAR cache %{cache}; version repository %{repository}; network timeout %{timeout}s, %{retries} retries; %{downloads} shared downloads",
                config = self.runtime.paths().config_file().display(),
                cache = self.runtime.paths().cache_dir().display(),
                repository = self.runtime.paths().repository_dir().display(),
                timeout = config.network.timeout,
                retries = config.network.max_retries,
                downloads = config.core.max_concurrent_downloads
            )
        );
    }

    pub fn instance_dir(&self) -> Result<PathBuf> {
        if let Some(name) = &self.instance {
            let registry =
                orbit_core::InstancesRegistry::load(self.runtime.paths().instances_file())
                    .with_context(|| tr!("Failed to load the instances registry").into_owned())?;
            let entry = registry.find(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}",
                    tr!("Unknown instance '%{name}'. Run 'orbit instances list' to see registered instances.", name = name)
                )
            })?;
            let path = PathBuf::from(&entry.path);
            if self.verbose && !self.quiet {
                eprintln!(
                    "{}",
                    tr!(
                        "Using instance '%{name}' at %{path}",
                        name = name,
                        path = path.display()
                    )
                );
            }
            return Ok(path);
        }

        let path = std::env::current_dir()
            .with_context(|| tr!("Failed to get the current directory").into_owned())?;
        if path.join("orbit.toml").exists() {
            if self.verbose && !self.quiet {
                eprintln!(
                    "{}",
                    tr!(
                        "Using the current directory as instance: %{path}",
                        path = path.display()
                    )
                );
            }
            return Ok(path);
        }

        let registry =
            orbit_core::InstancesRegistry::load(self.runtime.paths().instances_file())
                .with_context(|| tr!("Failed to load the instances registry").into_owned())?;
        if let Some(instance) = registry.default_instance() {
            let default_path = PathBuf::from(&instance.path);
            if self.verbose && !self.quiet {
                eprintln!(
                    "{}",
                    tr!(
                        "Using default instance '%{name}' at %{path}",
                        name = instance.name,
                        path = default_path.display()
                    )
                );
            }
            return Ok(default_path);
        }

        if self.verbose && !self.quiet {
            eprintln!(
                "{}",
                tr!(
                    "No project or default instance was found; using %{path}",
                    path = path.display()
                )
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
        let current_dir = std::env::current_dir()
            .with_context(|| tr!("Failed to get the current directory").into_owned())?;
        if current_dir.join("orbit.toml").exists() {
            return Ok(());
        }
        let registry =
            orbit_core::InstancesRegistry::load(self.runtime.paths().instances_file())
                .with_context(|| tr!("Failed to load the instances registry").into_owned())?;
        if let Some(instance) = registry.default_instance() {
            anyhow::bail!(
                "{}",
                tr!(
                    "Refusing to modify default instance '%{name}' outside its project directory; pass --instance '%{name}' or change to %{path}",
                    name = instance.name,
                    path = instance.path
                )
            );
        }
        Ok(())
    }
}

pub trait CommandHandler {
    async fn execute(self, ctx: &CliContext) -> Result<()>;
}

pub use activation::handle as handle_activation;
pub use add::handle as handle_add;
pub use audit::handle as handle_audit;
pub use config::handle as handle_config;
pub use export::handle as handle_export;
pub use fix::handle as handle_fix;
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
        select_package: package_selector(ctx),
        select_resolution: resolution_selector(ctx),
        confirm_install: (!ctx.dry_run).then(|| install_prompt(ctx)),
        progress: operation_progress(ctx),
    }
}

pub fn operation_progress(ctx: &CliContext) -> Option<orbit_core::ProgressReporter> {
    if ctx.output.ndjson_progress() {
        return Some(crate::cli::output::ndjson_progress_reporter(
            ctx.command,
            ctx.machine_sequence.clone(),
        ));
    }
    crate::cli::progress::reporter(ctx.quiet, ctx.runtime.config().ui.progress_bar)
}

fn package_selector(ctx: &CliContext) -> Option<orbit_core::PackageSelector> {
    if ctx.output.format == crate::cli::output::OutputFormat::Json {
        let command = ctx.command;
        let sequence = ctx.machine_sequence.clone();
        Some(Box::new(move |packages| {
            machine_select_package(command, &sequence, packages)
        }))
    } else {
        // `--yes` confirms a transaction; it must never silently choose one
        // real package identity over another.
        Some(Box::new(prompt_package))
    }
}

fn prompt_package(packages: &[String]) -> Result<usize, orbit_core::OrbitError> {
    eprintln!(
        "\n{}",
        tr!("The provider project contains multiple feasible JAR-declared packages:")
    );
    for (index, package) in packages.iter().enumerate() {
        eprintln!("  {}. {package}", index + 1);
    }
    loop {
        eprint!(
            "\n{}",
            tr!(
                "Choose the package to add [1-%{count}] (default 1): ",
                count = packages.len()
            )
        );
        use std::io::Write;
        std::io::stderr().flush().ok();
        let mut input = String::new();
        let Ok(bytes_read) = std::io::stdin().read_line(&mut input) else {
            return Err(interaction_failure(tr!(
                "Package selection could not read stdin"
            )));
        };
        if bytes_read == 0 {
            return Err(orbit_core::OrbitError::Cancelled(
                tr!("Package selection was cancelled because stdin closed").into_owned(),
            ));
        }
        if input.trim().is_empty() {
            return Ok(0);
        }
        if let Ok(choice) = input.trim().parse::<usize>()
            && (1..=packages.len()).contains(&choice)
        {
            return Ok(choice - 1);
        }
        eprintln!(
            "{}",
            tr!(
                "Please enter a number from 1 to %{count}.",
                count = packages.len()
            )
        );
    }
}

pub fn resolution_selector(ctx: &CliContext) -> Option<orbit_core::ResolutionSelector> {
    if ctx.output.format == crate::cli::output::OutputFormat::Json {
        let command = ctx.command;
        let sequence = ctx.machine_sequence.clone();
        Some(Box::new(move |alternatives| {
            machine_select_resolution(command, &sequence, alternatives)
        }))
    } else {
        // Every Pareto alternative remains an explicit decision even with
        // `--yes`; only the subsequent apply confirmation is skipped.
        Some(Box::new(prompt_resolution))
    }
}

fn install_prompt(ctx: &CliContext) -> orbit_core::InstallPrompt {
    if ctx.yes {
        let quiet = ctx.quiet;
        return Box::new(move |report| {
            if quiet {
                Ok(())
            } else {
                prompt_install_report(report, true)
            }
        });
    }
    if ctx.output.format == crate::cli::output::OutputFormat::Json {
        let command = ctx.command;
        let sequence = ctx.machine_sequence.clone();
        Box::new(move |report| machine_confirm_install(command, &sequence, report))
    } else {
        Box::new(|report| prompt_install_report(report, false))
    }
}

fn machine_select_package(
    command: &'static str,
    sequence: &std::sync::atomic::AtomicU64,
    packages: &[String],
) -> Result<usize, orbit_core::OrbitError> {
    use orbit_machine_protocol::{InteractionChoice, InteractionKind};
    let choices = packages
        .iter()
        .map(|package| InteractionChoice {
            id: package.clone(),
            label: package.clone(),
            description: Some(tr!("JAR-declared logical package").into_owned()),
            data: serde_json::json!({ "package": package }),
        })
        .collect();
    let envelope = machine_interaction(
        command,
        sequence,
        "package",
        InteractionKind::Package,
        &tr!("Choose the JAR-declared package identity to add"),
        choices,
        packages.first().cloned(),
    );
    let selected = read_machine_response(&envelope)?;
    packages
        .iter()
        .position(|package| package == &selected)
        .ok_or_else(|| {
            interaction_failure(tr!(
                "Package interaction selected unknown choice '%{choice}'",
                choice = selected
            ))
        })
}

fn machine_select_resolution(
    command: &'static str,
    sequence: &std::sync::atomic::AtomicU64,
    alternatives: &[orbit_core::ResolutionReport],
) -> Result<usize, orbit_core::OrbitError> {
    use orbit_machine_protocol::{InteractionEnvelope, InteractionKind};
    let choices = resolution_interaction_choices(alternatives);
    let envelope: InteractionEnvelope<serde_json::Value> = machine_interaction(
        command,
        sequence,
        "resolution",
        InteractionKind::Resolution,
        &tr!("Choose one non-dominated dependency solution"),
        choices,
        Some("1".to_string()),
    );
    read_machine_response(&envelope)?
        .parse::<usize>()
        .ok()
        .and_then(|selected| selected.checked_sub(1))
        .filter(|selected| *selected < alternatives.len())
        .ok_or_else(|| {
            interaction_failure(tr!("Resolution interaction selected an unknown choice"))
        })
}

fn resolution_interaction_choices(
    alternatives: &[orbit_core::ResolutionReport],
) -> Vec<orbit_machine_protocol::InteractionChoice<serde_json::Value>> {
    use orbit_machine_protocol::InteractionChoice;
    let packages = alternatives
        .iter()
        .flat_map(|alternative| {
            alternative
                .changes
                .iter()
                .map(|change| change.package.clone())
        })
        .collect::<std::collections::BTreeSet<_>>();
    let signatures: Vec<Vec<String>> = alternatives
        .iter()
        .map(|alternative| {
            alternative
                .changes
                .iter()
                .map(|change| {
                    serde_json::to_string(&serde_json::json!({
                        "change": crate::cli::output::package_change_view(change),
                        "candidate_identity": alternative.selected_candidates.get(&change.package),
                    }))
                    .expect("package change signature is serializable")
                })
                .collect()
        })
        .collect();
    alternatives
        .iter()
        .enumerate()
        .map(|(index, alternative)| {
            let mut changes = Vec::new();
            for package in &packages {
                let package_changes = alternative
                    .changes
                    .iter()
                    .enumerate()
                    .filter(|(_, change)| &change.package == package)
                    .collect::<Vec<_>>();
                if package_changes.is_empty() {
                    let current_version = alternatives
                        .iter()
                        .flat_map(|candidate| &candidate.changes)
                        .find(|change| &change.package == package)
                        .and_then(|change| change.current_version.clone());
                    changes.push(serde_json::json!({
                        "different": true,
                        "change": {
                            "package": package,
                            "kind": "keep",
                            "current_version": current_version,
                            "selected_version": current_version,
                            "selected_description": null,
                            "selected_artifact": null,
                        },
                    }));
                    continue;
                }
                for (change_index, change) in package_changes {
                    let signature = &signatures[index][change_index];
                    let common = signatures
                        .iter()
                        .all(|candidate| candidate.iter().any(|item| item == signature));
                    changes.push(serde_json::json!({
                        "different": !common,
                        "change": resolution_package_change_view(change),
                    }));
                }
            }
            InteractionChoice {
                id: (index + 1).to_string(),
                label: tr!("Option %{number}", number = index + 1),
                description: Some(tr!(
                    "%{count} logical package action(s)",
                    count = changes.len()
                )),
                data: serde_json::json!({
                    "changes": changes,
                }),
            }
        })
        .collect()
}

fn resolution_package_change_view(change: &orbit_core::PackageChange) -> serde_json::Value {
    let mut view = serde_json::to_value(crate::cli::output::package_change_view(change))
        .expect("package change view is serializable");
    let selected_artifact = crate::cli::output::selected_artifact_basename(change);
    view.as_object_mut()
        .expect("package change view is an object")
        .insert(
            "selected_artifact".to_string(),
            serde_json::to_value(selected_artifact).expect("artifact filename is serializable"),
        );
    view
}

fn machine_confirm_install(
    command: &'static str,
    sequence: &std::sync::atomic::AtomicU64,
    report: &orbit_core::InstallReport,
) -> Result<(), orbit_core::OrbitError> {
    use orbit_machine_protocol::{InteractionChoice, InteractionKind};
    if report.installed.is_empty() && report.removed.is_empty() && report.changes.is_empty() {
        return Ok(());
    }
    let plan = crate::cli::output::transaction_view(report, false);
    let envelope = machine_interaction(
        command,
        sequence,
        "confirmation",
        InteractionKind::Confirmation,
        &tr!("Review the logical package transaction before applying it"),
        vec![
            InteractionChoice {
                id: "proceed".to_string(),
                label: tr!("Apply changes").into_owned(),
                description: Some(tr!("Commit the displayed logical package actions").into_owned()),
                data: serde_json::to_value(plan).expect("transaction view is serializable"),
            },
            InteractionChoice {
                id: "cancel".to_string(),
                label: tr!("Cancel").into_owned(),
                description: Some(tr!("Leave the instance unchanged").into_owned()),
                data: serde_json::json!({}),
            },
        ],
        Some("cancel".to_string()),
    );
    match read_machine_response(&envelope)? {
        choice if choice == "proceed" => Ok(()),
        _ => Err(orbit_core::OrbitError::Cancelled(
            tr!("Interaction cancelled by user").into_owned(),
        )),
    }
}

fn machine_interaction(
    command: &'static str,
    sequence: &std::sync::atomic::AtomicU64,
    id_prefix: &str,
    interaction: orbit_machine_protocol::InteractionKind,
    prompt: &str,
    choices: Vec<orbit_machine_protocol::InteractionChoice<serde_json::Value>>,
    default_choice: Option<String>,
) -> orbit_machine_protocol::InteractionEnvelope<serde_json::Value> {
    use std::sync::atomic::Ordering;
    let sequence = sequence.fetch_add(1, Ordering::Relaxed) + 1;
    let mut envelope = orbit_machine_protocol::InteractionEnvelope::new(
        command,
        sequence,
        format!("{id_prefix}-{sequence}"),
        interaction,
        prompt,
        choices,
    );
    envelope.default_choice = default_choice;
    let line = serde_json::to_string(&envelope).expect("interaction envelope is serializable");
    crate::cli::output::write_machine_line(&line);
    envelope
}

fn read_machine_response(
    request: &orbit_machine_protocol::InteractionEnvelope<serde_json::Value>,
) -> Result<String, orbit_core::OrbitError> {
    let mut input = String::new();
    let bytes_read = std::io::stdin().read_line(&mut input).map_err(|error| {
        interaction_failure(tr!(
            "Interaction response could not read stdin: %{error}",
            error = error
        ))
    })?;
    if bytes_read == 0 {
        return Err(orbit_core::OrbitError::Cancelled(
            tr!("Interaction was cancelled because stdin closed").into_owned(),
        ));
    }
    validate_machine_response(request, input.trim())
}

fn validate_machine_response(
    request: &orbit_machine_protocol::InteractionEnvelope<serde_json::Value>,
    input: &str,
) -> Result<String, orbit_core::OrbitError> {
    let response: orbit_machine_protocol::InteractionResponse = serde_json::from_str(input)
        .map_err(|error| {
            interaction_failure(tr!("Invalid interaction response: %{error}", error = error))
        })?;
    if response.schema_version != orbit_machine_protocol::SCHEMA_VERSION {
        return Err(interaction_failure(tr!(
            "Interaction response schema %{actual} does not match %{expected}",
            actual = response.schema_version,
            expected = orbit_machine_protocol::SCHEMA_VERSION
        )));
    }
    if response.kind != "interaction_response" {
        return Err(interaction_failure(tr!(
            "Interaction response has an invalid type"
        )));
    }
    if response.interaction_id != request.interaction_id {
        return Err(interaction_failure(tr!(
            "Interaction response does not match the pending request"
        )));
    }
    if response.cancelled {
        return Err(orbit_core::OrbitError::Cancelled(
            tr!("Interaction cancelled by user").into_owned(),
        ));
    }
    let selected = response
        .selected_choice
        .ok_or_else(|| interaction_failure(tr!("Interaction response did not select a choice")))?;
    request
        .choices
        .iter()
        .any(|choice| choice.id == selected)
        .then_some(selected.clone())
        .ok_or_else(|| {
            interaction_failure(tr!(
                "Interaction selected unknown choice '%{choice}'",
                choice = selected
            ))
        })
}

fn prompt_resolution(
    alternatives: &[orbit_core::ResolutionReport],
) -> Result<usize, orbit_core::OrbitError> {
    eprintln!("\n{}", tr!("Multiple dependency solutions are available:"));
    eprintln!("{}", crate::cli::output::resolution_choices(alternatives));

    loop {
        eprint!(
            "\n{}",
            tr!(
                "Choose a dependency solution [1-%{count}] (default 1): ",
                count = alternatives.len()
            )
        );
        use std::io::Write;
        std::io::stderr().flush().ok();
        let mut input = String::new();
        let Ok(bytes_read) = std::io::stdin().read_line(&mut input) else {
            return Err(interaction_failure(tr!(
                "Dependency solution selection could not read stdin"
            )));
        };
        if bytes_read == 0 {
            return Err(orbit_core::OrbitError::Cancelled(
                tr!("Dependency solution selection was cancelled because stdin closed")
                    .into_owned(),
            ));
        }
        if input.trim().is_empty() {
            return Ok(0);
        }
        if let Ok(choice) = input.trim().parse::<usize>()
            && (1..=alternatives.len()).contains(&choice)
        {
            return Ok(choice - 1);
        }
        eprintln!(
            "{}",
            tr!(
                "Please enter a number from 1 to %{count}.",
                count = alternatives.len()
            )
        );
    }
}

pub fn prompt_install_report(
    report: &orbit_core::InstallReport,
    yes: bool,
) -> Result<(), orbit_core::OrbitError> {
    if report.installed.is_empty() && report.removed.is_empty() && report.changes.is_empty() {
        return Ok(());
    }
    if !report.changes.is_empty() {
        eprintln!("\n{}", tr!("Planned package transaction:"));
        eprintln!(
            "{}",
            crate::cli::output::package_changes_table(&report.changes)
        );
    }
    if !report.installed.is_empty() {
        eprintln!("\n{}", tr!("Selected package contents:"));
        for m in &report.installed {
            eprintln!("  {} v{}", m.mod_id, m.version);
            let dependency_count = m
                .dependencies
                .iter()
                .flat_map(orbit_core::metadata::DependencyExpression::relations)
                .count();
            if dependency_count > 0 {
                eprintln!(
                    "      ↳ {}",
                    tr!("%{count} dependencies", count = dependency_count)
                );
            }
            let bundled_count = count_bundled_mods(&m.bundled);
            if bundled_count > 0 {
                eprintln!(
                    "      ↳ {}",
                    tr!("%{count} bundled module(s)", count = bundled_count)
                );
            }
        }
    }
    if report.changes.is_empty() && !report.removed.is_empty() {
        eprintln!(
            "\n{}",
            tr!("The following unselected package versions will be removed:")
        );
        eprintln!(
            "{}",
            crate::cli::output::removed_packages_table(&report.removed)
        );
    }
    if !report.already_satisfied.is_empty() {
        eprintln!(
            "\n{}",
            tr!(
                "Already satisfied: %{packages}",
                packages = report.already_satisfied.join(", ")
            )
        );
    }
    if yes {
        return Ok(());
    }
    eprint!("\n{}", tr!("Do you want to continue? [Y/n] "));
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(orbit_core::OrbitError::Io)?;
    let input = input.trim().to_lowercase();
    if input.is_empty() || input == "y" || input == "yes" {
        Ok(())
    } else {
        Err(orbit_core::OrbitError::Cancelled(
            tr!("Package transaction cancelled by user").into_owned(),
        ))
    }
}

fn interaction_failure(message: impl std::fmt::Display) -> orbit_core::OrbitError {
    orbit_core::OrbitError::Other(anyhow::anyhow!(message.to_string()))
}

fn count_bundled_mods(bundled: &[orbit_core::BundledMod]) -> usize {
    bundled
        .iter()
        .map(|module| 1 + count_bundled_mods(&module.bundled))
        .sum()
}

pub fn print_resolution_diagnostics(
    diagnostics: &[orbit_core::resolver::types::CandidateDiagnostic],
) {
    if !diagnostics.is_empty() {
        eprintln!("\n{}", tr!("Upgrade diagnostics:"));
        eprintln!("{}", crate::cli::output::diagnostics_table(diagnostics));
    }
}

pub fn print_resolution_warnings(warnings: &[String]) {
    if !warnings.is_empty() {
        eprintln!("\n{}", tr!("Dependency ordering warnings:"));
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
    if ctx.quiet {
        return;
    }
    use crate::cli::output::OutputFormat;
    match ctx.output.format {
        OutputFormat::Text => {
            print_resolution_diagnostics(&report.diagnostics);
            print_resolution_warnings(&report.warnings);
            if ctx.dry_run {
                println!("\n{}", tr!("%{command} preview:", command = command));
                println!(
                    "{}",
                    crate::cli::output::package_changes_table(&report.changes)
                );
                return;
            }
            if report.installed.is_empty() && report.removed.is_empty() {
                println!("{}", tr!("No new mods were installed."));
            } else {
                println!(
                    "\n{}",
                    tr!(
                        "Applied %{installed} selected package version(s) and removed %{removed} unselected package version(s).",
                        installed = report.installed.len(),
                        removed = report.removed.len()
                    )
                );
            }
        }
        OutputFormat::Json => {
            let view = crate::cli::output::transaction_view(report, ctx.dry_run);
            ctx.print_json(command, &view);
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
            Err(error) => {
                return Err(error).with_context(|| tr!("Failed to read orbit.toml").into_owned());
            }
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
            .packages
            .values()
            .flat_map(|package| package.remotes.iter())
        {
            let provider = remote.provider();
            if provider != "file" && !catalogs.iter().any(|item| item == provider) {
                catalogs.push(provider.to_string());
            }
        }
    }
    orbit_core::providers::create_providers(&catalogs, runtime.config())
        .with_context(|| tr!("Failed to create providers").into_owned())
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
        anyhow::bail!(
            "{}",
            tr!("A platform prefix must be followed by a project slug or ID")
        );
    }

    let requested_platform = requested_platform.map(normalize_platform);
    if let (Some(prefixed), Some(requested)) = (prefixed_platform, requested_platform)
        && prefixed != requested
    {
        anyhow::bail!(
            "{}",
            tr!(
                "'%{input}' selects %{prefixed}, but --platform selects %{requested}; use one platform",
                input = input,
                prefixed = prefixed,
                requested = requested
            )
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
                anyhow::anyhow!(
                    "{}",
                    tr!(
                        "CurseForge remotes require a numeric project ID, got '%{locator}'",
                        locator = locator
                    )
                )
            })?,
        }),
        other => anyhow::bail!(
            "{}",
            tr!("Unsupported package remote '%{remote}'", remote = other)
        ),
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{PackageChange, PackageChangeKind, ResolutionReport};
    use orbit_machine_protocol::{
        InteractionChoice, InteractionEnvelope, InteractionKind, InteractionResponse,
    };

    use super::{
        resolution_interaction_choices, resolution_package_change_view, resolve_platform_target,
        validate_machine_response,
    };

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

    #[test]
    fn machine_resolution_marks_only_non_common_actions_as_different() {
        let common = package_change("fabric-api", "1", "2");
        let choices = resolution_interaction_choices(&[
            ResolutionReport {
                changes: vec![common.clone(), package_change("sodium", "1", "2")],
                ..ResolutionReport::default()
            },
            ResolutionReport {
                changes: vec![common, package_change("lithium", "1", "2")],
                ..ResolutionReport::default()
            },
        ]);

        assert_eq!(choices.len(), 2);
        for choice in choices {
            let changes = choice.data["changes"].as_array().unwrap();
            let fabric = changes
                .iter()
                .find(|change| change["change"]["package"] == "fabric-api")
                .unwrap();
            assert_eq!(fabric["different"], false);
            assert_eq!(
                changes
                    .iter()
                    .filter(|change| change["different"] == true)
                    .count(),
                2
            );
            assert_eq!(
                changes
                    .iter()
                    .filter(|change| change["change"]["kind"] == "keep")
                    .count(),
                1
            );
            assert!(choice.data.get("warnings").is_none());
            assert!(choice.data.get("diagnostics").is_none());
        }
    }

    #[test]
    fn machine_resolution_keeps_equal_version_candidate_variants_distinct() {
        let change = package_change("voxy", "1", "2");
        let choices = resolution_interaction_choices(&[
            ResolutionReport {
                selected_candidates: std::collections::BTreeMap::from([(
                    "voxy".to_string(),
                    "sha512:first".to_string(),
                )]),
                changes: vec![change.clone()],
                ..ResolutionReport::default()
            },
            ResolutionReport {
                selected_candidates: std::collections::BTreeMap::from([(
                    "voxy".to_string(),
                    "sha512:second".to_string(),
                )]),
                changes: vec![change],
                ..ResolutionReport::default()
            },
        ]);

        assert!(choices.iter().all(|choice| {
            choice.data["changes"][0]["different"] == true
                && choice.data.to_string().find("sha512").is_none()
        }));
    }

    #[test]
    fn machine_resolution_exposes_only_the_selected_jar_basename() {
        let mut change = package_change("voxy", "1", "2");
        change.selected_filename = Some(r"C:\private\cache\voxy-fabric-2.jar".to_string());

        let view = resolution_package_change_view(&change);

        assert_eq!(view["selected_artifact"], "voxy-fabric-2.jar");
        assert!(!view.to_string().contains("private"));
        assert!(!view.to_string().contains("cache"));
    }

    #[test]
    fn machine_response_must_match_schema_request_and_choice() {
        let request = InteractionEnvelope::new(
            "add",
            4,
            "package-4",
            InteractionKind::Package,
            "Choose package",
            vec![InteractionChoice {
                id: "sodium".to_string(),
                label: "sodium".to_string(),
                description: None,
                data: serde_json::json!({}),
            }],
        );
        let valid =
            serde_json::to_string(&InteractionResponse::selected("package-4", "sodium")).unwrap();
        assert_eq!(
            validate_machine_response(&request, &valid).unwrap(),
            "sodium"
        );

        let wrong_request =
            serde_json::to_string(&InteractionResponse::selected("package-5", "sodium")).unwrap();
        assert!(validate_machine_response(&request, &wrong_request).is_err());

        let unknown_choice =
            serde_json::to_string(&InteractionResponse::selected("package-4", "lithium")).unwrap();
        assert!(validate_machine_response(&request, &unknown_choice).is_err());

        let cancelled =
            serde_json::to_string(&InteractionResponse::cancelled("package-4")).unwrap();
        assert!(validate_machine_response(&request, &cancelled).is_err());
    }

    fn package_change(package: &str, current: &str, selected: &str) -> PackageChange {
        PackageChange {
            package: package.to_string(),
            current_version: Some(current.to_string()),
            selected_version: Some(selected.to_string()),
            filename: Some(format!("{package}-old.jar")),
            selected_filename: Some(format!("{package}-new.jar")),
            selected_description: None,
            kind: PackageChangeKind::Upgrade,
        }
    }
}
