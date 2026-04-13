use anyhow::Result;

pub fn handle(mod_name: Option<String>) -> Result<()> {
    match mod_name {
        Some(m) => println!("Upgrading mod: {}", m),
        None => println!("Upgrading all mods..."),
    }
    Ok(())
}
