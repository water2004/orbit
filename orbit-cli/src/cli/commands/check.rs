use super::CliContext;
use anyhow::{Context, Result};

use crate::cli::output::{CheckOutput, CheckSummary, OutputFormat, check_result_view};

pub async fn handle(version: String, modloader: Option<String>, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let manifest = orbit_core::ManifestFile::open(&instance_dir)
        .with_context(|| tr!("Failed to read orbit.toml").into_owned())?;
    let lockfile = orbit_core::Lockfile::open(&instance_dir)
        .with_context(|| tr!("Failed to read orbit.lock").into_owned())?;
    let loader = modloader.unwrap_or_else(|| manifest.inner.project.modloader.clone());
    let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;

    let results = orbit_core::check_compatibility_with_progress(
        &instance_dir,
        &lockfile.inner,
        &version,
        &loader,
        &providers,
        ctx.runtime.jar_cache(),
        super::operation_progress(ctx),
    )
    .await?;
    if results.is_empty() {
        if ctx.output.format == OutputFormat::Text {
            println!("{}", tr!("No online packages in orbit.lock to check."));
        } else {
            let view = CheckOutput {
                target_mc_version: version.clone(),
                target_loader: loader.clone(),
                summary: CheckSummary {
                    total: 0,
                    compatible: 0,
                    blocking: 0,
                },
                results: Vec::new(),
            };
            crate::cli::output::print_json("check", &view);
        }
        return Ok(());
    }

    let compatible = results.iter().filter(|result| result.compatible).count();
    let blocking = results.len() - compatible;
    let view = CheckOutput {
        target_mc_version: version.clone(),
        target_loader: loader.clone(),
        summary: CheckSummary {
            total: results.len(),
            compatible,
            blocking,
        },
        results: results.iter().map(check_result_view).collect(),
    };

    match ctx.output.format {
        OutputFormat::Text => {
            println!("{}", crate::cli::output::check_results_table(&results));
            println!(
                "\n{}",
                tr!(
                    "%{compatible} of %{total} mods are ready for Minecraft %{version}.",
                    compatible = compatible,
                    total = results.len(),
                    version = version
                )
            );
            let blockers: Vec<_> = results
                .iter()
                .filter(|result| !result.compatible)
                .map(|result| result.mod_name.as_str())
                .collect();
            if !blockers.is_empty() {
                println!(
                    "{}",
                    tr!(
                        "Blocking the upgrade: %{packages}.",
                        packages = blockers.join(", ")
                    )
                );
            }
        }
        OutputFormat::Json => {
            crate::cli::output::print_json("check", &view);
        }
    }
    Ok(())
}
