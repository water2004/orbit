use super::CliContext;
use anyhow::Result;

use crate::cli::output::{OutputFormat, sync_view};

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let report =
        orbit_core::sync_instance(&instance_dir, ctx.dry_run, super::install_interaction(ctx))
            .await?;

    if ctx.output.format == OutputFormat::Text {
        super::print_resolution_diagnostics(&report.diagnostics);
        super::print_resolution_warnings(&report.warnings);

        let deltas = crate::cli::output::sync_report_table(&report);
        if deltas != tr!("No local changes.") {
            println!("{deltas}");
        }
        if !report.removed.is_empty() {
            println!("\n{}", tr!("Removed unselected package versions:"));
            println!(
                "{}",
                crate::cli::output::removed_packages_table(&report.removed)
            );
        }
        println!(
            "{}",
            tr!(
                "Sync %{state}: %{platform} platform change(s), %{added} added, %{changed} changed, %{removed} removed, %{missing} missing, %{unlocked} unlocked.",
                state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                platform = report.platform_changes.len(),
                added = report.added.len(),
                changed = report.changed.len(),
                removed = report.removed.len(),
                missing = report.missing.len(),
                unlocked = report.unlocked.len()
            )
        );
        if !report.missing.is_empty() || !report.unlocked.is_empty() {
            println!(
                "{}",
                tr!("Run 'orbit install' to restore missing or unlocked mods.")
            );
        }
        return Ok(());
    }

    let view = sync_view(&report, ctx.dry_run);
    crate::cli::output::print_json("sync", &view);
    Ok(())
}
