use anyhow::Result;

pub fn handle_import(file: String) -> Result<()> {
    println!("Importing from: {}", file);
    Ok(())
}

pub fn handle_export(file: Option<String>) -> Result<()> {
    match file {
        Some(f) => println!("Exporting to: {}", f),
        None => println!("Exporting current instance as ZIP..."),
    }
    Ok(())
}

pub fn handle_check(version: String) -> Result<()> {
    println!("Checking availability on Minecraft version: {}", version);
    Ok(())
}

pub fn handle_cache_clean() -> Result<()> {
    println!("Cleaning global download cache...");
    Ok(())
}
