use super::CliContext;
use anyhow::Result;

use crate::cli::output::{OutputFormat, sync_view};

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let providers = orbit_core::providers::create_identification_providers(ctx.runtime.config())?;
    let report = orbit_core::sync_instance(&instance_dir, &providers, ctx.dry_run).await?;

    if ctx.output.format == OutputFormat::Text {
        super::print_resolution_warnings(&report.warnings);

        let deltas = crate::cli::output::sync_report_table(&report);
        if deltas != tr!("No local changes.") {
            ctx.print_result_line(format_args!("{deltas}"));
        }
        ctx.print_result_line(format_args!(
            "{}",
            tr!(
                "Sync %{state}: %{platform} platform change(s), %{added} added, %{changed} changed, %{removed} removed.",
                state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                platform = report.platform_changes.len(),
                added = report.added.len(),
                changed = report.changed.len(),
                removed = report.removed.len()
            )
        ));
        return Ok(());
    }

    let view = sync_view(&report, ctx.dry_run);
    ctx.print_json("sync", &view);
    Ok(())
}
