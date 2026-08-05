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
            anyhow::anyhow!(
                "{}",
                tr!(
                    "Package '%{package}' is not installed. Use its JAR-declared mod_id.",
                    package = name
                )
            )
        })?;
        let package = entry.mod_id.clone();
        let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
        match install_to_instance(
            InstallTarget::Package(package.clone()),
            "*",
            &instance_dir,
            &providers,
            ctx.runtime.candidate_storage(),
            InstallOptions {
                dry_run: ctx.dry_run,
                intent: InstallIntent::Upgrade,
                optional: false,
                env: None,
                string: None,
            },
            super::install_interaction(ctx),
        )
        .await
        {
            Ok(report) => {
                if report.installed.is_empty() && report.removed.is_empty() {
                    match ctx.output.format {
                        OutputFormat::Text => {
                            ctx.print_result_line(format_args!(
                                "{}",
                                no_upgrade_message(
                                    Some(&entry.mod_id),
                                    !report.diagnostics.is_empty(),
                                )
                            ));
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
                    "{}",
                    tr!(
                        "Package '%{package}' is not installed or no candidate source is available.",
                        package = package
                    )
                );
            }
            Err(OrbitError::Conflict(msg)) => anyhow::bail!(
                "{}",
                tr!("Dependency conflict:\n\n  %{detail}", detail = msg)
            ),
            Err(e) => anyhow::bail!("{}", tr!("Upgrade failed: %{detail}", detail = e)),
        }
    } else {
        let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
        match upgrade_all_in_instance(
            &instance_dir,
            &providers,
            ctx.runtime.candidate_storage(),
            ctx.dry_run,
            super::install_interaction(ctx),
        )
        .await
        {
            Ok(report) => {
                if report.installed.is_empty() && report.removed.is_empty() {
                    match ctx.output.format {
                        OutputFormat::Text => {
                            ctx.print_result_line(format_args!(
                                "{}",
                                no_upgrade_message(None, !report.diagnostics.is_empty())
                            ));
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
            Err(OrbitError::Conflict(msg)) => anyhow::bail!(
                "{}",
                tr!("Dependency conflict:\n\n  %{detail}", detail = msg)
            ),
            Err(e) => anyhow::bail!("{}", tr!("Upgrade failed: %{detail}", detail = e)),
        }
    }
}
