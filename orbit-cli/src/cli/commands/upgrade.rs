use anyhow::Result;
use super::CliContext;

pub async fn handle(_mod_name: Option<String>, _ctx: &CliContext) -> Result<()> {
    eprintln!("⚠ 'orbit upgrade' is not yet implemented.");
    std::process::exit(2);
}
