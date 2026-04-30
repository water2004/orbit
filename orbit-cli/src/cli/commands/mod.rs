pub mod init;
pub mod instances;
pub mod sync;
pub mod outdated;
pub mod upgrade;
pub mod search;
pub mod add;
pub mod install;
pub mod remove;
pub mod purge;
pub mod list;
pub mod info;
pub mod import;
pub mod export;
pub mod check;
pub mod cache;

use anyhow::Result;

pub trait CommandHandler {
    async fn execute(self) -> Result<()>;
}

pub use init::handle as handle_init;
pub use sync::handle as handle_sync;
pub use outdated::handle as handle_outdated;
pub use upgrade::handle as handle_upgrade;
pub use search::handle as handle_search;
pub use add::handle as handle_add;
pub use install::handle as handle_install;
pub use remove::handle as handle_remove;
pub use purge::handle as handle_purge;
pub use list::handle as handle_list;
pub use info::handle as handle_info;
pub use import::handle as handle_import;
pub use export::handle as handle_export;
pub use check::handle as handle_check;
