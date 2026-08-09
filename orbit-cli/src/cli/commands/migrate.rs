use std::path::PathBuf;

use anyhow::Result;

use super::CliContext;
use crate::cli::output::{MigrationExportView, MigrationOutput, MigrationSummary, OutputFormat};

pub async fn handle_check(
    target: PathBuf,
    source_pack: Option<PathBuf>,
    allow_removals: bool,
    ctx: &CliContext,
) -> Result<()> {
    let plan = build_plan(target, source_pack.as_deref(), allow_removals, ctx).await?;
    render_plan(&plan, "check", None, ctx);
    Ok(())
}

pub async fn handle_export(
    target: PathBuf,
    source_pack: Option<PathBuf>,
    consume_source_pack: bool,
    allow_removals: bool,
    ctx: &CliContext,
) -> Result<()> {
    let plan = build_plan(target, source_pack.as_deref(), allow_removals, ctx).await?;
    let preview = orbit_core::export_migration(&plan, true)?;
    if !ctx.dry_run {
        confirm_export(&plan, &preview, ctx)?;
    }
    let report = orbit_core::export_migration(&plan, ctx.dry_run)?;
    let applied = !ctx.dry_run;
    if applied
        && consume_source_pack
        && let Some(source_pack) = source_pack.as_deref()
    {
        orbit_core::consume_portable_instance(source_pack)?;
    }

    render_plan(
        &plan,
        "export",
        Some(MigrationExportView {
            applied,
            state_files: report.state_files,
            state_bytes: report.state_bytes,
        }),
        ctx,
    );
    if ctx.output.format == OutputFormat::Text {
        if applied {
            ctx.print_result_line(format_args!(
                "{}",
                tr!(
                    "Migration exported to %{target}. Run 'orbit install' in the target instance to materialize %{packages} exact package(s).",
                    target = report.target_dir.display(),
                    packages = report.packages
                )
            ));
        } else if ctx.dry_run {
            ctx.print_result_line(format_args!(
                "{}",
                tr!(
                    "Migration export preview: %{packages} package(s) and %{files} package state file(s).",
                    packages = report.packages,
                    files = report.state_files
                )
            ));
        }
    }
    Ok(())
}

async fn build_plan(
    target: PathBuf,
    source_pack: Option<&std::path::Path>,
    allow_removals: bool,
    ctx: &CliContext,
) -> Result<orbit_core::MigrationPlan> {
    let interaction = orbit_core::MigrationInteraction {
        select_resolution: super::resolution_selector(ctx),
        confirm_soft_fallback: (!allow_removals).then(|| soft_fallback_confirmation(ctx)),
        progress: super::operation_progress(ctx),
    };
    let options = orbit_core::MigrationOptions {
        allow_package_removals: allow_removals,
    };
    if let Some(source_pack) = source_pack {
        let source = orbit_core::extract_portable_instance(source_pack)?;
        let providers = super::create_instance_providers(source.path(), None, &ctx.runtime)?;
        return Ok(orbit_core::plan_migration_from_portable(
            source,
            &target,
            &providers,
            ctx.runtime.candidate_storage(),
            options,
            interaction,
        )
        .await?);
    }
    let source = ctx.instance_dir()?;
    let providers = super::create_instance_providers(&source, None, &ctx.runtime)?;
    Ok(orbit_core::plan_migration(
        &source,
        &target,
        &providers,
        ctx.runtime.candidate_storage(),
        options,
        interaction,
    )
    .await?)
}

fn soft_fallback_confirmation(ctx: &CliContext) -> orbit_core::MigrationFallbackConfirmation {
    if ctx.output.format == OutputFormat::Json {
        let command = ctx.command;
        let sequence = ctx.machine_sequence.clone();
        return Box::new(move |preview| {
            use orbit_machine_protocol::{InteractionChoice, InteractionKind};
            let prompt = format!(
                "{}\n\n{}",
                tr!("The complete source package set is unavailable for the target runtime:"),
                preview.strict_failure
            );
            let envelope = super::machine_interaction(
                command,
                &sequence,
                "migration_removals",
                InteractionKind::Confirmation,
                &prompt,
                vec![
                    InteractionChoice {
                        id: "proceed".to_string(),
                        label: tr!("Search removable-package solutions").into_owned(),
                        description: Some(
                            tr!("Find the Pareto-minimal package-removal solutions").into_owned(),
                        ),
                        data: serde_json::json!({}),
                    },
                    InteractionChoice {
                        id: "cancel".to_string(),
                        label: tr!("Cancel migration").into_owned(),
                        description: Some(tr!("Keep every source package required").into_owned()),
                        data: serde_json::json!({}),
                    },
                ],
                Some("cancel".to_string()),
            );
            match super::read_machine_response(&envelope)? {
                choice if choice == "proceed" => Ok(()),
                _ => Err(orbit_core::OrbitError::Cancelled(
                    tr!("Migration cancelled by user").into_owned(),
                )),
            }
        });
    }

    Box::new(|preview| {
        eprintln!("\n{}", tr!("Strict migration is unavailable:"));
        eprintln!("{}", preview.strict_failure);
        eprint!(
            "\n{}",
            tr!("Search for Pareto-minimal package-removal solutions? [y/N] ")
        );
        use std::io::Write;
        std::io::stderr().flush().ok();
        let mut input = String::new();
        let bytes = std::io::stdin()
            .read_line(&mut input)
            .map_err(orbit_core::OrbitError::Io)?;
        if bytes == 0 {
            return Err(orbit_core::OrbitError::Cancelled(
                tr!("Migration cancelled because stdin closed").into_owned(),
            ));
        }
        if matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            Ok(())
        } else {
            Err(orbit_core::OrbitError::Cancelled(
                tr!("Migration cancelled by user").into_owned(),
            ))
        }
    })
}

