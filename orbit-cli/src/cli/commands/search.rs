use anyhow::Result;

pub async fn handle(query: String) -> Result<()> {
    println!("Searching for mods: {}", query);
    Ok(())
}
