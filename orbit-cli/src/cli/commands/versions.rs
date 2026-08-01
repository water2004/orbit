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
        ctx.runtime.candidate_storage(),
        super::operation_progress(ctx),
    )
    .await?;
    let output = PackageVersionsOutput {
        package: report.package,
        constraint: report.constraint,
        string: report.string,
        policy: crate::cli::output::package_version_policy_view(&report.policy),
        selected_version: report.selected_version,
        candidates: report
            .candidates
            .into_iter()
            .map(|candidate| PackageVersionCandidateView {
                version: candidate.version,
                numeric_core: candidate.numeric_core,
                string_tokens: candidate.string_tokens,
                numeric_filterable: candidate.numeric_filterable,
                numeric_error: candidate.numeric_error,
                sources: candidate.sources,
                details: candidate.details,
                selected: candidate.selected,
                matches_constraint: candidate.matches_constraint,
            })
            .collect(),
    };
    match ctx.output.format {
        OutputFormat::Text => {
            ctx.print_result_line(format_args!("{}", package_versions_table(&output)))
        }
        OutputFormat::Json => ctx.print_json("versions", &output),
    }
    Ok(())
}
