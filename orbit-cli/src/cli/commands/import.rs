use anyhow::Result;

pub async fn handle(file: String) -> Result<()> {
    println!("Importing from: {}", file);
    Ok(())
}
