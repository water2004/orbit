use anyhow::Result;

pub async fn handle(mod_name: Option<String>) -> Result<()> {
    match mod_name {
        Some(m) => println!("Installing mod: {}", m),
        None => println!("Installing all mods from orbit.toml"),
    }
    Ok(())
}
