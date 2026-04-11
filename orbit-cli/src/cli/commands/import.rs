use anyhow::Result;

pub fn handle(file: String) -> Result<()> {
    println!("Importing from: {}", file);
    Ok(())
}
