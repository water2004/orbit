use super::CliContext;
use anyhow::Result;
use orbit_core::{
    InstallIntent, InstallOptions, InstallTarget, OrbitError, install_to_instance,
    upgrade_all_in_instance,
};

use crate::cli::output::{OutputFormat, no_upgrade_message};

pub async fn handle(mod_name: Option<String>, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;

    if let Some(name) = mod_name {
        let lockfile = orbit_core::Lockfile::open(&instance_dir)?;
        let entry = lockfile.find_entry(&name).ok_or_else(|| {
            anyhow::anyhow!("Package '{name}' is not installed. Use its JAR-declared mod_id.")
        })?;
        let package = entry.mod_id.clone();
        let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
        match install_to_instance(
            InstallTarget::Package(package.clone()),
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
            super::install_interaction(ctx),
        )
        .await
        {
            Ok(report) => {
                if report.installed.is_empty() && report.removed.is_empty() {
                    match ctx.output.format {
                        OutputFormat::Text => {
                            println!(
                                "{}",
                                no_upgrade_message(
                                    Some(&entry.mod_id),
                                    !report.diagnostics.is_empty(),
                                )
                            );
                        }
                        OutputFormat::Json => {
                            super::print_transaction_result("upgrade", &report, ctx);
                        }
                    }
                } else {
                    super::print_transaction_result("upgrade", &report, ctx);
                }
                Ok(())
            }
            Err(OrbitError::ModNotFound(_)) => {
                anyhow::bail!(
                    "Mod '{package}' is not installed or no candidate source is available."
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
            super::install_interaction(ctx),
        )
        .await
        {
            Ok(report) => {
                if report.installed.is_empty() && report.removed.is_empty() {
                    match ctx.output.format {
                        OutputFormat::Text => {
                            println!(
                                "{}",
                                no_upgrade_message(None, !report.diagnostics.is_empty())
                            );
                        }
                        OutputFormat::Json => {
                            super::print_transaction_result("upgrade", &report, ctx);
                        }
                    }
                } else {
                    super::print_transaction_result("upgrade", &report, ctx);
                }
                Ok(())
            }
            Err(OrbitError::Conflict(msg)) => anyhow::bail!("Dependency conflict:\n\n  {msg}"),
            Err(e) => anyhow::bail!("Upgrade failed: {e}"),
        }
    }
}
