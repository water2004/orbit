use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::ManifestFile;

use crate::cli::output::{
    DiagnosticView, OutdatedOutput, OutdatedSummary, OutputFormat, diagnostic_view,
    no_upgrade_message, outdated_mod_view,
};

pub async fn handle(mod_name: Option<String>, ctx: &CliContext) -> Result<()> {
    let dir = ctx.instance_dir()?;
    let manifest_file =
        ManifestFile::open(&dir).with_context(|| tr!("Failed to read orbit.toml").into_owned())?;
    let lock = orbit_core::workspace::Lockfile::open(&dir)
        .with_context(|| tr!("Failed to read orbit.lock").into_owned())?;
    let requested_package = mod_name
        .as_deref()
        .map(|name| -> Result<String> {
            let entry = lock.find_entry(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}",
                    tr!("'%{name}' was not found in orbit.lock", name = name)
                )
            })?;
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
            ctx.runtime.candidate_storage(),
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
            ctx.runtime.candidate_storage(),
            progress,
        )
        .await
    }
    .with_context(|| tr!("Failed to check for updates").into_owned())?;
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
    if ctx.output.format == OutputFormat::Text && !ctx.quiet {
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
                ctx.print_result_line(format_args!(
                    "{}",
                    no_upgrade_message(requested_package.as_deref(), !diagnostics.is_empty())
                ));
            }
            OutputFormat::Json => {
                ctx.print_json("outdated", &view);
            }
        }
        return Ok(());
    }

    match ctx.output.format {
        OutputFormat::Text => {
            ctx.print_result_line(format_args!("\n{}", tr!("Updates available:")));
            ctx.print_result_line(format_args!(
                "{}",
                crate::cli::output::outdated_table(&results)
            ));
        }
        OutputFormat::Json => {
            ctx.print_json("outdated", &view);
        }
    }
    Ok(())
}
