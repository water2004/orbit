//! Runtime-agent capabilities selected from the shared compatibility table.

use crate::error::RuntimeDataError;
use crate::loader::LoaderKind;

pub(crate) use orbit_compatibility::runtime_agent::Capabilities as RuntimeAgentCapabilities;

#[cfg(test)]
use orbit_compatibility::runtime_agent::{CodeSourceCapability, ModuleIdentityCapability};

pub(crate) fn capabilities_for(
    loader: LoaderKind,
    minecraft_version: &str,
    loader_version: &str,
) -> Result<RuntimeAgentCapabilities, RuntimeDataError> {
    orbit_compatibility::runtime_agent::select(loader, minecraft_version, loader_version).map_err(
        |_| RuntimeDataError::UnsupportedRuntimeAgent {
            loader: loader.as_str().to_string(),
            version: loader_version.to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_source_model_at_the_forge_secure_jar_boundary() {
        let legacy = capabilities_for(LoaderKind::Forge, "1.16.5", "36.2.42").unwrap();
        let modular = capabilities_for(LoaderKind::Forge, "1.17.1", "37.1.1").unwrap();
        assert_eq!(legacy.code_sources, &[CodeSourceCapability::File]);
        assert_eq!(
            modular.code_sources,
            &[CodeSourceCapability::File, CodeSourceCapability::Union]
        );
    }

    #[test]
    fn selects_quilt_native_module_identity_only_when_published() {
        let old = capabilities_for(LoaderKind::Quilt, "1.19.2", "0.17.8").unwrap();
        let current = capabilities_for(LoaderKind::Quilt, "1.20.1", "0.18.1").unwrap();
        assert_eq!(old.module_identity, None);
        assert_eq!(
            current.module_identity,
            Some(ModuleIdentityCapability::Quilt)
        );
    }

    #[test]
    fn accepts_all_verified_neoforge_version_schemes() {
        assert!(capabilities_for(LoaderKind::NeoForge, "1.20.1", "47.1.106").is_ok());
        assert!(capabilities_for(LoaderKind::NeoForge, "1.21.1", "21.1.200").is_ok());
        assert!(capabilities_for(LoaderKind::NeoForge, "26.1.2", "26.1.2.94").is_ok());
    }

    #[test]
    fn rejects_unverified_future_loader_lines() {
        let error = capabilities_for(LoaderKind::Forge, "27.1", "65.0.0").unwrap_err();
        assert!(error.to_string().contains("not been verified"));
    }
}
