use crate::cli::commands::CliContext;
use anyhow::Result;

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let registry = orbit_core::InstancesRegistry::load(ctx.runtime.paths().instances_file())?;
    if registry.instances.is_empty() {
        println!("No instances registered. Use 'orbit init' to get started.");
        return Ok(());
    }

    let current = std::env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .map(|path| path.to_string_lossy().into_owned());
    println!(
        "{}",
        crate::cli::output::instances_table(&registry.instances, current.as_deref())
    );
    Ok(())
}
