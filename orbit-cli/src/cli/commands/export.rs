use anyhow::Result;

pub async fn handle(
    file: Option<String>,
    target: Option<String>,
    format: String,
) -> Result<()> {
    // TODO: Phase 2 — 打包导出
    let output = file.as_deref().unwrap_or("orbit-export.zip");
    println!("Exporting to: {output} (target={target:?}, format={format})");
    Ok(())
}
