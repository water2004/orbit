use std::path::PathBuf;

use anyhow::Result;

use crate::cli::commands::CliContext;
use crate::cli::output::{InstanceRegisterOutput, OutputFormat, instance_view};

pub async fn handle(name: String, path: PathBuf, ctx: &CliContext) -> Result<()> {
    let entry = orbit_core::register_existing_instance(ctx.runtime.paths(), &name, &path)?;
    match ctx.output.format {
        OutputFormat::Text => println!(
            "{}",
            tr!(
                "Registered Orbit instance '%{name}' at %{path}.",
                name = entry.name,
                path = entry.path
            )
        ),
        OutputFormat::Json => crate::cli::output::print_json(
            "instances",
            &InstanceRegisterOutput {
                subcommand: "register".to_string(),
                instance: instance_view(&entry, false),
            },
        ),
    }
    Ok(())
}
