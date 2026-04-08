mod utils;
mod adaptors;
mod models;
mod cli;

use cli::{Cli, Commands, InstanceCommands, CacheCommands};
use cli::commands::*;
use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name } => handle_init(name)?,
        Commands::Instances { command } => match command {
            InstanceCommands::List => handle_instances_list()?,
            InstanceCommands::Default { name } => handle_instances_default(name)?,
            InstanceCommands::Remove { name } => handle_instances_remove(name)?,
        },
        Commands::Sync => handle_sync()?,
        Commands::Update => handle_update()?,
        Commands::Upgrade { mod_name } => handle_upgrade(mod_name)?,
        Commands::Search { query } => handle_search(query)?,
        Commands::Install { mod_name } => handle_install(mod_name)?,
        Commands::Remove { mod_name } => handle_remove(mod_name)?,
        Commands::Purge { mod_name } => handle_purge(mod_name)?,
        Commands::List => handle_list()?,
        Commands::Import { file } => handle_import(file)?,
        Commands::Export { file } => handle_export(file)?,
        Commands::Check { version } => handle_check(version)?,
        Commands::Cache { command } => match command {
            CacheCommands::Clean => handle_cache_clean()?,
        },
    }

    Ok(())
}
