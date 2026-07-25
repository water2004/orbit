use super::CliContext;
use anyhow::{Context, Result};

pub async fn handle(version: String, modloader: Option<String>, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let manifest =
        orbit_core::ManifestFile::open(&instance_dir).context("failed to read orbit.toml")?;
    let lockfile =
        orbit_core::Lockfile::open(&instance_dir).context("failed to read orbit.lock")?;
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
        println!("No online packages in orbit.lock to check.");
        return Ok(());
    }

    let compatible = results.iter().filter(|result| result.compatible).count();
    for result in &results {
        if let Some(available) = &result.available_version {
            println!(
                "  {}  {}  ✓ {} available on {}",
                result.mod_name, result.current_version, available, result.provider
            );
        } else {
            println!(
                "  {}  {}  ✗ no compatible version yet",
                result.mod_name, result.current_version
            );
        }
    }
    println!(
        "\n{} of {} mods are ready for Minecraft {version}.",
        compatible,
        results.len()
    );
    let blockers: Vec<_> = results
        .iter()
        .filter(|result| !result.compatible)
        .map(|result| result.mod_name.as_str())
        .collect();
    if !blockers.is_empty() {
        println!("Blocking the upgrade: {}.", blockers.join(", "));
    }
    Ok(())
}
