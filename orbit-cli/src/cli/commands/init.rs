use anyhow::Result;

pub async fn handle(
    name: String,
    mc_version: Option<String>,
    modloader: Option<String>,
    modloader_version: Option<String>,
) -> Result<()> {
    // TODO: Phase 2 — 调用 orbit_core::manifest::OrbitManifest
    println!("Initializing Orbit project: {name}");
    if let Some(ref v) = mc_version { println!("  mc_version: {v}"); }
    if let Some(ref l) = modloader { println!("  modloader: {l}"); }
    if let Some(ref mv) = modloader_version { println!("  modloader_version: {mv}"); }
    Ok(())
}
