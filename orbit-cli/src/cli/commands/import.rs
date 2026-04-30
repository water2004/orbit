use anyhow::Result;

pub async fn handle(
    file: String,
    merge_strategy: Option<String>,
) -> Result<()> {
    // TODO: Phase 2 — 解析导入文件 + 合并依赖
    let strategy = merge_strategy.as_deref().unwrap_or("prefer-existing");
    println!("Importing from: {file} (merge_strategy={strategy})");
    Ok(())
}
