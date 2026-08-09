use crate::cli::commands::CliContext;
use anyhow::Result;

use crate::cli::output::{CacheOutput, OutputFormat};

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let working_directory = std::env::current_dir()?;
    let protected_paths = [
        ctx.runtime.paths().config_file(),
        ctx.runtime.paths().instances_file(),
        working_directory.as_path(),
    ];
    let summary = orbit_core::inspect_cache(ctx.runtime.paths().cache_dir(), &protected_paths)?;
    if summary.files == 0 {
        match ctx.output.format {
            OutputFormat::Text => {
                ctx.print_result_line(format_args!("{}", tr!("Cache is already empty.")))
            }
            OutputFormat::Json => {
                ctx.print_json(
                    "cache",
                    &CacheOutput {
                        subcommand: "clean".to_string(),
                        dry_run: ctx.dry_run,
                        cache_path: summary.path.to_string_lossy().into_owned(),
                        files_before: summary.files,
                        bytes_before: summary.bytes,
                        files_removed: 0,
                        bytes_freed: 0,
                    },
                );
            }
        }
        return Ok(());
    }

    if ctx.output.format == OutputFormat::Text {
        ctx.print_result_line(format_args!(
            "{}",
            tr!(
                "Cache contains %{files} file(s), %{bytes} at %{path}.",
                files = summary.files,
                bytes = format_bytes(summary.bytes),
                path = summary.path.display()
            )
        ));
    }
    if ctx.dry_run {
        match ctx.output.format {
            OutputFormat::Text => {
                ctx.print_result_line(format_args!("{}", tr!("[dry-run] Cache was not modified.")))
            }
            OutputFormat::Json => {
                ctx.print_json(
                    "cache",
                    &CacheOutput {
                        subcommand: "clean".to_string(),
                        dry_run: true,
                        cache_path: summary.path.to_string_lossy().into_owned(),
                        files_before: summary.files,
                        bytes_before: summary.bytes,
                        files_removed: 0,
                        bytes_freed: 0,
                    },
                );
            }
        }
        return Ok(());
    }
    if !ctx.yes && ctx.output.format == OutputFormat::Text && !confirm()? {
        return Err(
            orbit_core::OrbitError::Cancelled(tr!("Cache clean cancelled.").into_owned()).into(),
        );
    }

    let cleaned = orbit_core::clean_cache(ctx.runtime.paths().cache_dir(), &protected_paths)?;
    match ctx.output.format {
        OutputFormat::Text => {
            ctx.print_result_line(format_args!(
                "{}",
                tr!(
                    "Cleaned cache: freed %{bytes}.",
                    bytes = format_bytes(cleaned.bytes)
                )
            ));
        }
        OutputFormat::Json => {
            ctx.print_json(
                "cache",
                &CacheOutput {
                    subcommand: "clean".to_string(),
                    dry_run: false,
                    cache_path: cleaned.path.to_string_lossy().into_owned(),
                    files_before: summary.files,
                    bytes_before: summary.bytes,
                    files_removed: summary.files,
                    bytes_freed: cleaned.bytes,
                },
            );
        }
    }
    Ok(())
}

fn confirm() -> Result<bool> {
    eprint!("{}", tr!("Delete all cached files? [y/N] "));
    use std::io::Write;
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}
