use anyhow::Result;

pub fn handle(name: String) -> Result<()> {
    println!("Removing instance from Orbit: {}", name);
    Ok(())
}
