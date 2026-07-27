use anyhow::Result;

use super::CliContext;
use crate::cli::output::{DependencyEnvironmentOutput, OutputFormat};

pub fn handle(package: String, environment: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let report =
        orbit_core::set_dependency_environment(&instance_dir, &package, &environment, ctx.dry_run)?;
    let output = DependencyEnvironmentOutput {
        package: report.package,
        configured: report
            .configured
            .map(|environment| environment.as_str().to_string()),
        effective: report
            .effective
            .map(|environment| environment.as_str().to_string()),
        dry_run: report.dry_run,
    };
    match ctx.output.format {
        OutputFormat::Text => {
            let prefix = if output.dry_run { "[dry-run] " } else { "" };
            let configured = output.configured.as_deref().unwrap_or("auto");
            let effective = output
                .effective
                .as_deref()
                .unwrap_or("pending JAR selection");
            println!(
                "{prefix}{} env = {configured} (effective: {effective})",
                output.package
            );
        }
        OutputFormat::Json => crate::cli::output::print_json("env", &output),
    }
    Ok(())
}
