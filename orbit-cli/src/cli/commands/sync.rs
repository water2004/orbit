use super::CliContext;
use anyhow::Result;

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let report =
        orbit_core::sync_instance(&instance_dir, ctx.dry_run, super::install_interaction(ctx))
            .await?;
    super::print_resolution_diagnostics(&report.diagnostics);
    super::print_resolution_warnings(&report.warnings);

    let deltas = crate::cli::output::sync_report_table(&report);
    if deltas != "No local changes." {
        println!("{deltas}");
    }
    if !report.removed.is_empty() {
        println!("\nRemoved unselected package versions:");
        println!(
            "{}",
            crate::cli::output::removed_packages_table(&report.removed)
        );
    }
    println!(
        "Sync {}: {} platform, {} added, {} changed, {} removed, {} missing, {} unlocked.",
        if ctx.dry_run { "preview" } else { "complete" },
        report.platform_changes.len(),
        report.added.len(),
        report.changed.len(),
        report.removed.len(),
        report.missing.len(),
        report.unlocked.len()
    );
    if !report.missing.is_empty() || !report.unlocked.is_empty() {
        println!("Run 'orbit install' to restore missing or unlocked mods.");
    }
    Ok(())
}
