use crate::cli::commands::CliContext;
use anyhow::Result;

pub async fn handle(_ctx: &CliContext) -> Result<()> {
    let registry = orbit_core::InstancesRegistry::load()?;
    if registry.instances.is_empty() {
        println!("No instances registered. Use 'orbit init' to get started.");
        return Ok(());
    }

    let current = std::env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    println!("  current  name  path  mc  loader");
    for instance in registry.instances {
        let path = std::path::PathBuf::from(&instance.path);
        let is_current = current
            .as_ref()
            .is_some_and(|current| path.canonicalize().ok().as_ref() == Some(current));
        let current_marker = if is_current { "*" } else { " " };
        let default_marker = if instance.is_default { "(default)" } else { "" };
        println!(
            "{current_marker} {default_marker:9} {}  {}  {}  {}",
            instance.name, instance.path, instance.mc_version, instance.modloader
        );
    }
    Ok(())
}
