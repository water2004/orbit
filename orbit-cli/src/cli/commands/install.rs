use anyhow::{Context, Result};
use orbit_core::{OrbitError, install_to_instance};
use orbit_core::providers::create_providers_default;
use super::CliContext;

pub async fn handle(
    mod_name: Option<String>,
    constraint: Option<String>,
    _target: Option<String>,
    _group: Option<String>,
    _no_optional: bool,
    _locked: bool,
    ctx: &CliContext,
) -> Result<()> {
    match mod_name {
        Some(slug) => handle_install_mod(slug, constraint.unwrap_or_else(|| "*".into()), ctx).await,
        None => {
            eprintln!("⚠ Full environment restore ('orbit install') is not yet implemented.");
            eprintln!("  Use 'orbit install <slug>' to install a single mod.");
            std::process::exit(2);
        }
    }
}

pub async fn handle_install_mod(slug: String, constraint: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = std::env::current_dir().context("failed to get current directory")?;
    let providers = create_providers_default().context("failed to create providers")?;

    match install_to_instance(&slug, &constraint, &instance_dir, &providers, false, ctx.dry_run).await {
        Ok(report) => {
            if ctx.dry_run {
                for m in &report.installed {
                    println!("  [dry-run] would install {} v{}", m.key, m.version);
                }
                return Ok(());
            }
            for m in &report.installed {
                println!("  + installed {} v{}", m.key, m.version);
                for (dep_id, dep_ver, _) in &m.jar_deps {
                    println!("      ↳ {dep_id} {dep_ver}");
                }
            }
            for dep in &report.already_satisfied { println!("  ✓ {dep} (already satisfied)"); }
            for dep in &report.skipped_optional { println!("  ~ {dep} (optional, skipped)"); }
            println!("\nInstalled {} mod(s), {} already satisfied, {} optional skipped.",
                report.installed.len(), report.already_satisfied.len(), report.skipped_optional.len());
            Ok(())
        }
        Err(OrbitError::ModNotFound(_)) => {
            let results = providers[0].search(&slug, None, None, 5).await.context("search failed")?;
            if results.is_empty() {
                anyhow::bail!("No mod found for '{slug}' on any platform.");
            }
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
                if trimmed.is_empty() { anyhow::bail!("Install cancelled."); }
                match trimmed.parse::<usize>() {
                    Ok(idx) if idx < results.len() => results[idx].slug.clone(),
                    _ => anyhow::bail!("Invalid choice."),
                }
            };
            eprintln!("Installing {}...", slug);
            Box::pin(handle_install_mod(slug, constraint, ctx)).await
        }
        Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
        Err(e) => anyhow::bail!("Install failed: {e}"),
    }
}

fn format_downloads(d: u64) -> String {
    if d >= 1_000_000 { format!("{:.1}M", d as f64 / 1_000_000.0) }
    else if d >= 1_000 { format!("{:.1}K", d as f64 / 1_000.0) }
    else { d.to_string() }
}
