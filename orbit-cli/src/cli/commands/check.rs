use anyhow::Result;

pub async fn handle(
    version: String,
    modloader: Option<String>,
) -> Result<()> {
    // TODO: Phase 2 — 调用 orbit_core::checker::check_compatibility
    println!("Checking compatibility with Minecraft {version} (modloader={modloader:?})...");
    Ok(())
}
