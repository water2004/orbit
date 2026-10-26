use anyhow::{Context, Result};
use std::path::Path;

pub fn create_instance_providers(
    instance_dir: &Path,
    platform: Option<&str>,
    runtime: &orbit_core::RuntimeContext,
) -> Result<Vec<Box<dyn orbit_core::ModProvider>>> {
    let mut catalogs = if let Some(platform) = platform {
        vec![normalize_platform(platform).to_string()]
    } else {
        orbit_core::instance_config::discovery_catalogs(instance_dir)?
    };
    if platform.is_none()
        && let Ok(lockfile) = orbit_core::Lockfile::open(instance_dir)
    {
        for entry in &lockfile.inner.packages {
            for remote in &entry.remotes {
                let provider = remote.provider();
                if provider != "file" && !catalogs.iter().any(|item| item == provider) {
                    catalogs.push(provider.to_string());
                }
            }
            for source in &entry.artifact_sources {
                let provider = source.provider();
                if provider != "file" && !catalogs.iter().any(|item| item == provider) {
                    catalogs.push(provider.to_string());
                }
            }
        }
    }
    if platform.is_none()
        && let Ok(manifest) = orbit_core::ManifestFile::open(instance_dir)
    {
        for remote in manifest
            .inner
            .packages
            .values()
            .flat_map(|package| package.remotes.iter())
        {
            let provider = remote.provider();
            if provider != "file" && !catalogs.iter().any(|item| item == provider) {
                catalogs.push(provider.to_string());
            }
        }
    }
    orbit_core::providers::create_providers(&catalogs, runtime.config())
        .with_context(|| tr!("Failed to create providers").into_owned())
}

pub fn resolve_platform_target<'a>(
    input: &'a str,
    requested_platform: Option<&str>,
) -> Result<(Option<String>, &'a str)> {
    let (prefixed_platform, target) = if let Some(target) = input.strip_prefix("mr:") {
        (Some("modrinth"), target)
    } else if let Some(target) = input.strip_prefix("cf:") {
        (Some("curseforge"), target)
    } else {
        (None, input)
    };
    if target.is_empty() {
        anyhow::bail!(
            "{}",
            tr!("A platform prefix must be followed by a project slug or ID")
        );
    }

    let requested_platform = requested_platform.map(normalize_platform);
    if let (Some(prefixed), Some(requested)) = (prefixed_platform, requested_platform)
        && prefixed != requested
    {
        anyhow::bail!(
            "{}",
            tr!(
                "'%{input}' selects %{prefixed}, but --platform selects %{requested}; use one platform",
                input = input,
                prefixed = prefixed,
                requested = requested
            )
        );
    }
    Ok((
        prefixed_platform
            .or(requested_platform)
            .map(ToString::to_string),
        target,
    ))
}

pub(super) fn normalize_platform(platform: &str) -> &str {
    match platform {
        "mr" => "modrinth",
        "cf" => "curseforge",
        other => other,
    }
}

pub fn parse_package_remote(provider: &str, locator: &str) -> Result<orbit_core::PackageRemote> {
    match normalize_platform(provider) {
        "file" => Ok(orbit_core::PackageRemote::File {
            path: locator.to_string(),
        }),
        "modrinth" => Ok(orbit_core::PackageRemote::Modrinth {
            project_id: locator.to_string(),
        }),
        "curseforge" => Ok(orbit_core::PackageRemote::Curseforge {
            project_id: locator.parse().map_err(|_| {
                anyhow::anyhow!(
                    "{}",
                    tr!(
                        "CurseForge remotes require a numeric project ID, got '%{locator}'",
                        locator = locator
                    )
                )
            })?,
        }),
        other => anyhow::bail!(
            "{}",
            tr!("Unsupported package remote '%{remote}'", remote = other)
        ),
    }
}
