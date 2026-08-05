use anyhow::Result;

use super::CliContext;
use crate::cli::output::{OutputFormat, PackageActivationOutput};

pub fn handle(package: String, enabled: bool, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let report = orbit_core::set_package_activation(&instance_dir, &package, enabled, ctx.dry_run)?;
    let output = PackageActivationOutput {
        package: report.package,
        previous_enabled: report.previous_enabled,
        enabled: report.enabled,
        changed: report.changed,
        dry_run: report.dry_run,
    };
    match ctx.output.format {
        OutputFormat::Text => {
            let prefix = if output.dry_run {
                tr!("[dry-run] ")
            } else {
                tr!("")
            };
            let action = if output.enabled {
                tr!("enabled").into_owned()
            } else {
                tr!("disabled").into_owned()
            };
            let state = if output.changed {
                action
            } else {
                tr!("already %{state}", state = action)
            };
            ctx.print_result_line(format_args!(
                "{}",
                tr!(
                    "%{prefix}%{package} is %{state}.",
                    prefix = prefix,
                    package = output.package,
                    state = state
                )
            ));
        }
        OutputFormat::Json => ctx.print_json(if enabled { "enable" } else { "disable" }, &output),
    }
    Ok(())
}
