use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::{
    InstallIntent, InstallOptions, InstallTarget, OrbitError, install_local_file_to_instance,
    install_to_instance,
};

pub async fn handle(
    mod_name: String,
    platform: Option<String>,
    version: Option<String>,
    env: Option<String>,
    optional: bool,
    no_deps: bool,
    ctx: &CliContext,
) -> Result<()> {
    let local_path = mod_name
        .strip_prefix("file:")
        .or_else(|| (platform.as_deref() == Some("file")).then_some(mod_name.as_str()));
    if let Some(path) = local_path {
        if platform
            .as_deref()
            .is_some_and(|platform| platform != "file")
        {
            anyhow::bail!("file: dependencies cannot be combined with --platform");
        }
        let instance_dir = ctx.instance_dir()?;
        let providers = if no_deps {
            Vec::new()
        } else {
            super::create_instance_providers(&instance_dir, None, &ctx.runtime)?
        };
        let report = install_local_file_to_instance(
            std::path::Path::new(path),
            version.as_deref(),
            &instance_dir,
            &providers,
            ctx.runtime.jar_cache(),
            InstallOptions {
                no_deps,
                dry_run: ctx.dry_run,
                intent: InstallIntent::Add,
                optional,
                env,
            },
            super::install_interaction(ctx),
        )
        .await
        .map_err(|error| anyhow::anyhow!("Add failed: {error}"))?;
        super::print_resolution_diagnostics(&report.diagnostics);
        super::print_resolution_warnings(&report.warnings);
        if ctx.dry_run {
            println!("\nAdd preview:");
            println!(
                "{}",
                crate::cli::output::package_changes_table(&report.changes)
            );
        } else if report.installed.is_empty() {
            println!("Add cancelled.");
        } else {
            println!(
                "Successfully added local mod and {} dependency mod(s); removed {} unselected package version(s).",
                report.installed.len().saturating_sub(1),
                report.removed.len()
            );
        }
        return Ok(());
    }

    let constraint = version.unwrap_or_else(|| "*".into());
    let (selected_platform, slug) = super::resolve_platform_target(&mod_name, platform.as_deref())?;
    let instance_dir = ctx.instance_dir()?;
    let providers = super::create_instance_providers(
        &instance_dir,
        selected_platform.as_deref(),
        &ctx.runtime,
    )?;
    let provider_name = selected_platform
        .as_deref()
        .or_else(|| providers.first().map(|provider| provider.name()))
        .ok_or_else(|| anyhow::anyhow!("no provider is configured for add"))?;
    let remote = super::parse_package_remote(provider_name, slug)?;

    match install_to_instance(
        InstallTarget::Remote(remote),
        &constraint,
        &instance_dir,
        &providers,
        ctx.runtime.jar_cache(),
        InstallOptions {
            no_deps,
            dry_run: ctx.dry_run,
            intent: InstallIntent::Add,
            optional,
            env: env.clone(),
        },
        super::install_interaction(ctx),
    )
    .await
    {
        Ok(report) => {
            super::print_resolution_diagnostics(&report.diagnostics);
            super::print_resolution_warnings(&report.warnings);
            if ctx.dry_run {
                println!("\nAdd preview:");
                println!(
                    "{}",
                    crate::cli::output::package_changes_table(&report.changes)
                );
                return Ok(());
            }
            if report.installed.is_empty() && report.removed.is_empty() {
                println!("No new mods were installed.");
            } else {
                println!(
                    "\nSuccessfully installed {} mod(s) and removed {} unselected package version(s).",
                    report.installed.len(),
                    report.removed.len()
                );
            }
            Ok(())
        }
        Err(OrbitError::ModNotFound(_)) => {
            let mut suggestion = None;
            for provider in &providers {
                let results = provider
                    .search(slug, None, None, 5)
                    .await
                    .context("search failed")?;
                if !results.is_empty() {
                    suggestion = Some((provider.name().to_string(), results));
                    break;
                }
            }
            let Some((suggestion_platform, results)) = suggestion else {
                anyhow::bail!("No mod found for '{slug}' on any configured platform.");
            };
            eprintln!("Could not find '{slug}'. Did you mean:");
            for (i, item) in results.iter().enumerate() {
                let dl = format_downloads(item.downloads);
                eprintln!(
                    "  [{i}] {s} — {n}  ⬇ {dl}  mc [{mc}]",
                    s = item.slug,
                    n = item.name,
                    dl = dl,
                    mc = item
                        .mc_versions
                        .iter()
                        .rev()
                        .take(3)
                        .map(|s: &String| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let project_id = if ctx.yes {
                results[0].project_id.clone()
            } else {
                eprint!("\nChoose a number (or press Enter to cancel): ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("Add cancelled.");
                }
                match trimmed.parse::<usize>() {
                    Ok(idx) if idx < results.len() => results[idx].project_id.clone(),
                    _ => anyhow::bail!("Invalid choice."),
                }
            };
            eprintln!("Installing project {}...", project_id);
            Box::pin(handle(
                project_id,
                Some(suggestion_platform),
                Some(constraint),
                env,
                optional,
                no_deps,
                ctx,
            ))
            .await
        }
        Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
        Err(e) => anyhow::bail!("Add failed: {e}"),
    }
}

fn format_downloads(d: u64) -> String {
    if d >= 1_000_000 {
        format!("{:.1}M", d as f64 / 1_000_000.0)
    } else if d >= 1_000 {
        format!("{:.1}K", d as f64 / 1_000.0)
    } else {
        d.to_string()
    }
}
