use anyhow::Result;

pub async fn handle(version: String) -> Result<()> {
    println!("Checking availability on Minecraft version: {}", version);
    Ok(())
}
