use anyhow::Result;

use super::{CliContext, confirm_data_purge};
use crate::cli::output::{OutputFormat, PurgeOutput, owned_path_view};

pub async fn handle(package: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let plan = orbit_core::plan_data_purge(&instance_dir, &package, ctx.dry_run)?;
    confirm_data_purge(ctx, &plan)?;
    let report = orbit_core::apply_data_purge(&instance_dir, &plan, ctx.dry_run)?;

    match ctx.output.format {
        OutputFormat::Text => {
            if ctx.dry_run {
                ctx.print_result_line(format_args!(
                    "{}",
                    tr!(
                        "[dry-run] would remove '%{package}' and %{count} runtime-owned path(s).",
                        package = report.mod_id,
                        count = report.removed.len()
                    )
                ));
            } else {
                ctx.print_result_line(format_args!(
                    "{}",
                    tr!(
                        "Purged '%{package}' and %{count} runtime-owned path(s).",
                        package = report.mod_id,
                        count = report.removed.len()
                    )
                ));
            }
        }
        OutputFormat::Json => ctx.print_json(
            "purge",
            &PurgeOutput {
                mod_id: report.mod_id,
                jar_deleted: report.jar_deleted,
                data_removed: report.removed.iter().map(owned_path_view).collect(),
            },
        ),
    }
    Ok(())
}
