use anyhow::Result;

pub async fn handle(
    mod_name: String,
    platform: Option<String>,
) -> Result<()> {
    // TODO: Phase 2 — 调用 provider.get_mod_info()
    println!("Fetching info for '{mod_name}' (platform={platform:?})...");
    Ok(())
}
