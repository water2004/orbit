mod cli;

use cli::{Cli, commands::{CliContext, CommandHandler}};
use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
