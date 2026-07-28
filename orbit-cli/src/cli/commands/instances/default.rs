use crate::cli::commands::CliContext;
use anyhow::Result;

use crate::cli::output::{InstanceDefaultOutput, OutputFormat};

pub async fn handle(name: String, ctx: &CliContext) -> Result<()> {
    orbit_core::set_default_instance(ctx.runtime.paths(), &name)?;
    match ctx.output.format {
        OutputFormat::Text => {
            println!("{}", tr!("Default instance set to '%{name}'.", name = name))
        }
        OutputFormat::Json => crate::cli::output::print_json(
            "instances",
            &InstanceDefaultOutput {
                subcommand: "default".to_string(),
                name,
            },
        ),
    }
    Ok(())
}
