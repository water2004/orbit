use anyhow::Result;

pub fn handle_list() -> Result<()> {
    println!("Listing all managed instances...");
    Ok(())
}

pub fn handle_default(name: String) -> Result<()> {
    println!("Setting default instance: {}", name);
    Ok(())
}

pub fn handle_remove(name: String) -> Result<()> {
    println!("Removing instance from Orbit: {}", name);
    Ok(())
}
