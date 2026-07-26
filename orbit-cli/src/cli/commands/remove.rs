use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::{OrbitError, list_dependencies, remove_from_instance};

use crate::cli::output::{OutputFormat, RemoveOutput};

pub async fn handle(input: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;

    match remove_from_instance(&input, &instance_dir, ctx.dry_run) {
        Ok(report) => {
            match ctx.output.format {
                OutputFormat::Text => {
                    if ctx.dry_run {
                        println!("[dry-run] would remove '{}'.", report.mod_id);
                        return Ok(());
                    }
                    println!(
                        "Removed '{}'{}.",
                        report.mod_id,
                        if report.jar_deleted {
                            " and its JAR file"
                        } else {
                            ""
                        }
                    );
                }
                OutputFormat::Json => {
                    crate::cli::output::print_json(
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
            let deps = list_dependencies(&instance_dir).context("failed to list dependencies")?;
            if deps.is_empty() {
                anyhow::bail!("No dependencies in orbit.toml.");
            }
            eprintln!("'{input}' not found in orbit.toml. Installed dependencies:");
            for (i, package) in deps.iter().enumerate() {
                eprintln!("  [{i}] {package}");
            }
            let key = if ctx.yes {
                anyhow::bail!("'{input}' not found. Use an exact JAR-declared mod_id.");
            } else {
                eprint!("\nChoose a number (or press Enter to cancel): ");
                let mut choice = String::new();
                std::io::stdin().read_line(&mut choice).ok();
                let trimmed = choice.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("Remove cancelled.");
                }
                match trimmed.parse::<usize>() {
                    Ok(i) if i < deps.len() => deps[i].clone(),
                    _ => anyhow::bail!("Invalid choice."),
                }
            };
            Box::pin(handle(key, ctx)).await
        }
        Err(OrbitError::Conflict(msg)) => anyhow::bail!("{msg}"),
        Err(OrbitError::ManifestNotFound) => {
            anyhow::bail!("orbit.toml not found in this directory.")
        }
        Err(OrbitError::LockfileNotFound) => anyhow::bail!("orbit.lock not found."),
        Err(e) => anyhow::bail!("Remove failed: {e}"),
    }
}
