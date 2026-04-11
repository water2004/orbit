use anyhow::Result;

pub fn handle(query: String) -> Result<()> {
    println!("Searching for mods: {}", query);
    Ok(())
}
