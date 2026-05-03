use anyhow::Result;
use crate::cli::commands::CliContext;

pub async fn handle(_ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit instances list' is not yet implemented.");
    std::process::exit(2);
}
