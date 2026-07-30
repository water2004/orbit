use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::{
    InstallIntent, InstallOptions, InstallTarget, OrbitError, install_local_file_to_instance,
    install_to_instance,
};

use crate::cli::output::OutputFormat;

pub async fn handle(
    mod_name: String,
    platform: Option<String>,
    version: Option<String>,
    env: Option<String>,
    optional: bool,
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
            anyhow::bail!(
                "{}",
                tr!("file: dependencies cannot be combined with --platform")
            );
        }
        let instance_dir = ctx.instance_dir()?;
        let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
        let report = install_local_file_to_instance(
            std::path::Path::new(path),
            version.as_deref(),
            &instance_dir,
            &providers,
            ctx.runtime.jar_cache(),
            InstallOptions {
                dry_run: ctx.dry_run,
                intent: InstallIntent::Add,
                optional,
                env,
            },
            super::install_interaction(ctx),
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", tr!("Add failed: %{detail}", detail = error)))?;
        super::print_transaction_result("add", &report, ctx);
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
        .ok_or_else(|| anyhow::anyhow!("{}", tr!("No provider is configured for add")))?;
    let remote = super::parse_package_remote(provider_name, slug)?;

    match install_to_instance(
        InstallTarget::Remote(remote),
        &constraint,
        &instance_dir,
        &providers,
        ctx.runtime.jar_cache(),
        InstallOptions {
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
            super::print_transaction_result("add", &report, ctx);
            Ok(())
        }
        Err(OrbitError::ModNotFound(_)) => {
            if ctx.output.format == OutputFormat::Json {
                // Search fallback is interactive; in JSON mode we surface the
                // not-found error rather than prompting.
                anyhow::bail!(OrbitError::ModNotFound(slug.to_string()));
            }
            let mut suggestion = None;
            for provider in &providers {
                let results = provider
                    .search(slug, None, None, 5)
                    .await
                    .with_context(|| tr!("Search failed").into_owned())?;
                if !results.is_empty() {
                    suggestion = Some((provider.name().to_string(), results));
                    break;
                }
            }
            let Some((suggestion_platform, results)) = suggestion else {
                anyhow::bail!(
                    "{}",
                    tr!(
                        "No mod was found for '%{slug}' on any configured provider.",
                        slug = slug
                    )
                );
            };
            if ctx.yes {
                anyhow::bail!(
                    "{}",
                    tr!(
                        "'%{slug}' requires choosing a search result; rerun without --yes or use an exact provider project ID.",
                        slug = slug
                    )
                );
            }
            eprintln!(
                "{}",
                tr!("Could not find '%{slug}'. Did you mean:", slug = slug)
            );
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
            eprint!("\n{}", tr!("Choose a number (or press Enter to cancel): "));
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            let trimmed = input.trim();
            if trimmed.is_empty() {
                return Err(
                    orbit_core::OrbitError::Cancelled(tr!("Add cancelled.").into_owned()).into(),
                );
            }
            let project_id = match trimmed.parse::<usize>() {
                Ok(idx) if idx < results.len() => results[idx].project_id.clone(),
                _ => anyhow::bail!("{}", tr!("Invalid choice.")),
            };
            ctx.print_information_line(format_args!(
                "{}",
                tr!("Installing project %{project}…", project = project_id)
            ));
            Box::pin(handle(
                project_id,
                Some(suggestion_platform),
                Some(constraint),
                env,
                optional,
                ctx,
            ))
            .await
        }
        Err(OrbitError::Conflict(msg)) => anyhow::bail!(
            "{}",
            tr!("Dependency conflict:\n\n  %{detail}", detail = msg)
        ),
        Err(e) => anyhow::bail!("{}", tr!("Add failed: %{detail}", detail = e)),
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
