use anyhow::Result;

pub async fn handle(mod_name: Option<String>) -> Result<()> {
    // TODO: Phase 2 — 遍历 orbit.lock，调用 provider 查询最新版本
    match mod_name {
        Some(ref m) => println!("Checking if '{m}' is outdated..."),
        None => println!("Checking for outdated mods..."),
    }
    Ok(())
}
