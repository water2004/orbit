use anyhow::{Context, Result};
use std::collections::HashSet;
use orbit_core::{check_mod_outdated, ManifestFile};
use orbit_core::providers::create_providers_default;
use super::CliContext;

pub async fn handle(mod_name: Option<String>, _ctx: &CliContext) -> Result<()> {
    let dir = std::env::current_dir().context("failed to get current directory")?;
    let manifest_file = ManifestFile::open(&dir).context("failed to read orbit.toml")?;
    let lock = orbit_core::workspace::Lockfile::open(&dir).context("failed to read orbit.lock")?;

    let providers = create_providers_default().context("failed to create providers")?;
    let provider = &providers[0];

    let mc_version = &manifest_file.inner.project.mc_version;
    let loader = &manifest_file.inner.project.modloader;

    let installed_ids: HashSet<&str> = lock.inner.packages.iter()
        .map(|e| e.mod_id.as_str())
        .collect();

    let modrinth_entries: Vec<_> = lock.inner.packages.iter()
        .filter(|e| e.modrinth.is_some())
        .collect();

    if modrinth_entries.is_empty() {
        println!("No modrinth-sourced mods installed.");
        return Ok(());
    }

    let total = modrinth_entries.len();
    eprintln!("Checking {total} mod(s) for updates (mc={mc_version}, loader={loader})...");

    let mut results = Vec::new();
    for (i, entry) in modrinth_entries.iter().enumerate() {
        let mr = entry.modrinth.as_ref().unwrap();
        let slug = &mr.slug;
        eprintln!("  [{}/{}] {} ...", i + 1, total, entry.mod_id);

        let mut versions = match provider.get_versions(slug, Some(mc_version), Some(loader)).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));

        if let Some(outdated) = check_mod_outdated(
            &entry.mod_id,
            &mr.version,
            &versions,
            &installed_ids,
        ) {
            results.push(outdated);
        }
    }

    if let Some(ref name) = mod_name {
        results.retain(|m| m.mod_id == *name);
    }

    if results.is_empty() {
        println!("\nAll mods are up to date.");
        return Ok(());
    }

    println!("\nUpdates available:\n");
    for m in &results {
        let latest = m.latest_overall.as_ref();
        let compat = m.latest_compatible.as_ref();

        match (latest, compat) {
            (Some(latest_v), Some(compat_v)) if latest_v.version_number == compat_v.version_number => {
                println!("  {} {} → {}", m.mod_id, m.current_version, latest_v.version_number);
            }
            (Some(latest_v), Some(compat_v)) => {
                println!("  {} {} → {} (latest: {})",
                    m.mod_id, m.current_version, compat_v.version_number, latest_v.version_number);
            }
            (Some(latest_v), None) => {
                println!("  {} {} → {} [incompatible: missing dependencies]",
                    m.mod_id, m.current_version, latest_v.version_number);
            }
            (None, _) => {}
        }
    }

    Ok(())
}
