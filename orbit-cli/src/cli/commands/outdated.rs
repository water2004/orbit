use anyhow::{Context, Result};
use orbit_core::{check_all_outdated, ManifestFile};
use orbit_core::providers::create_providers_default;
use super::CliContext;

pub async fn handle(mod_name: Option<String>, _ctx: &CliContext) -> Result<()> {
    let dir = std::env::current_dir().context("failed to get current directory")?;
    let manifest_file = ManifestFile::open(&dir).context("failed to read orbit.toml")?;
    let lock = orbit_core::workspace::Lockfile::open(&dir).context("failed to read orbit.lock")?;

    let providers = create_providers_default().context("failed to create providers")?;

    let total = lock.inner.packages.iter().filter(|e| e.modrinth.is_some()).count();
    eprintln!("Checking {total} mod(s) for updates (mc={}, loader={})...\n  This may download candidate JARs for verification.",
        manifest_file.inner.project.mc_version,
        manifest_file.inner.project.modloader,
    );

    let (mut results, _) = orbit_core::outdated::check_all_outdated(&manifest_file.inner, &lock.inner, &providers).await
        .context("failed to check for updates")?;

    if let Some(ref name) = mod_name {
        results.retain(|m| m.mod_id == *name);
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
