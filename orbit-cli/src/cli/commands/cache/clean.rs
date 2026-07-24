use crate::cli::commands::CliContext;
use anyhow::Result;

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let summary = orbit_core::inspect_cache()?;
    if summary.files == 0 {
        println!("Cache is already empty.");
        return Ok(());
    }

    println!(
        "Cache contains {} file(s), {} at {}.",
        summary.files,
        format_bytes(summary.bytes),
        summary.path.display()
    );
    if ctx.dry_run {
        println!("[dry-run] Cache was not modified.");
        return Ok(());
    }
    if !ctx.yes && !confirm()? {
        println!("Cache clean cancelled.");
        return Ok(());
    }

    let cleaned = orbit_core::clean_cache()?;
    println!("Cleaned cache: freed {}.", format_bytes(cleaned.bytes));
    Ok(())
}

fn confirm() -> Result<bool> {
    eprint!("Delete all cached files? [y/N] ");
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
