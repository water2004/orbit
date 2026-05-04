use anyhow::{Context, Result};
use orbit_core::{OrbitError, install_to_instance};
use orbit_core::providers::create_providers_default;
use super::CliContext;

pub async fn handle(
    mod_name: String,
    _platform: Option<String>,
    version: Option<String>,
    _env: Option<String>,
    _optional: bool,
    no_deps: bool,
    ctx: &CliContext,
) -> Result<()> {
    let constraint = version.unwrap_or_else(|| "*".into());
    let slug = mod_name.trim_start_matches("mr:").trim_start_matches("cf:");
    let instance_dir = std::env::current_dir().context("failed to get current directory")?;
    let providers = create_providers_default().context("failed to create providers")?;

    let yes = ctx.yes;
    let prompt_fn: Option<Box<dyn FnOnce(&orbit_core::InstallReport) -> bool + Send>> = if ctx.dry_run {
        None
    } else {
        Some(Box::new(move |report| super::prompt_install_report(report, yes)))
    };

    match install_to_instance(slug, &constraint, &instance_dir, &providers, no_deps, ctx.dry_run, false, prompt_fn).await {
        Ok(report) => {
            if ctx.dry_run {
                for m in &report.installed { println!("  [dry-run] would install {} v{}", m.mod_id, m.version); }
                return Ok(());
            }
            if report.installed.is_empty() {
                println!("No new mods were installed.");
            } else {
                println!("\nSuccessfully installed {} mod(s).", report.installed.len());
            }
            Ok(())
        }
        Err(OrbitError::ModNotFound(_)) => {
            let results = providers[0].search(slug, None, None, 5).await.context("search failed")?;
            if results.is_empty() { anyhow::bail!("No mod found for '{slug}' on any platform."); }
            eprintln!("Could not find '{slug}'. Did you mean:");
            for (i, item) in results.iter().enumerate() {
                let dl = format_downloads(item.downloads);
                eprintln!("  [{i}] {s} — {n}  ⬇ {dl}  mc [{mc}]",
                    s = item.slug, n = item.name, dl = dl,
                    mc = item.mc_versions.iter().rev().take(3).map(|s: &String| s.as_str()).collect::<Vec<_>>().join(", "));
            }
            let slug = if ctx.yes { results[0].slug.clone() } else {
                eprint!("\nChoose a number (or press Enter to cancel): ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                let trimmed = input.trim();
                if trimmed.is_empty() { anyhow::bail!("Add cancelled."); }
                match trimmed.parse::<usize>() {
                    Ok(idx) if idx < results.len() => results[idx].slug.clone(),
                    _ => anyhow::bail!("Invalid choice."),
                }
            };
            eprintln!("Installing {}...", slug);
            Box::pin(handle(slug, _platform, Some(constraint), _env, _optional, no_deps, ctx)).await
        }
        Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
        Err(e) => anyhow::bail!("Add failed: {e}"),
    }
}

fn format_downloads(d: u64) -> String {
    if d >= 1_000_000 { format!("{:.1}M", d as f64 / 1_000_000.0) }
    else if d >= 1_000 { format!("{:.1}K", d as f64 / 1_000.0) }
    else { d.to_string() }
}
