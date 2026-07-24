use super::CliContext;
use anyhow::{Context, Result};

pub async fn handle(mod_name: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let lock = orbit_core::Lockfile::open(&instance_dir).context("failed to read orbit.lock")?;
    let entry = lock
        .find_entry(&mod_name)
        .ok_or_else(|| anyhow::anyhow!("'{mod_name}' is not installed"))?;
    let mod_id = entry.mod_id.clone();
    let slug = entry
        .modrinth
        .as_ref()
        .map(|modrinth| modrinth.slug.clone());
    let config_dir = instance_dir.join("config");
    let candidates = orbit_core::find_config_candidates(&mod_id, slug.as_deref(), &config_dir)?;
    let selected = select_candidates(&candidates, ctx)?;

    let removed = orbit_core::remove_from_instance(&mod_id, &instance_dir, ctx.dry_run)?;
    if ctx.dry_run {
        println!(
            "[dry-run] would purge '{}' and {} config file(s).",
            removed.mod_id,
            selected.len()
        );
        return Ok(());
    }
    let removed_configs = orbit_core::remove_config_candidates(&config_dir, &selected)?;
    println!(
        "Purged {}: removed {} jar and {} config file(s).",
        removed.mod_id,
        usize::from(removed.jar_deleted),
        removed_configs.len()
    );
    Ok(())
}

fn select_candidates(
    candidates: &[orbit_core::CandidateConfig],
    ctx: &CliContext,
) -> Result<Vec<orbit_core::CandidateConfig>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    eprintln!("Found {} candidate config file(s):", candidates.len());
    let mut selected = Vec::new();
    for candidate in candidates {
        if ctx.yes || ctx.dry_run {
            eprintln!("  {} ({})", candidate.path, candidate.reason);
            selected.push(candidate.clone());
            continue;
        }
        eprint!("  {} [{}] remove? [y/N] ", candidate.path, candidate.reason);
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
