use anyhow::Result;

pub fn handle_sync() -> Result<()> {
    println!("Syncing mods...");
    Ok(())
}

pub fn handle_update() -> Result<()> {
    println!("Checking for updates...");
    Ok(())
}

pub fn handle_upgrade(mod_name: Option<String>) -> Result<()> {
    match mod_name {
        Some(m) => println!("Upgrading mod: {}", m),
        None => println!("Upgrading all mods..."),
    }
    Ok(())
}
