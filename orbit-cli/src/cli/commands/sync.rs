use super::CliContext;
use anyhow::Result;

use crate::cli::output::{OutputFormat, sync_view};

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let providers =
        orbit_core::providers::create_identification_providers(&ctx.runtime.config().auth)?;
    let report = orbit_core::sync_instance(&instance_dir, &providers, ctx.dry_run).await?;

    if ctx.output.format == OutputFormat::Text {
        super::print_resolution_warnings(&report.warnings);

        let deltas = crate::cli::output::sync_report_table(&report);
        if deltas != tr!("No local changes.") {
            println!("{deltas}");
        }
        println!(
            "{}",
            tr!(
                "Sync %{state}: %{platform} platform change(s), %{added} added, %{changed} changed, %{removed} removed from lock, %{missing} missing on disk.",
                state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                platform = report.platform_changes.len(),
                added = report.added.len(),
                changed = report.changed.len(),
                removed = report.removed.len(),
                missing = report.missing.len()
            )
        );
        if !report.missing.is_empty() {
            println!("{}", tr!("Run 'orbit fix' to resolve missing packages."));
        }
        return Ok(());
    }

    let view = sync_view(&report, ctx.dry_run);
    crate::cli::output::print_json("sync", &view);
    Ok(())
}
