use anyhow::Result;
use super::CliContext;

pub async fn handle(_tree: bool, _target: Option<String>, _ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit list' is not yet implemented.");
    std::process::exit(2);
}
