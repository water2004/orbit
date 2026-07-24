use super::CliContext;
use anyhow::Result;
use orbit_core::{
    InstallIntent, InstallOptions, OrbitError, install_to_instance, upgrade_all_in_instance,
};

pub async fn handle(mod_name: Option<String>, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;

    if let Some(name) = mod_name {
        let lockfile = orbit_core::Lockfile::open(&instance_dir)?;
        let entry = lockfile
            .find_entry(name.trim_start_matches("mr:").trim_start_matches("cf:"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Mod '{name}' is not installed. Use 'orbit add {name}' to install it."
                )
            })?;
        let slug = entry.source_slug().map(str::to_string).ok_or_else(|| {
            anyhow::anyhow!(
                "Mod '{}' is a local file and has no online source to upgrade",
                entry.mod_id
            )
        })?;
        let providers = super::create_instance_providers(
            &instance_dir,
            Some(entry.provider.as_str()),
            &ctx.runtime,
        )?;
        match install_to_instance(
            &slug,
            "*",
            &instance_dir,
            &providers,
            ctx.runtime.jar_cache(),
            InstallOptions {
                no_deps: false,
                dry_run: ctx.dry_run,
                intent: InstallIntent::Upgrade,
                optional: false,
                env: None,
            },
            super::install_interaction(ctx.dry_run, ctx.yes),
        )
        .await
        {
            Ok(report) => {
                super::print_resolution_diagnostics(&report.diagnostics);
                super::print_resolution_warnings(&report.warnings);
                if ctx.dry_run {
                    for m in &report.installed {
                        println!("  [dry-run] would select {} v{}", m.mod_id, m.version);
                    }
                    print_dry_run_removals(&report.removed);
                    return Ok(());
                }
                if report.installed.is_empty() && report.removed.is_empty() {
                    println!("No new versions were installed.");
                } else {
                    println!(
                        "\nApplied {} selected package version(s) and removed {} unselected package version(s).",
                        report.installed.len(),
                        report.removed.len()
                    );
                }
                Ok(())
            }
            Err(OrbitError::ModNotFound(_)) => {
                anyhow::bail!(
                    "Mod '{slug}' is not installed or found. Use 'orbit add {slug}' to install it."
                );
            }
            Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
            Err(e) => anyhow::bail!("Upgrade failed: {e}"),
        }
    } else {
        let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
        match upgrade_all_in_instance(
            &instance_dir,
            &providers,
            ctx.runtime.jar_cache(),
            ctx.dry_run,
            super::install_interaction(ctx.dry_run, ctx.yes),
        )
        .await
        {
            Ok(report) => {
                super::print_resolution_diagnostics(&report.diagnostics);
                super::print_resolution_warnings(&report.warnings);
                if ctx.dry_run {
                    for m in &report.installed {
                        println!("  [dry-run] would select {} v{}", m.mod_id, m.version);
                    }
                    print_dry_run_removals(&report.removed);
                    return Ok(());
                }
                if report.installed.is_empty() && report.removed.is_empty() {
                    println!("No new versions were installed. All mods are up to date.");
                } else {
                    println!(
                        "\nApplied {} selected package version(s) and removed {} unselected package version(s).",
                        report.installed.len(),
                        report.removed.len()
                    );
                }
                Ok(())
            }
            Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
            Err(e) => anyhow::bail!("Upgrade failed: {e}"),
        }
    }
}

fn print_dry_run_removals(removals: &[orbit_core::RemovedPackage]) {
    for package in removals {
        println!(
            "  [dry-run] would remove {} v{} ({})",
            package.mod_id, package.version, package.filename
        );
    }
}
