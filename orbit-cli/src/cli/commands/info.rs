use super::CliContext;
use anyhow::Result;

pub async fn handle(_mod_name: String, _platform: Option<String>, _ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit info' is not yet implemented.");
    std::process::exit(2);
}
