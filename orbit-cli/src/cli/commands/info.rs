use super::CliContext;
use anyhow::Result;

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
                println!("{}", crate::cli::output::mod_info_table(provider.name(), &info));
                return Ok(());
            }
            Err(orbit_core::OrbitError::ModNotFound(_)) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("Could not find '{slug}' on any configured platform.")
}
