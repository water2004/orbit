use anyhow::Result;

pub async fn handle(mod_name: String) -> Result<()> {
    println!("Deep purging mod and its config: {}", mod_name);
    Ok(())
}
