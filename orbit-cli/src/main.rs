mod cli;

use clap::Parser;
use cli::{
    Cli,
    commands::{CliContext, CommandHandler},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let runtime = orbit_core::RuntimeContext::load(orbit_core::RuntimePathOptions {
        layout: cli.data_layout,
        config_file: cli.config.clone(),
        cache_dir: cli.cache_dir.clone(),
    })?;
    let ctx = CliContext {
        verbose: cli.verbose,
        quiet: cli.quiet,
        yes: cli.yes,
        dry_run: cli.dry_run,
        instance: cli.instance.clone(),
        runtime,
    };
    cli.command.execute(&ctx).await
}
