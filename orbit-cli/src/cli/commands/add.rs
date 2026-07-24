use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::{
    InstallOptions, InstallPrompt, OrbitError, install_local_file_to_instance, install_to_instance,
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
            super::create_instance_providers(&instance_dir, None)?
        };
        let yes = ctx.yes;
        let prompt_fn: Option<InstallPrompt> = if ctx.dry_run {
            None
        } else {
            Some(Box::new(move |report| {
                super::prompt_install_report(report, yes)
            }))
        };
        let report = install_local_file_to_instance(
            std::path::Path::new(path),
            version.as_deref(),
            &instance_dir,
            &providers,
            InstallOptions {
                no_deps,
                dry_run: ctx.dry_run,
                existing_ok: false,
                optional,
                env,
            },
            prompt_fn,
        )
        .await
        .map_err(|error| anyhow::anyhow!("Add failed: {error}"))?;
        super::print_resolution_diagnostics(&report.diagnostics);
        super::print_resolution_warnings(&report.warnings);
        if ctx.dry_run {
            for installed in &report.installed {
                println!(
                    "  [dry-run] would install {} v{}",
                    installed.mod_id, installed.version
                );
            }
        } else if report.installed.is_empty() {
            println!("Add cancelled.");
        } else {
            println!(
                "Successfully added local mod and {} dependency mod(s).",
                report.installed.len().saturating_sub(1)
            );
        }
        return Ok(());
    }

    let constraint = version.unwrap_or_else(|| "*".into());
    let (selected_platform, slug) = super::resolve_platform_target(&mod_name, platform.as_deref())?;
    let instance_dir = ctx.instance_dir()?;
    let providers = super::create_instance_providers(&instance_dir, selected_platform.as_deref())?;

    let yes = ctx.yes;
    let prompt_fn: Option<InstallPrompt> = if ctx.dry_run {
        None
    } else {
        Some(Box::new(move |report| {
            super::prompt_install_report(report, yes)
        }))
    };

    match install_to_instance(
        slug,
        &constraint,
        &instance_dir,
        &providers,
        InstallOptions {
            no_deps,
            dry_run: ctx.dry_run,
            existing_ok: false,
            optional,
            env: env.clone(),
        },
        prompt_fn,
    )
    .await
    {
        Ok(report) => {
            super::print_resolution_diagnostics(&report.diagnostics);
            super::print_resolution_warnings(&report.warnings);
            if ctx.dry_run {
                for m in &report.installed {
                    println!("  [dry-run] would install {} v{}", m.mod_id, m.version);
                }
                return Ok(());
            }
            if report.installed.is_empty() {
                println!("No new mods were installed.");
            } else {
                println!(
                    "\nSuccessfully installed {} mod(s).",
                    report.installed.len()
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
            let slug = if ctx.yes {
                results[0].slug.clone()
            } else {
                eprint!("\nChoose a number (or press Enter to cancel): ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("Add cancelled.");
                }
                match trimmed.parse::<usize>() {
                    Ok(idx) if idx < results.len() => results[idx].slug.clone(),
                    _ => anyhow::bail!("Invalid choice."),
                }
            };
            eprintln!("Installing {}...", slug);
            Box::pin(handle(
                slug,
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
