//! Strongly typed loader identity and normalized loader-level semantics.
//!
//! The closed loader identity and invariant semantics live in the shared
//! compatibility crate. Core only expands the selected platform capability
//! scheme into solver package versions.

pub use orbit_compatibility::ModLoader as LoaderKind;
pub(crate) use orbit_compatibility::loader::{
    DependencyVersionScheme as VersionScheme, LoaderSemantics, NestedPriorityPolicy,
    PlatformCapabilityScheme,
};

pub(crate) fn semantics(loader: LoaderKind) -> LoaderSemantics {
    orbit_compatibility::loader::semantics(loader)
}

pub(crate) fn platform_capabilities(
    loader: LoaderKind,
    loader_version: &str,
) -> Result<Vec<(&'static str, String)>, String> {
    let capabilities = match semantics(loader).platform_capabilities {
        PlatformCapabilityScheme::MirrorLoader { package } => {
            vec![(package, loader_version.to_string())]
        }
        PlatformCapabilityScheme::ForgeMajor => {
            let major = orbit_compatibility::NumericVersion::parse(loader_version)
                .ok_or_else(|| {
                    format!(
                        "Forge loader version '{loader_version}' has no numeric major component"
                    )
                })?
                .major()
                .to_string();
            vec![("javafml", major.clone()), ("lowcodefml", major)]
        }
        PlatformCapabilityScheme::NeoForgeFmlOne => vec![
            ("javafml", "1".to_string()),
            ("lowcodefml", "1".to_string()),
        ],
    };
    Ok(capabilities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_names_roundtrip_through_serde_and_from_str() {
        for loader in LoaderKind::ALL {
            assert_eq!(loader.as_str().parse::<LoaderKind>().unwrap(), loader);
            assert_eq!(
                serde_json::from_str::<LoaderKind>(&serde_json::to_string(&loader).unwrap())
                    .unwrap(),
                loader
            );
        }
    }

    #[test]
    fn every_loader_has_exactly_one_semantics_row() {
        for loader in LoaderKind::ALL {
            assert!(!semantics(loader).canonical_package.is_empty());
        }
    }

    #[test]
    fn malformed_forge_version_is_not_reused_as_a_capability_version() {
        assert!(platform_capabilities(LoaderKind::Forge, "unknown").is_err());
    }
}
