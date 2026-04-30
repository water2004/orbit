use anyhow::Result;

pub async fn handle(
    query: String,
    platform: Option<String>,
    limit: usize,
) -> Result<()> {
    // TODO: Phase 2 — 调用 provider.search()
    println!("Searching for '{query}' (platform={platform:?}, limit={limit})...");
    Ok(())
}
