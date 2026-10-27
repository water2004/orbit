use super::CliContext;
use crate::cli::output::OutputFormat;
use orbit_core::updates::Status;

pub async fn handle(current_version: Option<String>, ctx: &CliContext) -> anyhow::Result<()> {
    let report = orbit_core::updates::check(
        ctx.runtime.config(),
        current_version
            .as_deref()
            .unwrap_or(env!("CARGO_PKG_VERSION")),
    )
    .await?;
    match ctx.output.format {
        OutputFormat::Json => ctx.print_json(&report),
        OutputFormat::Text => {
            let message = match report.status {
                Status::UpdateAvailable => tr!("A new Orbit release is available"),
                Status::UpToDate => tr!("Orbit is up to date"),
                Status::Ahead => tr!("This build is newer than the latest stable release"),
                Status::NoRelease => tr!("No published stable release is available"),
            };
            ctx.print_result_line(format_args!("{message} · {}", report.current_version));
            if let Some(release) = report.release {
                ctx.print_result_line(format_args!(
                    "{} · {}\n{}",
                    release.name, release.version, release.url
                ));
            }
        }
    }
    Ok(())
}
