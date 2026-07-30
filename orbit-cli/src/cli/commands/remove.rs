use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::{OrbitError, list_packages, remove_from_instance};

use crate::cli::output::{OutputFormat, RemoveOutput};

pub async fn handle(input: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;

    match remove_from_instance(&input, &instance_dir, ctx.dry_run) {
        Ok(report) => {
            match ctx.output.format {
                OutputFormat::Text => {
                    if ctx.dry_run {
                        ctx.print_result_line(format_args!(
                            "{}",
                            tr!(
                                "[dry-run] would remove '%{package}'.",
                                package = report.mod_id
                            )
                        ));
                        return Ok(());
                    }
                    ctx.print_result_line(format_args!(
                        "{}",
                        tr!(
                            "Removed '%{package}'%{files}.",
                            package = report.mod_id,
                            files = if report.jar_deleted {
                                tr!(" and its package files").into_owned()
                            } else {
                                String::new()
                            }
                        )
                    ));
                }
                OutputFormat::Json => {
                    ctx.print_json(
                        "remove",
                        &RemoveOutput {
                            mod_id: report.mod_id,
                            jar_deleted: report.jar_deleted,
                        },
                    );
                }
            }
            Ok(())
        }
        Err(OrbitError::ModNotFound(_)) => {
            if ctx.output.format == OutputFormat::Json {
                anyhow::bail!(OrbitError::ModNotFound(input));
            }
            let deps = list_packages(&instance_dir)
                .with_context(|| tr!("Failed to list dependencies").into_owned())?;
            if deps.is_empty() {
                anyhow::bail!("{}", tr!("No dependencies in orbit.toml."));
            }
            if ctx.yes {
                anyhow::bail!(
                    "{}",
                    tr!(
                        "'%{input}' was not found. Use an exact JAR-declared mod_id.",
                        input = input
                    )
                );
            }
            eprintln!(
                "{}",
                tr!(
                    "'%{input}' was not found in orbit.toml. Installed dependencies:",
                    input = input
                )
            );
            for (i, package) in deps.iter().enumerate() {
                eprintln!("  [{i}] {package}");
            }
            eprint!("\n{}", tr!("Choose a number (or press Enter to cancel): "));
            let mut choice = String::new();
            std::io::stdin().read_line(&mut choice).ok();
            let trimmed = choice.trim();
            if trimmed.is_empty() {
                return Err(orbit_core::OrbitError::Cancelled(
                    tr!("Remove cancelled.").into_owned(),
                )
                .into());
            }
            let key = match trimmed.parse::<usize>() {
                Ok(i) if i < deps.len() => deps[i].clone(),
                _ => anyhow::bail!("{}", tr!("Invalid choice.")),
            };
            Box::pin(handle(key, ctx)).await
        }
        Err(OrbitError::Conflict(msg)) => anyhow::bail!("{msg}"),
        Err(OrbitError::ManifestNotFound) => {
            anyhow::bail!(
                "{}",
                tr!("orbit.toml was not found in the current instance")
            )
        }
        Err(OrbitError::LockfileNotFound) => {
            anyhow::bail!(
                "{}",
                tr!("orbit.lock was not found in the current instance")
            )
        }
        Err(e) => anyhow::bail!("{}", tr!("Remove failed: %{detail}", detail = e)),
    }
}
