use anyhow::Result;

use super::{CliContext, confirm_data_reset};
use crate::cli::output::{OutputFormat, ResetOutput, owned_path_view};

pub async fn handle(package: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let plan = orbit_core::plan_data_reset(&instance_dir, &package, ctx.dry_run)?;
    confirm_data_reset(ctx, &plan)?;
    let report = orbit_core::apply_data_reset(&instance_dir, &plan, ctx.dry_run)?;

    match ctx.output.format {
        OutputFormat::Text => {
            let message = if ctx.dry_run {
                tr!(
                    "[dry-run] would reset %{count} runtime-owned path(s) for '%{package}'.",
                    count = report.removed.len(),
                    package = report.mod_id
                )
            } else {
                tr!(
                    "Reset %{count} runtime-owned path(s) for '%{package}'; the package remains installed.",
                    count = report.removed.len(),
                    package = report.mod_id
                )
            };
            ctx.print_result_line(format_args!("{message}"));
            for warning in &report.warnings {
                ctx.print_information_line(format_args!(
                    "{}",
                    tr!("Warning: %{warning}", warning = warning)
                ));
            }
        }
        OutputFormat::Json => ctx.print_json(
            "reset",
            &ResetOutput {
                mod_id: report.mod_id,
                data_removed: report.removed.iter().map(owned_path_view).collect(),
                warnings: report.warnings,
            },
        ),
    }
    Ok(())
}
