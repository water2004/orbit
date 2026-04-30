use anyhow::Result;

pub async fn handle(
    target: Option<String>,
    group: Option<String>,
    no_optional: bool,
    locked: bool,
) -> Result<()> {
    // TODO: Phase 2 — 调用 orbit_core::resolver::resolve + installer::install_all
    let target = target.as_deref().unwrap_or("both");
    println!("Installing mods (target={target}, group={group:?}, no_optional={no_optional}, locked={locked})...");
    Ok(())
}
