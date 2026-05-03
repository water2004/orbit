use anyhow::Result;
use super::CliContext;

pub async fn handle(
    _mod_name: String,
    _platform: Option<String>,
    _version: Option<String>,
    _env: Option<String>,
    _optional: bool,
    _no_deps: bool,
    _ctx: &CliContext,
) -> Result<()> {
    eprintln!("⚠ 'orbit add' is not yet implemented. Use 'orbit install <slug>' instead.");
    std::process::exit(2)
}
