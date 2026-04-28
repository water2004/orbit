use anyhow::Result;

pub async fn handle(name: String) -> Result<()> {
    println!("Setting default instance: {}", name);
    Ok(())
}
