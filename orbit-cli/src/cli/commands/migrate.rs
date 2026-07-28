use std::path::PathBuf;

use anyhow::Result;

use super::CliContext;
use crate::cli::output::{MigrationExportView, MigrationOutput, MigrationSummary, OutputFormat};

pub async fn handle_check(target: PathBuf, ctx: &CliContext) -> Result<()> {
    let plan = build_plan(target, ctx).await?;
    render_plan(&plan, "check", None, ctx);
    Ok(())
}

pub async fn handle_export(target: PathBuf, ctx: &CliContext) -> Result<()> {
    let plan = build_plan(target, ctx).await?;
    let preview = orbit_core::export_migration(&plan, true)?;
    let confirmed = ctx.dry_run || confirm_export(&plan, &preview, ctx);
    let report = if confirmed {
        orbit_core::export_migration(&plan, ctx.dry_run)?
    } else {
        preview
    };
    let applied = confirmed && !ctx.dry_run;

    render_plan(
        &plan,
        "export",
        Some(MigrationExportView {
            applied,
            config_files: report.config_files,
            config_bytes: report.config_bytes,
        }),
        ctx,
    );
    if ctx.output.format == OutputFormat::Text {
        if applied {
            println!(
                "{}",
                tr!(
                    "Migration exported to %{target}. Run 'orbit install' in the target instance to materialize %{packages} exact package(s).",
                    target = report.target_dir.display(),
                    packages = report.packages
                )
            );
        } else if ctx.dry_run {
            println!(
                "{}",
                tr!(
                    "Migration export preview: %{packages} package(s) and %{configs} configuration file(s).",
                    packages = report.packages,
                    configs = report.config_files
                )
            );
        } else {
            println!("{}", tr!("Migration export cancelled."));
        }
    }
    Ok(())
}

async fn build_plan(target: PathBuf, ctx: &CliContext) -> Result<orbit_core::MigrationPlan> {
    let source = ctx.instance_dir()?;
    let providers = super::create_instance_providers(&source, None, &ctx.runtime)?;
    Ok(orbit_core::plan_migration(
        &source,
        &target,
        &providers,
        ctx.runtime.jar_cache(),
        orbit_core::MigrationInteraction {
            select_resolution: super::resolution_selector(ctx),
            progress: super::operation_progress(ctx),
        },
    )
    .await?)
}

fn render_plan(
    plan: &orbit_core::MigrationPlan,
    subcommand: &str,
    export: Option<MigrationExportView>,
    ctx: &CliContext,
) {
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
            crate::cli::output::print_json(
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
) -> bool {
    if ctx.yes {
        return true;
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
                        "Write Orbit state and %{configs} configuration file(s)",
                        configs = preview.config_files
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
        return super::read_machine_response(&envelope).is_ok_and(|choice| choice == "proceed");
    }

    eprintln!(
        "{}",
        crate::cli::output::package_changes_table(&plan.changes)
    );
    eprint!(
        "\n{}",
        tr!(
            "Export this migration and %{configs} configuration file(s) to %{target}? [y/N] ",
            configs = preview.config_files,
            target = preview.target_dir.display()
        )
    );
    use std::io::Write;
    std::io::stderr().flush().ok();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
