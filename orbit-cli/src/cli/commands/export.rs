use super::CliContext;
use anyhow::Result;

pub async fn handle(
    _file: Option<String>,
    _target: Option<String>,
    _format: String,
    _ctx: &CliContext,
) -> Result<()> {
    eprintln!("⚠ 'orbit export' is not yet implemented.");
    std::process::exit(2);
}
