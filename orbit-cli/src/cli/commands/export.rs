use anyhow::Result;

pub async fn handle(file: Option<String>) -> Result<()> {
    match file {
        Some(f) => println!("Exporting to: {}", f),
        None => println!("Exporting current instance as ZIP..."),
    }
    Ok(())
}
