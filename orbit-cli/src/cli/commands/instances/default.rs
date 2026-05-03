use anyhow::Result;
use crate::cli::commands::CliContext;

pub async fn handle(_name: String, _ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit instances default' is not yet implemented.");
    std::process::exit(2);
}
