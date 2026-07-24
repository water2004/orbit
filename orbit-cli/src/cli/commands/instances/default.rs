use crate::cli::commands::CliContext;
use anyhow::Result;

pub async fn handle(name: String, ctx: &CliContext) -> Result<()> {
    orbit_core::set_default_instance(ctx.runtime.paths(), &name)?;
    println!("Default instance set to '{name}'.");
    Ok(())
}
