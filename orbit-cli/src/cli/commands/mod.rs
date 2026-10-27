pub mod activation;
pub mod add;
pub mod audit;
pub mod cache;
pub mod capabilities;
pub mod config;
pub mod constraint;
pub mod env;
pub mod export;
pub mod fix;
pub mod import;
pub mod info;
pub mod init;
pub mod install;
pub mod instances;
pub mod launch;
pub mod list;
pub mod migrate;
pub mod outdated;
pub mod ownership;
pub mod purge;
pub mod remote;
pub mod remove;
pub mod reset;
pub mod search;
pub mod sync;
pub mod update;
pub mod upgrade;
pub mod versions;

pub use constraint::handle as handle_constraint;
pub use env::handle as handle_env;
pub use versions::handle as handle_versions;

use anyhow::Result;

pub trait CommandHandler {
    async fn execute(self, ctx: &CliContext) -> Result<()>;
}

pub use activation::handle as handle_activation;
pub use add::handle as handle_add;
pub use audit::handle as handle_audit;
pub use capabilities::handle as handle_capabilities;
pub use config::handle as handle_config;
pub use export::handle as handle_export;
pub use fix::handle as handle_fix;
pub use import::handle as handle_import;
pub use info::handle as handle_info;
pub use init::handle as handle_init;
pub use install::handle as handle_install;
pub use launch::handle as handle_launch;
pub use list::handle as handle_list;
pub use outdated::handle as handle_outdated;
pub use ownership::handle as handle_ownership;
pub use purge::handle as handle_purge;
pub use remote::handle as handle_remote;
pub use remove::handle as handle_remove;
pub use reset::handle as handle_reset;
pub use search::handle as handle_search;
pub use sync::handle as handle_sync;
pub use upgrade::handle as handle_upgrade;

use interaction::{machine_interaction, read_machine_response};
use providers::normalize_platform;
mod context;
mod interaction;
mod providers;
pub use context::CliContext;
pub use interaction::{
    confirm_data_purge, confirm_data_reset, install_interaction, operation_progress,
    print_resolution_diagnostics, print_resolution_warnings, print_transaction_result,
    resolution_selector,
};
pub use providers::{create_instance_providers, parse_package_remote, resolve_platform_target};
