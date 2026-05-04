use anyhow::{Context, Result};
use orbit_core::{OrbitError, install_to_instance, upgrade_all_in_instance};
use orbit_core::providers::create_providers_default;
use super::CliContext;

pub async fn handle(mod_name: Option<String>, ctx: &CliContext) -> Result<()> {
    let instance_dir = std::env::current_dir().context("failed to get current directory")?;
    let providers = create_providers_default().context("failed to create providers")?;

    let yes = ctx.yes;
    let prompt_fn: Option<Box<dyn FnOnce(&orbit_core::InstallReport) -> bool + Send>> = if ctx.dry_run {
        None
    } else {
        Some(Box::new(move |report| super::prompt_install_report(report, yes)))
    };

    if let Some(name) = mod_name {
        let slug = name.trim_start_matches("mr:").trim_start_matches("cf:");
        match install_to_instance(slug, "*", &instance_dir, &providers, false, ctx.dry_run, true, prompt_fn).await {
            Ok(report) => {
                if ctx.dry_run {
                    for m in &report.installed { println!("  [dry-run] would upgrade {} to v{}", m.mod_id, m.version); }
                    return Ok(());
                }
                if report.installed.is_empty() {
                    println!("No new versions were installed.");
                } else {
                    println!("\nSuccessfully upgraded {} mod(s).", report.installed.len());
                }
                Ok(())
            }
            Err(OrbitError::ModNotFound(_)) => {
                anyhow::bail!("Mod '{slug}' is not installed or found. Use 'orbit add {slug}' to install it.");
            }
            Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
            Err(e) => anyhow::bail!("Upgrade failed: {e}"),
        }
    } else {
        match upgrade_all_in_instance(&instance_dir, &providers, ctx.dry_run, prompt_fn).await {
            Ok(report) => {
                if ctx.dry_run {
                    for m in &report.installed { println!("  [dry-run] would upgrade {} to v{}", m.mod_id, m.version); }
                    return Ok(());
                }
                if report.installed.is_empty() {
                    println!("No new versions were installed. All mods are up to date.");
                } else {
                    println!("\nSuccessfully upgraded {} mod(s).", report.installed.len());
                }
                Ok(())
            }
            Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
            Err(e) => anyhow::bail!("Upgrade failed: {e}"),
        }
    }
}
