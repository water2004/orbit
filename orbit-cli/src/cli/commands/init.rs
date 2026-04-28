use anyhow::Result;

pub async fn handle(name: String) -> Result<()> {
    println!("Initializing Orbit project: {}", name);
    // TODO: 实现初始化逻辑
    Ok(())
}
