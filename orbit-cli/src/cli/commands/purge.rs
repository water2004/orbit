use anyhow::Result;

pub fn handle(mod_name: String) -> Result<()> {
    println!("Deep purging mod and its config: {}", mod_name);
    Ok(())
}
