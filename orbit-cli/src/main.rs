mod cli;

use clap::Parser;
use cli::{
    Cli,
    commands::{CliContext, CommandHandler},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 加载全局配置（首次运行自动创建 config.toml）
    let _global_config = orbit_core::GlobalConfig::load()?;

    let cli = Cli::parse();
    let ctx = CliContext {
        verbose: cli.verbose,
        quiet: cli.quiet,
        yes: cli.yes,
        dry_run: cli.dry_run,
        instance: cli.instance.clone(),
    };
    cli.command.execute(&ctx).await
}
