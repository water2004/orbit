use anyhow::Result;

pub fn handle(name: String) -> Result<()> {
    println!("Setting default instance: {}", name);
    Ok(())
}
