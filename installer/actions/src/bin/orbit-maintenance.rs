//! Explicit per-user cleanup, separate from root-run Debian maintainer scripts.
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};
use orbit_installer_actions::{discovery, execution, platform, transaction::Result};

#[derive(Parser)]
#[command(about = "Clean current-user Orbit data before uninstalling the packages")]
struct Arguments {
    #[arg(long, default_value_t = orbit_i18n::LanguageMode::System)]
    language: orbit_i18n::LanguageMode,
    /// Choose cache, settings (keep instances), or all (delete instances).
    #[arg(long, value_enum, conflicts_with = "restore")]
    level: Option<Level>,
    /// Preview exact paths without changing files.
    #[arg(long, conflicts_with = "restore")]
    dry_run: bool,
    /// Approve the displayed cleanup; never chooses a missing level.
    #[arg(long)]
    yes: bool,
    #[arg(long, value_enum, default_value_t = Output::Text)]
    output_format: Output,
    /// Restore a retained recovery plan after interrupted staging, before deletion.
    #[arg(long)]
    restore: Option<PathBuf>,
}
#[derive(Clone, Copy, ValueEnum)]
enum Level {
    Cache,
    Settings,
    All,
}
impl From<Level> for discovery::CleanupLevel {
    fn from(level: Level) -> Self {
        match level {
            Level::Cache => Self::Cache,
            Level::Settings => Self::Settings,
            Level::All => Self::All,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Output {
    Text,
    Json,
}

fn prompt(message: &str) -> Result<String> {
    if !io::stdin().is_terminal() {
        return Err("interactive input is unavailable; supply --level and --yes explicitly".into());
    }
    eprint!("{message} ");
    io::stderr().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Err("cleanup cancelled".into());
    }
    Ok(answer.trim().into())
}
fn confirm(yes: bool) -> Result<()> {
    if !yes
        && prompt(&orbit_i18n::text(
            "Permanently remove the listed data? Type yes to continue:",
        ))? != "yes"
    {
        return Err("cleanup cancelled".into());
    }
    Ok(())
}
fn run(arguments: Arguments) -> Result<()> {
    orbit_i18n::install(arguments.language);
    let roots = platform::current_user()?;
    if let Some(path) = arguments.restore {
        let plan = serde_json::from_slice::<discovery::Plan>(&std::fs::read(&path)?)?;
        eprintln!("{}", discovery::preview(&plan));
        eprintln!(
            "{}",
            orbit_i18n::text(
                "Recovery restores staged paths only; permanently deleted files cannot be recovered."
            )
        );
        confirm(arguments.yes)?;
        execution::rollback(&plan)?;
        std::fs::remove_file(path)?;
        return Ok(());
    }
    let level = match arguments.level {
        Some(level) => level.into(),
        None => {
            eprintln!(
                "{}",
                orbit_i18n::text(
                    "1. All caches only\n2. Configuration and caches (keep instances)\n3. Everything, including instances and worlds"
                )
            );
            match prompt(&orbit_i18n::text("Cleanup level [1]:"))?.as_str() {
                "" | "1" => discovery::CleanupLevel::Cache,
                "2" => discovery::CleanupLevel::Settings,
                "3" => discovery::CleanupLevel::All,
                _ => return Err("invalid cleanup level".into()),
            }
        }
    };
    let plan = discovery::discover(roots, level)?;
    if arguments.dry_run {
        if arguments.output_format == Output::Json {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        } else {
            println!("{}", discovery::preview(&plan));
        }
        return Ok(());
    }
    eprintln!("{}", discovery::preview(&plan));
    confirm(arguments.yes)?;
    let recovery_directory = platform::recovery_directory()?;
    orbit_installer_actions::transaction::check_root(&recovery_directory)?;
    std::fs::create_dir_all(&recovery_directory)?;
    let mut recovery = tempfile::Builder::new()
        .prefix("orbit-cleanup-")
        .suffix(".json")
        .tempfile_in(&recovery_directory)?;
    serde_json::to_writer(&mut recovery, &plan)?;
    recovery.as_file().sync_all()?;
    let (file, recovery_path) = recovery.keep()?;
    drop(file);
    eprintln!(
        "{} {}",
        orbit_i18n::text("Recovery plan:"),
        recovery_path.display()
    );
    if let Err(error) = execution::stage(&plan) {
        return Err(format!("{error}; recovery plan: {}", recovery_path.display()).into());
    }
    if let Err(error) = execution::commit(&plan) {
        return Err(format!(
            "cleanup incomplete: {error}; recovery plan: {}",
            recovery_path.display()
        )
        .into());
    }
    std::fs::remove_file(recovery_path)?;
    match std::fs::remove_dir(recovery_directory) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => return Err(error.into()),
    }
    if arguments.output_format == Output::Json {
        println!("{}", serde_json::json!({"status":"cleaned", "level":level}));
    } else {
        println!(
            "{}",
            orbit_i18n::text(
                "User data cleanup completed. Remove the installed programs using your package manager."
            )
        );
    }
    Ok(())
}
fn main() {
    orbit_i18n::install(orbit_i18n::requested_from_args(std::env::args_os()));
    let matches = orbit_i18n::get_matches(Arguments::command());
    let arguments = Arguments::from_arg_matches(&matches).expect("Clap matches the derived schema");
    if let Err(error) = run(arguments) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
