use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::ManifestFile;

use crate::cli::output::{
    DiagnosticView, OutdatedOutput, OutdatedSummary, OutputFormat, diagnostic_view,
    no_upgrade_message, outdated_mod_view,
};

pub async fn handle(mod_name: Option<String>, ctx: &CliContext) -> Result<()> {
    let dir = ctx.instance_dir()?;
    let manifest_file = ManifestFile::open(&dir).context("failed to read orbit.toml")?;
    let lock = orbit_core::workspace::Lockfile::open(&dir).context("failed to read orbit.lock")?;
    let requested_package = mod_name
        .as_deref()
        .map(|name| -> Result<String> {
            let entry = lock
                .find_entry(name)
                .ok_or_else(|| anyhow::anyhow!("'{name}' was not found in orbit.lock"))?;
            Ok(entry.mod_id.clone())
        })
        .transpose()?;

    let providers = super::create_instance_providers(&dir, None, &ctx.runtime)?;

    let selector = super::resolution_selector(ctx);
    let progress = super::operation_progress(ctx);
    let report = if requested_package.is_some() {
        orbit_core::outdated::check_outdated_with_interaction(
            &dir,
            &manifest_file.inner,
            &lock.inner,
            &providers,
            ctx.runtime.jar_cache(),
            orbit_core::outdated::OutdatedInteraction {
                package: requested_package.clone(),
                select_resolution: selector,
                progress,
            },
        )
        .await
    } else {
        orbit_core::outdated::check_all_outdated_with_progress(
            &dir,
            &manifest_file.inner,
            &lock.inner,
            &providers,
            selector,
            ctx.runtime.jar_cache(),
            progress,
        )
        .await
    }
    .context("failed to check for updates")?;
    let diagnostics: Vec<DiagnosticView> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            requested_package
                .as_deref()
                .is_none_or(|package| diagnostic.package == package)
        })
        .map(diagnostic_view)
        .collect();
    if ctx.output.format == OutputFormat::Text {
        super::print_resolution_diagnostics(&report.diagnostics);
        super::print_resolution_warnings(&report.warnings);
    }
    let results: Vec<orbit_core::OutdatedMod> = report
        .updates
        .iter()
        .filter(|outdated| {
            requested_package
                .as_deref()
                .is_none_or(|package| outdated.mod_id == package)
        })
        .cloned()
        .collect();

    let summary = OutdatedSummary {
        upgrades: results.len(),
        up_to_date: if results.is_empty() && report.diagnostics.is_empty() {
            1
        } else {
            0
        },
    };
    let view = OutdatedOutput {
        package: requested_package.clone(),
        summary,
        updates: results.iter().map(outdated_mod_view).collect(),
        diagnostics: diagnostics.clone(),
        warnings: report.warnings.clone(),
    };

    if results.is_empty() {
        match ctx.output.format {
            OutputFormat::Text => {
                println!(
                    "{}",
                    no_upgrade_message(requested_package.as_deref(), !diagnostics.is_empty())
                );
            }
            OutputFormat::Json => {
                crate::cli::output::print_json("outdated", &view);
            }
        }
        return Ok(());
    }

    match ctx.output.format {
        OutputFormat::Text => {
            println!("\nUpdates available:");
            println!("{}", crate::cli::output::outdated_table(&results));
        }
        OutputFormat::Json => {
            crate::cli::output::print_json("outdated", &view);
        }
    }
    Ok(())
}
