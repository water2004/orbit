use super::CliContext;
use anyhow::Result;

use crate::cli::output::{OutputFormat, info_view};

pub async fn handle(mod_name: String, platform: Option<String>, ctx: &CliContext) -> Result<()> {
    let (selected_platform, slug) = super::resolve_platform_target(&mod_name, platform.as_deref())?;
    let instance_dir = ctx.instance_dir()?;
    let providers = super::create_instance_providers(
        &instance_dir,
        selected_platform.as_deref(),
        &ctx.runtime,
    )?;

    for provider in providers {
        match provider.get_mod_info(slug).await {
            Ok(info) => {
                let view = info_view(provider.name(), &info);
                match ctx.output.format {
                    OutputFormat::Text => {
                        ctx.print_result(format_args!(
                            "{}",
                            crate::cli::output::mod_info_table(provider.name(), &info)
                        ));
                    }
                    OutputFormat::Json => {
                        ctx.print_json("info", &view);
                    }
                }
                return Ok(());
            }
            Err(orbit_core::OrbitError::ModNotFound(_)) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!(
        "{}",
        tr!(
            "Could not find '%{slug}' on any configured platform.",
            slug = slug
        )
    )
}
