use anyhow::Result;

pub async fn handle(
    tree: bool,
    _target: Option<String>,
) -> Result<()> {
    // TODO: Phase 2 — 读取 orbit.lock 并格式化输出
    if tree {
        println!("Listing installed mods (tree mode)...");
    } else {
        println!("Listing installed mods...");
    }
    Ok(())
}
