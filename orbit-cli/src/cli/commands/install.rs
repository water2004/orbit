use anyhow::Result;
use super::CliContext;

/// `orbit install` — 根据 orbit.toml + orbit.lock 还原全部模组。
/// 不接受 mod 名称参数（单个模组安装请用 `orbit add`）。
pub async fn handle(
    _target: Option<String>,
    _group: Option<String>,
    _no_optional: bool,
    _locked: bool,
    _ctx: &CliContext,
) -> Result<()> {
    eprintln!("⚠ Full environment restore ('orbit install') is not yet implemented.");
    eprintln!("  Use 'orbit add <slug>' to install a single mod.");
    std::process::exit(2);
}
