use anyhow::Result;

use super::CliContext;
use crate::cli::output::{
    OutputFormat, PackageVersionCandidateView, PackageVersionsOutput, package_versions_table,
};

pub async fn handle(package: String, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
    let report = orbit_core::list_package_versions(
        &instance_dir,
        &package,
        &providers,
        ctx.runtime.jar_cache(),
        super::operation_progress(ctx),
    )
    .await?;
    let output = PackageVersionsOutput {
        package: report.package,
        constraint: report.constraint,
        selected_version: report.selected_version,
        candidates: report
            .candidates
            .into_iter()
            .map(|candidate| PackageVersionCandidateView {
                version: candidate.version,
                sources: candidate.sources,
                details: candidate.details,
                selected: candidate.selected,
                matches_constraint: candidate.matches_constraint,
            })
            .collect(),
    };
    match ctx.output.format {
        OutputFormat::Text => println!("{}", package_versions_table(&output)),
        OutputFormat::Json => crate::cli::output::print_json("versions", &output),
    }
    Ok(())
}
