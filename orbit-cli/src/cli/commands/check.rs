use super::CliContext;
use anyhow::Result;

pub async fn handle(_version: String, _modloader: Option<String>, _ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit check' is not yet implemented.");
    std::process::exit(2);
}
