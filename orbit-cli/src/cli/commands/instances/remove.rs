use anyhow::Result;

pub async fn handle(name: String) -> Result<()> {
    println!("Removing instance from Orbit: {}", name);
    Ok(())
}
