use super::CliContext;
use anyhow::Result;

use crate::cli::output::{OutputFormat, restore_view};

/// `orbit install` — 根据 orbit.toml + orbit.lock 还原全部模组。
/// 不接受 mod 名称参数（单个模组安装请用 `orbit add`）。
pub async fn handle(
    target: Option<String>,
    group: Option<String>,
    no_optional: bool,
    locked: bool,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let providers = if locked {
        Vec::new()
    } else {
        super::create_instance_providers(&instance_dir, None, &ctx.runtime)?
    };
    let report = orbit_core::restore_instance(
        &instance_dir,
        &providers,
        ctx.runtime.jar_cache(),
        orbit_core::RestoreOptions {
            target,
            group,
            no_optional,
            locked,
            dry_run: ctx.dry_run,
        },
        super::install_interaction(ctx),
    )
    .await?;
    if ctx.output.format == OutputFormat::Text {
        super::print_resolution_diagnostics(&report.diagnostics);
        super::print_resolution_warnings(&report.warnings);

        if ctx.dry_run {
            for package in &report.restored {
                println!(
                    "  {}",
                    tr!("[dry-run] would restore %{package}", package = package)
                );
            }
            if !report.removed.is_empty() {
                println!("\n{}", tr!("Packages to remove:"));
                println!(
                    "{}",
                    crate::cli::output::removed_packages_table(&report.removed)
                );
            }
            println!(
                "{}",
                tr!(
                    "Restore preview: %{restore} to restore, %{remove} to remove, %{present} already present, %{skipped} skipped.",
                    restore = report.restored.len(),
                    remove = report.removed.len(),
                    present = report.already_present.len(),
                    skipped = report.skipped.len()
                )
            );
        } else {
            println!(
                "{}",
                tr!(
                    "Installed %{installed} mods, removed %{removed} unselected package version(s), skipped %{present} already present and %{excluded} excluded by policy.",
                    installed = report.restored.len(),
                    removed = report.removed.len(),
                    present = report.already_present.len(),
                    excluded = report.skipped.len()
                )
            );
        }
        return Ok(());
    }

    let view = restore_view(&report, ctx.dry_run);
    crate::cli::output::print_json("install", &view);
    Ok(())
}
