use anyhow::Result;

use super::CliContext;

/// `orbit fix` — discover the complete candidate universe, solve the package
/// graph, show the selected transaction, and apply it after confirmation.
pub async fn handle(ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
    let report = orbit_core::fix_instance(
        &instance_dir,
        &providers,
        ctx.runtime.candidate_storage(),
        ctx.dry_run,
        super::install_interaction(ctx),
    )
    .await?;
    super::print_transaction_result("fix", &report, ctx);
    Ok(())
}
