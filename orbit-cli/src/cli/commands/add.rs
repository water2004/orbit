use anyhow::Result;

pub async fn handle(
    mod_name: String,
    platform: Option<String>,
    version: Option<String>,
    env: Option<String>,
    optional: bool,
    no_deps: bool,
) -> Result<()> {
    // TODO: Phase 2 — 调用 provider.resolve() + manifest 写入 + installer
    println!("Adding {mod_name} (platform={platform:?}, version={version:?}, env={env:?}, optional={optional}, no_deps={no_deps})...");
    Ok(())
}
