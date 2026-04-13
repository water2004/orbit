use anyhow::Result;

pub fn handle(version: String) -> Result<()> {
    println!("Checking availability on Minecraft version: {}", version);
    Ok(())
}
