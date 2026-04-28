mod cli;

use cli::{Cli, commands::CommandHandler};
use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.command.execute().await
}
