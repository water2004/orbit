use super::CliContext;
use anyhow::{Context, Result};

use crate::cli::output::{OutputFormat, PurgeOutput};

pub async fn handle(mod_name: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let lock = orbit_core::Lockfile::open(&instance_dir)
        .with_context(|| tr!("Failed to read orbit.lock").into_owned())?;
    let entry = lock.find_entry(&mod_name).ok_or_else(|| {
        anyhow::anyhow!(
            "{}",
            tr!("Package '%{package}' is not installed", package = mod_name)
        )
    })?;
    let mod_id = entry.mod_id.clone();
    let config_dir = instance_dir.join("config");
    let candidates = orbit_core::find_config_candidates(&mod_id, None, &config_dir)?;
    let selected = select_candidates(&candidates, ctx)?;

    let removed = orbit_core::remove_from_instance(&mod_id, &instance_dir, ctx.dry_run)?;
    if ctx.dry_run {
        match ctx.output.format {
            OutputFormat::Text => {
                println!(
                    "{}",
                    tr!(
                        "[dry-run] would purge '%{package}' and %{configs} config file(s).",
                        package = removed.mod_id,
                        configs = selected.len()
                    )
                );
            }
            OutputFormat::Json => {
                crate::cli::output::print_json(
                    "purge",
                    &PurgeOutput {
                        mod_id: removed.mod_id,
                        jar_deleted: removed.jar_deleted,
                        configs_removed: selected.iter().map(|c| c.path.clone()).collect(),
                    },
                );
            }
        }
        return Ok(());
    }
    let removed_configs = orbit_core::remove_config_candidates(&config_dir, &selected)?;
    match ctx.output.format {
        OutputFormat::Text => {
            println!(
                "{}",
                tr!(
                    "Purged %{package}: removed %{files} package file set(s) and %{configs} config file(s).",
                    package = removed.mod_id,
                    files = usize::from(removed.jar_deleted),
                    configs = removed_configs.len()
                )
            );
        }
        OutputFormat::Json => {
            crate::cli::output::print_json(
                "purge",
                &PurgeOutput {
                    mod_id: removed.mod_id,
                    jar_deleted: removed.jar_deleted,
                    configs_removed: removed_configs,
                },
            );
        }
    }
    Ok(())
}

fn select_candidates(
    candidates: &[orbit_core::CandidateConfig],
    ctx: &CliContext,
) -> Result<Vec<orbit_core::CandidateConfig>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    eprintln!(
        "{}",
        tr!(
            "Found %{count} candidate config file(s):",
            count = candidates.len()
        )
    );
    let mut selected = Vec::new();
    for candidate in candidates {
        if ctx.yes || ctx.dry_run {
            eprintln!("  {} ({})", candidate.path, candidate.reason);
            selected.push(candidate.clone());
            continue;
        }
        eprint!(
            "  {}",
            tr!(
                "%{path} [%{reason}] remove? [y/N] ",
                path = candidate.path,
                reason = candidate.reason
            )
        );
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            selected.push(candidate.clone());
        }
    }
    Ok(selected)
}
