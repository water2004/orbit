use super::CliContext;
use anyhow::Result;

pub async fn handle(_ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit sync' is not yet implemented.");
    std::process::exit(2);
}
