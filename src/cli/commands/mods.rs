use anyhow::Result;

pub fn handle_install(mod_name: Option<String>) -> Result<()> {
    match mod_name {
        Some(m) => println!("Installing mod: {}", m),
        None => println!("Installing all mods from orbit.toml"),
    }
    Ok(())
}

pub fn handle_remove(mod_name: String) -> Result<()> {
    println!("Removing mod: {}", mod_name);
    Ok(())
}

pub fn handle_purge(mod_name: String) -> Result<()> {
    println!("Deep purging mod and its config: {}", mod_name);
    Ok(())
}

pub fn handle_list() -> Result<()> {
    println!("Listing installed mods...");
    Ok(())
}

pub fn handle_search(query: String) -> Result<()> {
    println!("Searching for mods: {}", query);
    Ok(())
}
