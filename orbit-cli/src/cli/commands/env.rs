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
            let prefix = if output.dry_run {
                tr!("[dry-run] ")
            } else {
                tr!("")
            };
            let configured = output
                .configured
                .clone()
                .unwrap_or_else(|| tr!("auto").into_owned());
            let effective = output
                .effective
                .clone()
                .unwrap_or_else(|| tr!("pending JAR selection").into_owned());
            println!(
                "{}",
                tr!(
                    "%{prefix}%{package} env = %{configured} (effective: %{effective})",
                    prefix = prefix,
                    package = output.package,
                    configured = tr!(&configured),
                    effective = tr!(&effective)
                )
            );
        }
        OutputFormat::Json => crate::cli::output::print_json("env", &output),
    }
    Ok(())
}
