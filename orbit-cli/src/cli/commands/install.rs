use anyhow::Result;

use super::CliContext;
use crate::cli::output::{OutputFormat, install_instance_view};

/// `orbit install` — materialize only the exact content recorded by orbit.lock.
pub async fn handle(
    target: Option<String>,
    group: Option<String>,
    no_optional: bool,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
    let report = orbit_core::install_instance(
        &instance_dir,
        &providers,
        ctx.runtime.jar_cache(),
        orbit_core::InstanceInstallOptions {
            selection: orbit_core::PackageSelection {
                target,
                group,
                no_optional,
            },
            dry_run: ctx.dry_run,
        },
        super::operation_progress(ctx),
    )
    .await?;

    match ctx.output.format {
        OutputFormat::Text => {
            println!(
                "{}",
                tr!(
                    "Install %{state}: %{installed} exact package(s) materialized, %{present} already present, %{skipped} excluded by policy.",
                    state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                    installed = report.installed.len(),
                    present = report.already_present.len(),
                    skipped = report.skipped.len()
                )
            );
        }
        OutputFormat::Json => {
            let view = install_instance_view(&report, ctx.dry_run);
            crate::cli::output::print_json("install", &view);
        }
    }
    Ok(())
}
