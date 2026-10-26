//! Optional analysis/observation support is not an installation or resolver gate.
use std::path::Path;

use orbit_compatibility::{CompatibilityError, ModLoader};
use serde::Serialize;

use crate::{OrbitError, workspace::ManifestFile};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureSupport {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceFeatureSupport {
    pub audit: FeatureSupport,
    pub runtime_observation: FeatureSupport,
}

pub fn instance_feature_support(instance: &Path) -> Result<InstanceFeatureSupport, OrbitError> {
    let manifest = ManifestFile::open(instance)?.inner;
    for_platform(
        manifest.project.loader_kind()?,
        &manifest.project.mc_version,
        &manifest.project.modloader_version,
    )
}

pub(crate) fn runtime_observation_support(instance: &Path) -> Result<FeatureSupport, OrbitError> {
    let manifest = ManifestFile::open(instance)?.inner;
    availability(orbit_compatibility::runtime_agent::select(
        manifest.project.loader_kind()?,
        &manifest.project.mc_version,
        &manifest.project.modloader_version,
    ))
}

pub(crate) fn for_platform(
    loader: ModLoader,
    minecraft: &str,
    version: &str,
) -> Result<InstanceFeatureSupport, OrbitError> {
    Ok(InstanceFeatureSupport {
        audit: availability(orbit_compatibility::audit::select(
            loader, minecraft, version,
        ))?,
        runtime_observation: availability(orbit_compatibility::runtime_agent::select(
            loader, minecraft, version,
        ))?,
    })
}

fn availability<T>(result: Result<T, CompatibilityError>) -> Result<FeatureSupport, OrbitError> {
    match result {
        Ok(_) => Ok(FeatureSupport {
            available: true,
            reason: None,
        }),
        Err(error @ CompatibilityError::Unsupported { .. }) => Ok(FeatureSupport {
            available: false,
            reason: Some(error.to_string()),
        }),
        Err(error) => Err(OrbitError::Other(error.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_optional_capabilities_are_values_not_a_global_loader_rejection() {
        let features = for_platform(ModLoader::Forge, "1.12.2", "14.23.5.2860").unwrap();
        assert!(!features.audit.available);
        assert!(features.runtime_observation.available);
        assert!(features.audit.reason.unwrap().contains("bytecode"));
        let future = for_platform(ModLoader::Forge, "27.1", "65.0.0").unwrap();
        assert!(!future.audit.available);
        assert!(!future.runtime_observation.available);
        assert!(
            for_platform(ModLoader::Fabric, "26.3", "0.20.1")
                .unwrap()
                .audit
                .available
        );
    }
}
