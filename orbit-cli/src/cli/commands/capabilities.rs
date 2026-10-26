use super::CliContext;
use crate::cli::output::OutputFormat;
use anyhow::Result;

pub async fn handle(ctx: &CliContext) -> Result<()> {
    let support = orbit_core::feature_support::instance_feature_support(&ctx.instance_dir()?)?;
    match ctx.output.format {
        OutputFormat::Json => ctx.print_json(&support),
        OutputFormat::Text => {
            for (name, feature) in [
                (tr!("Bytecode audit"), support.audit),
                (tr!("Runtime data observation"), support.runtime_observation),
            ] {
                let state = if feature.available {
                    tr!("Available").into_owned()
                } else {
                    feature.reason.unwrap_or_default()
                };
                ctx.print_result_line(format_args!("{name}: {state}"));
            }
        }
    }
    Ok(())
}
