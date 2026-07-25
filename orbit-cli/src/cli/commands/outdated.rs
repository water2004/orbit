use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::ManifestFile;

pub async fn handle(mod_name: Option<String>, ctx: &CliContext) -> Result<()> {
    let dir = ctx.instance_dir()?;
    let manifest_file = ManifestFile::open(&dir).context("failed to read orbit.toml")?;
    let lock = orbit_core::workspace::Lockfile::open(&dir).context("failed to read orbit.lock")?;
    let requested_package = mod_name
        .as_deref()
        .map(|name| {
            let entry = lock
                .find_entry(name)
                .ok_or_else(|| anyhow::anyhow!("'{name}' was not found in orbit.lock"))?;
            if entry.provider == "file" {
                anyhow::bail!(
                    "'{}' is a local file and has no online source to check",
                    entry.mod_id
                );
            }
            Ok(entry.mod_id.clone())
        })
        .transpose()?;

    let providers = super::create_instance_providers(&dir, None, &ctx.runtime)?;

    let selector = super::resolution_selector(ctx.dry_run, ctx.yes);
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
    let diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            requested_package
                .as_deref()
                .is_none_or(|package| diagnostic.package == package)
        })
        .cloned()
        .collect();
    super::print_resolution_diagnostics(&diagnostics);
    super::print_resolution_warnings(&report.warnings);
    let mut results = report.updates;

    if let Some(package) = requested_package.as_deref() {
        results.retain(|outdated| outdated.mod_id == package);
    }

    if results.is_empty() {
        println!(
            "{}",
            crate::cli::output::no_upgrade_message(
                requested_package.as_deref(),
                !diagnostics.is_empty()
            )
        );
        return Ok(());
    }

    println!("\nUpdates available:");
    println!("{}", crate::cli::output::outdated_table(&results));

    Ok(())
}
