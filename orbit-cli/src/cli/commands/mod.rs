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

/// 全局 CLI 上下文，传递给所有命令 handler。
#[derive(Debug, Clone)]
pub struct CliContext {
    pub verbose: bool,
    pub quiet: bool,
    pub yes: bool,
    pub dry_run: bool,
    pub instance: Option<String>,
}

pub trait CommandHandler {
    async fn execute(self, ctx: &CliContext) -> Result<()>;
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

pub fn prompt_install_report(report: &orbit_core::InstallReport, yes: bool) -> bool {
    if report.installed.is_empty() {
        return true;
    }
    eprintln!("\nThe following mods will be installed/upgraded:");
    for m in &report.installed {
        eprintln!("  + {} v{}", m.mod_id, m.version);
        for (dep_id, dep_ver, _) in &m.jar_deps {
            eprintln!("      ↳ {} {}", dep_id, dep_ver);
        }
        for imp in &m.implanted {
            eprintln!("      ↳ [implanted] {} {}", imp.name, imp.version);
        }
    }
    if !report.already_satisfied.is_empty() {
        eprintln!("\nAlready satisfied: {}", report.already_satisfied.join(", "));
    }
    if yes {
        return true;
    }
    eprint!("\nDo you want to continue? [Y/n] ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}
