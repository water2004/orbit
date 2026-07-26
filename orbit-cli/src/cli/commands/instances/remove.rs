use crate::cli::commands::CliContext;
use anyhow::Result;

use crate::cli::output::{InstanceRemoveOutput, OutputFormat};

pub async fn handle(name: String, ctx: &CliContext) -> Result<()> {
    let removed = orbit_core::remove_instance(ctx.runtime.paths(), &name)?;
    let current = std::env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    let removed_path = std::path::PathBuf::from(&removed.path).canonicalize().ok();
    if current.is_some() && current == removed_path {
        eprintln!("Warning: removed instance is the current working directory.");
    }
    match ctx.output.format {
        OutputFormat::Text => {
            println!("Removed '{name}' from Orbit tracking. Files on disk were NOT deleted.");
        }
        OutputFormat::Json => crate::cli::output::print_json(
            "instances",
            &InstanceRemoveOutput {
                subcommand: "remove".to_string(),
                name,
            },
        ),
    }
    Ok(())
}