fn render_plan(
    plan: &orbit_core::MigrationPlan,
    subcommand: &str,
    export: Option<MigrationExportView>,
    ctx: &CliContext,
) {
    if ctx.quiet {
        return;
    }
    match ctx.output.format {
        OutputFormat::Text => {
            super::print_resolution_diagnostics(&plan.diagnostics);
            super::print_resolution_warnings(&plan.warnings);
            if plan.changes.is_empty() {
                println!("{}", tr!("The target package set is unchanged."));
            } else {
                println!(
                    "{}",
                    crate::cli::output::package_changes_table(&plan.changes)
                );
            }
            println!(
                "{}",
                tr!(
                    "Migration plan: Minecraft %{source} -> %{target}, %{loader} %{loader_version}, %{packages} selected package(s).",
                    source = plan.source_mc_version,
                    target = plan.target_mc_version,
                    loader = plan.target_loader,
                    loader_version = plan.target_loader_version,
                    packages = plan.selected_packages
                )
            );
        }
        OutputFormat::Json => {
            ctx.print_json(
                "migrate",
                &migration_view(plan, subcommand, export, ctx.dry_run),
            );
        }
    }
}

fn migration_view(
    plan: &orbit_core::MigrationPlan,
    subcommand: &str,
    export: Option<MigrationExportView>,
    dry_run: bool,
) -> MigrationOutput {
    let mut summary = MigrationSummary {
        selected_packages: plan.selected_packages,
        installs: 0,
        upgrades: 0,
        downgrades: 0,
        replacements: 0,
        removals: 0,
    };
    for change in &plan.changes {
        match change.kind {
            orbit_core::PackageChangeKind::Install => summary.installs += 1,
            orbit_core::PackageChangeKind::Upgrade => summary.upgrades += 1,
            orbit_core::PackageChangeKind::Downgrade => summary.downgrades += 1,
            orbit_core::PackageChangeKind::Replace => summary.replacements += 1,
            orbit_core::PackageChangeKind::Remove => summary.removals += 1,
        }
    }
    MigrationOutput {
        subcommand: subcommand.to_string(),
        dry_run,
        target_directory: plan.target_dir().to_string_lossy().into_owned(),
        source_mc_version: plan.source_mc_version.clone(),
        target_mc_version: plan.target_mc_version.clone(),
        target_loader: plan.target_loader.clone(),
        target_loader_version: plan.target_loader_version.clone(),
        summary,
        changes: plan
            .changes
            .iter()
            .map(crate::cli::output::package_change_view)
            .collect(),
        diagnostics: plan
            .diagnostics
            .iter()
            .map(crate::cli::output::diagnostic_view)
            .collect(),
        warnings: plan.warnings.clone(),
        export,
    }
}

fn confirm_export(
    plan: &orbit_core::MigrationPlan,
    preview: &orbit_core::MigrationExportReport,
    ctx: &CliContext,
) -> Result<(), orbit_core::OrbitError> {
    if ctx.yes {
        return Ok(());
    }
    if ctx.output.format == OutputFormat::Json {
        use orbit_machine_protocol::{InteractionChoice, InteractionKind};
        let envelope = super::machine_interaction(
            "migrate",
            &ctx.machine_sequence,
            "confirmation",
            InteractionKind::Confirmation,
            &tr!("Export the selected migration into the target instance"),
            vec![
                InteractionChoice {
                    id: "proceed".to_string(),
                    label: tr!("Export migration").into_owned(),
                    description: Some(tr!(
                        "Write Orbit state and %{files} package state file(s)",
                        files = preview.state_files
                    )),
                    data: serde_json::to_value(migration_view(plan, "export", None, false))
                        .expect("migration view is serializable"),
                },
                InteractionChoice {
                    id: "cancel".to_string(),
                    label: tr!("Cancel").into_owned(),
                    description: Some(tr!("Leave the target instance unchanged").into_owned()),
                    data: serde_json::json!({}),
                },
            ],
            Some("cancel".to_string()),
        );
        return match super::read_machine_response(&envelope)? {
            choice if choice == "proceed" => Ok(()),
            _ => Err(orbit_core::OrbitError::Cancelled(
                tr!("Migration export cancelled by user").into_owned(),
            )),
        };
    }

    eprintln!(
        "{}",
        crate::cli::output::package_changes_table(&plan.changes)
    );
    eprint!(
        "\n{}",
        tr!(
            "Export this migration and %{files} package state file(s) to %{target}? [y/N] ",
            files = preview.state_files,
            target = preview.target_dir.display()
        )
    );
    use std::io::Write;
    std::io::stderr().flush().ok();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(orbit_core::OrbitError::Io)?;
    if matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(orbit_core::OrbitError::Cancelled(
            tr!("Migration export cancelled by user").into_owned(),
        ))
    }
}
