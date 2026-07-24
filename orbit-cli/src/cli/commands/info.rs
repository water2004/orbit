use super::CliContext;
use anyhow::Result;

pub async fn handle(mod_name: String, platform: Option<String>, ctx: &CliContext) -> Result<()> {
    let (prefix_platform, slug) = if let Some(slug) = mod_name.strip_prefix("mr:") {
        (Some("modrinth"), slug)
    } else if let Some(slug) = mod_name.strip_prefix("cf:") {
        (Some("curseforge"), slug)
    } else {
        (None, mod_name.as_str())
    };
    let instance_dir = ctx.instance_dir()?;
    let providers =
        super::create_instance_providers(&instance_dir, platform.as_deref().or(prefix_platform))?;

    for provider in providers {
        match provider.get_mod_info(slug).await {
            Ok(info) => {
                print_info(provider.name(), &info);
                return Ok(());
            }
            Err(orbit_core::OrbitError::ModNotFound(_)) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("Could not find '{slug}' on any configured platform.")
}

fn print_info(provider: &str, info: &orbit_core::providers::ModInfo) {
    println!("{} ({provider})", info.name);
    println!("  id: {}", info.project_id);
    println!("  slug: {}", info.slug);
    println!("  description: {}", info.description);
    if !info.authors.is_empty() {
        println!("  authors: {}", info.authors.join(", "));
    }
    println!("  latest version: {}", info.latest_version);
    println!(
        "  client side: {}   server side: {}",
        side(info.client_side.as_ref()),
        side(info.server_side.as_ref())
    );
    println!(
        "  license: {}",
        info.license.as_deref().unwrap_or("unknown")
    );
    println!("  downloads: {}", info.downloads);
    if !info.categories.is_empty() {
        println!("  categories: {}", info.categories.join(", "));
    }

    if !info.recent_versions.is_empty() {
        println!("\n  Recent versions:");
        for version in &info.recent_versions {
            println!(
                "    {}   mc {}   {}   released {}",
                version.version,
                version.mc_versions.join(", "),
                version.loader,
                version.released_at
            );
        }
    }

    println!("\n  Dependencies:");
    if info.dependencies.is_empty() {
        println!("    (none)");
    } else {
        for dependency in &info.dependencies {
            let name = dependency
                .slug
                .as_deref()
                .or(dependency.project_id.as_deref())
                .unwrap_or("unknown");
            let kind = if dependency.required {
                "required"
            } else {
                "optional"
            };
            println!("    {name} ({kind})");
        }
    }
}

fn side(value: Option<&orbit_core::providers::SideSupport>) -> &str {
    match value {
        Some(orbit_core::providers::SideSupport::Required) => "required",
        Some(orbit_core::providers::SideSupport::Optional) => "optional",
        Some(orbit_core::providers::SideSupport::Unsupported) => "unsupported",
        None => "unknown",
    }
}
