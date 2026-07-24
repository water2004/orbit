use super::CliContext;
use anyhow::Result;

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
        super::install_interaction(ctx.dry_run, ctx.yes),
    )
    .await?;
    super::print_resolution_diagnostics(&report.diagnostics);
    super::print_resolution_warnings(&report.warnings);

    if ctx.dry_run {
        for package in &report.restored {
            println!("  [dry-run] would restore {package}");
        }
        for package in &report.removed {
            println!(
                "  [dry-run] would remove {} {} ({})",
                package.mod_id, package.version, package.filename
            );
        }
        println!(
            "Restore preview: {} to restore, {} to remove, {} already present, {} skipped.",
            report.restored.len(),
            report.removed.len(),
            report.already_present.len(),
            report.skipped.len()
        );
    } else {
        println!(
            "Installed {} mods, removed {} unselected package version(s), skipped {} already present and {} excluded by policy.",
            report.restored.len(),
            report.removed.len(),
            report.already_present.len(),
            report.skipped.len()
        );
    }
    Ok(())
}
