use super::CliContext;
use anyhow::Result;

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
    let report = orbit_core::sync_instance(&instance_dir, &providers, ctx.dry_run).await?;

    for package in &report.added {
        println!("  + added      {package}");
    }
    for package in &report.changed {
        println!("  ~ changed    {package}");
    }
    for package in &report.missing {
        println!("  - missing    {package}");
    }
    for package in &report.unlocked {
        println!("  ? unlocked   {package}");
    }
    println!(
        "Sync {}: {} added, {} changed, {} missing, {} unlocked.",
        if ctx.dry_run { "preview" } else { "complete" },
        report.added.len(),
        report.changed.len(),
        report.missing.len(),
        report.unlocked.len()
    );
    if !report.missing.is_empty() || !report.unlocked.is_empty() {
        println!("Run 'orbit install' to restore missing or unlocked mods.");
    }
    Ok(())
}
