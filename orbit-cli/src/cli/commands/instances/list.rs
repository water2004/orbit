use crate::cli::commands::CliContext;
use anyhow::Result;

use crate::cli::output::{InstancesOutput, OutputFormat, instance_view};

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let registry = orbit_core::InstancesRegistry::load(ctx.runtime.paths().instances_file())?;
    if registry.instances.is_empty() {
        if ctx.output.format == OutputFormat::Text {
            println!(
                "{}",
                tr!("No instances registered. Use 'orbit init' to get started.")
            );
        } else {
            crate::cli::output::print_json(
                "instances",
                &InstancesOutput {
                    subcommand: "list".to_string(),
                    instances: Vec::new(),
                },
            );
        }
        return Ok(());
    }

    let current = std::env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    let views: Vec<_> = registry
        .instances
        .iter()
        .map(|instance| {
            let is_current = current.as_ref().is_some_and(|current| {
                std::path::Path::new(&instance.path)
                    .canonicalize()
                    .ok()
                    .as_deref()
                    == Some(current)
            });
            instance_view(instance, is_current)
        })
        .collect();

    match ctx.output.format {
        OutputFormat::Text => {
            println!(
                "{}",
                crate::cli::output::instances_table(
                    &registry.instances,
                    current
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .as_deref(),
                )
            );
        }
        OutputFormat::Json => {
            crate::cli::output::print_json(
                "instances",
                &InstancesOutput {
                    subcommand: "list".to_string(),
                    instances: views,
                },
            );
        }
    }
    Ok(())
}
