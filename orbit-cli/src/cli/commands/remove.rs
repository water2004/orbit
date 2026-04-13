use anyhow::Result;

pub fn handle(mod_name: String) -> Result<()> {
    println!("Removing mod: {}", mod_name);
    Ok(())
}
