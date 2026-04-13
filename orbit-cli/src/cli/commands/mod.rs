pub mod init;
pub mod instances;
pub mod sync;
pub mod update;
pub mod upgrade;
pub mod search;
pub mod install;
pub mod remove;
pub mod purge;
pub mod list;
pub mod import;
pub mod export;
pub mod check;
pub mod cache;

use anyhow::Result;

pub trait CommandHandler {
    fn execute(self) -> Result<()>;
}

pub use init::handle as handle_init;
pub use instances::{handle_list as handle_instances_list, handle_default as handle_instances_default, handle_remove as handle_instances_remove};
pub use sync::handle as handle_sync;
pub use update::handle as handle_update;
pub use upgrade::handle as handle_upgrade;
pub use search::handle as handle_search;
pub use install::handle as handle_install;
pub use remove::handle as handle_remove;
pub use purge::handle as handle_purge;
pub use list::handle as handle_list;
pub use import::handle as handle_import;
pub use export::handle as handle_export;
pub use check::handle as handle_check;
pub use cache::handle_clean as handle_cache_clean;

