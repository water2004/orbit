use anyhow::{Context, Result};

use super::CliContext;
use crate::cli::output::{
    OutputFormat, OwnershipOutput, owned_artifact_view, owned_path_view, package_ownership_table,
};

pub fn handle(package: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let ownership = orbit_core::package_ownership(&instance_dir, &package)
        .with_context(|| tr!("Failed to inspect package ownership").into_owned())?;

    match ctx.output.format {
        OutputFormat::Text => {
            ctx.print_result_line(format_args!("{}", package_ownership_table(&ownership)));
        }
        OutputFormat::Json => ctx.print_json(
            "ownership",
            &OwnershipOutput {
                mod_id: ownership.mod_id,
                artifacts: ownership
                    .artifacts
                    .iter()
                    .map(owned_artifact_view)
                    .collect(),
                data: ownership.data.iter().map(owned_path_view).collect(),
            },
        ),
    }
    Ok(())
}
