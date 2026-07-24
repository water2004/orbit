use crate::cli::commands::CliContext;
use anyhow::Result;

pub async fn handle(_ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit cache clean' is not yet implemented.");
    std::process::exit(2);
}
