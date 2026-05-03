use anyhow::Result;
use super::CliContext;

pub async fn handle(_ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit sync' is not yet implemented.");
    std::process::exit(2);
}
