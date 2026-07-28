pub mod add;
pub mod audit;
pub mod cache;
pub mod check;
pub mod config;
pub mod env;
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

pub use env::handle as handle_env;

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
    /// 输出格式与进度协议，由全局 `--format` / `--progress-format` 决定。
    pub output: crate::cli::output::OutputCfg,
}

impl CliContext {
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

pub use add::handle as handle_add;
pub use audit::handle as handle_audit;
pub use check::handle as handle_check;
pub use config::handle as handle_config;
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
    crate::cli::progress::reporter(ctx.quiet, &ctx.runtime.config().ui.progress_bar)
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

fn prompt_package(packages: &[String]) -> Result<usize, String> {
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
            return Err(tr!("Package selection could not read stdin").into_owned());
        };
        if bytes_read == 0 {
            return Err(tr!("Package selection was cancelled because stdin closed").into_owned());
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
        return Box::new(|report| prompt_install_report(report, true));
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
) -> Result<usize, String> {
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
            tr!(
                "Package interaction selected unknown choice '%{choice}'",
                choice = selected
            )
        })
}

fn machine_select_resolution(
    command: &'static str,
    sequence: &std::sync::atomic::AtomicU64,
    alternatives: &[orbit_core::ResolutionReport],
) -> Result<usize, String> {
    use orbit_machine_protocol::{InteractionEnvelope, InteractionKind};
    let choices = resolution_interaction_choices(alternatives);
    let envelope: InteractionEnvelope<serde_json::Value> = machine_interaction(
        command,
        sequence,
        "resolution",
        InteractionKind::Resolution,
        &tr!("Choose one Pareto-maximal dependency solution"),
        choices,
        Some("1".to_string()),
    );
    read_machine_response(&envelope)?
        .parse::<usize>()
        .ok()
        .and_then(|selected| selected.checked_sub(1))
        .filter(|selected| *selected < alternatives.len())
        .ok_or_else(|| tr!("Resolution interaction selected an unknown choice").into_owned())
}

fn resolution_interaction_choices(
    alternatives: &[orbit_core::ResolutionReport],
) -> Vec<orbit_machine_protocol::InteractionChoice<serde_json::Value>> {
    use orbit_machine_protocol::InteractionChoice;
    let signatures: Vec<Vec<String>> = alternatives
        .iter()
        .map(|alternative| {
            alternative
                .changes
                .iter()
                .map(|change| {
                    serde_json::to_string(&crate::cli::output::package_change_view(change))
                        .expect("package change view is serializable")
                })
                .collect()
        })
        .collect();
    alternatives
        .iter()
        .enumerate()
        .map(|(index, alternative)| {
            let changes = alternative
                .changes
                .iter()
                .enumerate()
                .map(|(change_index, change)| {
                    let signature = &signatures[index][change_index];
                    let common = signatures
                        .iter()
                        .all(|candidate| candidate.iter().any(|item| item == signature));
                    serde_json::json!({
                        "different": !common,
                        "change": crate::cli::output::package_change_view(change),
                    })
                })
                .collect::<Vec<_>>();
            InteractionChoice {
                id: (index + 1).to_string(),
                label: tr!("Option %{number}", number = index + 1),
                description: Some(tr!(
                    "%{count} logical package action(s)",
                    count = changes.len()
                )),
                data: serde_json::json!({
                    "changes": changes,
                    "warnings": alternative.warnings,
                    "diagnostics": alternative
                        .diagnostics
                        .iter()
                        .map(crate::cli::output::diagnostic_view)
                        .collect::<Vec<_>>(),
                }),
            }
        })
        .collect()
}

fn machine_confirm_install(
    command: &'static str,
    sequence: &std::sync::atomic::AtomicU64,
    report: &orbit_core::InstallReport,
) -> bool {
    use orbit_machine_protocol::{InteractionChoice, InteractionKind};
    if report.installed.is_empty() && report.removed.is_empty() && report.changes.is_empty() {
        return true;
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
    read_machine_response(&envelope).is_ok_and(|choice| choice == "proceed")
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
) -> Result<String, String> {
    let mut input = String::new();
    let bytes_read = std::io::stdin().read_line(&mut input).map_err(|error| {
        tr!(
            "Interaction response could not read stdin: %{error}",
            error = error
        )
    })?;
    if bytes_read == 0 {
        return Err(tr!("Interaction was cancelled because stdin closed").into_owned());
    }
    validate_machine_response(request, input.trim())
}

fn validate_machine_response(
    request: &orbit_machine_protocol::InteractionEnvelope<serde_json::Value>,
    input: &str,
) -> Result<String, String> {
    let response: orbit_machine_protocol::InteractionResponse = serde_json::from_str(input)
        .map_err(|error| tr!("Invalid interaction response: %{error}", error = error))?;
    if response.schema_version != orbit_machine_protocol::SCHEMA_VERSION {
        return Err(tr!(
            "Interaction response schema %{actual} does not match %{expected}",
            actual = response.schema_version,
            expected = orbit_machine_protocol::SCHEMA_VERSION
        ));
    }
    if response.kind != "interaction_response" {
        return Err(tr!("Interaction response has an invalid type").into_owned());
    }
    if response.interaction_id != request.interaction_id {
        return Err(tr!("Interaction response does not match the pending request").into_owned());
    }
    if response.cancelled {
        return Err(tr!("Interaction cancelled by user").into_owned());
    }
    let selected = response
        .selected_choice
        .ok_or_else(|| tr!("Interaction response did not select a choice").into_owned())?;
    request
        .choices
        .iter()
        .any(|choice| choice.id == selected)
        .then_some(selected.clone())
        .ok_or_else(|| {
            tr!(
                "Interaction selected unknown choice '%{choice}'",
                choice = selected
            )
        })
}

fn prompt_resolution(alternatives: &[orbit_core::ResolutionReport]) -> Result<usize, String> {
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
            return Err(tr!("Dependency solution selection could not read stdin").into_owned());
        };
        if bytes_read == 0 {
            return Err(
                tr!("Dependency solution selection was cancelled because stdin closed")
                    .into_owned(),
            );
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

pub fn prompt_install_report(report: &orbit_core::InstallReport, yes: bool) -> bool {
    if report.installed.is_empty() && report.removed.is_empty() && report.changes.is_empty() {
        return true;
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
        return true;
    }
    eprint!("\n{}", tr!("Do you want to continue? [Y/n] "));
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
        "      ↳ {indent}[{}] {} {}",
        tr!("bundled"),
        bundled.mod_id,
        bundled.version
    );
    for child in &bundled.bundled {
        print_bundled_mod(child, depth + 1);
    }
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
        resolution_interaction_choices, resolve_platform_target, validate_machine_response,
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
                1
            );
        }
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
