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

    let report = orbit_core::outdated::check_all_outdated_with_progress(
        &dir,
        &manifest_file.inner,
        &lock.inner,
        &providers,
        super::resolution_selector(ctx.dry_run, ctx.yes),
        ctx.runtime.jar_cache(),
        super::operation_progress(ctx),
    )
    .await
    .context("failed to check for updates")?;
    super::print_resolution_diagnostics(&report.diagnostics);
    super::print_resolution_warnings(&report.warnings);
    let mut results = report.updates;

    if let Some(package) = requested_package {
        results.retain(|outdated| outdated.mod_id == package);
    }

    if results.is_empty() {
        println!("All mods are up to date.");
        return Ok(());
    }

    println!("\nUpdates available:\n");
    for m in &results {
        println!("  {} {} → {}", m.mod_id, m.current_version, m.new_version);
    }

    Ok(())
}
