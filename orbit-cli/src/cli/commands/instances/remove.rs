use crate::cli::commands::CliContext;
use anyhow::Result;

pub async fn handle(name: String, _ctx: &CliContext) -> Result<()> {
    let removed = orbit_core::remove_instance(&name)?;
    let current = std::env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    let removed_path = std::path::PathBuf::from(&removed.path).canonicalize().ok();
    if current.is_some() && current == removed_path {
        eprintln!("Warning: removed instance is the current working directory.");
    }
    println!("Removed '{name}' from Orbit tracking. Files on disk were NOT deleted.");
    Ok(())
}
