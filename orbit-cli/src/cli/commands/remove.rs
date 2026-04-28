use anyhow::Result;

pub async fn handle(mod_name: String) -> Result<()> {
    println!("Removing mod: {}", mod_name);
    Ok(())
}
