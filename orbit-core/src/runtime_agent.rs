use crate::error::RuntimeDataError;
use crate::loader::LoaderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeSourceCapability {
    File,
    Union,
}

impl CodeSourceCapability {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Union => "union",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleIdentityCapability {
    Quilt,
}

impl ModuleIdentityCapability {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Quilt => "quilt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeAgentCapabilities {
    pub(crate) code_sources: &'static [CodeSourceCapability],
    pub(crate) module_identity: Option<ModuleIdentityCapability>,
    pub(crate) system_library_property: Option<&'static str>,
    pub(crate) java_range: [u32; 2],
}

const VERIFIED_JAVA: [u32; 2] = [8, 25];

#[derive(Debug, Clone, Copy)]
struct CapabilityRange {
    loader: LoaderKind,
    minimum: [u32; 3],
    maximum: [u32; 3],
    capabilities: RuntimeAgentCapabilities,
}

const FILE: &[CodeSourceCapability] = &[CodeSourceCapability::File];
const FILE_AND_UNION: &[CodeSourceCapability] =
    &[CodeSourceCapability::File, CodeSourceCapability::Union];

// These are compatibility facts, not feature guesses. Extend a range only
// after checking that Loader line's class-definition and nested-JAR behavior.
const CAPABILITY_RANGES: &[CapabilityRange] = &[
    CapabilityRange {
        loader: LoaderKind::Fabric,
        minimum: [0, 4, 0],
        maximum: [0, 19, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE,
            module_identity: None,
            system_library_property: Some("fabric.systemLibraries"),
            java_range: VERIFIED_JAVA,
        },
    },
    CapabilityRange {
        loader: LoaderKind::Quilt,
        minimum: [0, 12, 0],
        maximum: [0, 17, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE,
            module_identity: None,
            system_library_property: Some("loader.systemLibraries"),
            java_range: VERIFIED_JAVA,
        },
    },
    CapabilityRange {
        loader: LoaderKind::Quilt,
        minimum: [0, 18, 0],
        maximum: [0, 30, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE,
            module_identity: Some(ModuleIdentityCapability::Quilt),
            system_library_property: Some("loader.systemLibraries"),
            java_range: VERIFIED_JAVA,
        },
    },
    CapabilityRange {
        loader: LoaderKind::Forge,
        minimum: [14, 0, 0],
        maximum: [36, u32::MAX, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE,
            module_identity: None,
            system_library_property: None,
            java_range: VERIFIED_JAVA,
        },
    },
    CapabilityRange {
        loader: LoaderKind::Forge,
        minimum: [37, 0, 0],
        maximum: [64, u32::MAX, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE_AND_UNION,
            module_identity: None,
            system_library_property: None,
            java_range: VERIFIED_JAVA,
        },
    },
    // The short NeoForge version scheme follows the target Minecraft line.
    CapabilityRange {
        loader: LoaderKind::NeoForge,
        minimum: [20, 2, 0],
        maximum: [21, 11, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE_AND_UNION,
            module_identity: None,
            system_library_property: None,
            java_range: VERIFIED_JAVA,
        },
    },
    CapabilityRange {
        loader: LoaderKind::NeoForge,
        minimum: [26, 1, 0],
        maximum: [26, 2, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE_AND_UNION,
            module_identity: None,
            system_library_property: None,
            java_range: VERIFIED_JAVA,
        },
    },
    // NeoForge's original 1.20.1 line retained Forge's 47.x version scheme.
    CapabilityRange {
        loader: LoaderKind::NeoForge,
        minimum: [47, 1, 0],
        maximum: [47, 1, u32::MAX],
        capabilities: RuntimeAgentCapabilities {
            code_sources: FILE_AND_UNION,
            module_identity: None,
            system_library_property: None,
            java_range: VERIFIED_JAVA,
        },
    },
];

pub(crate) fn capabilities_for(
    loader: LoaderKind,
    minecraft_version: &str,
    loader_version: &str,
) -> Result<RuntimeAgentCapabilities, RuntimeDataError> {
    let version = parse_loader_version(minecraft_version, loader_version).ok_or_else(|| {
        RuntimeDataError::UnsupportedRuntimeAgent {
            loader: loader.as_str().to_string(),
            version: loader_version.to_string(),
        }
    })?;
    CAPABILITY_RANGES
        .iter()
        .find(|range| {
            range.loader == loader && version >= range.minimum && version <= range.maximum
        })
        .map(|range| range.capabilities)
        .ok_or_else(|| RuntimeDataError::UnsupportedRuntimeAgent {
            loader: loader.as_str().to_string(),
            version: loader_version.to_string(),
        })
}

fn parse_loader_version(minecraft_version: &str, value: &str) -> Option<[u32; 3]> {
    let value = value
        .strip_prefix(minecraft_version)
        .and_then(|suffix| suffix.strip_prefix('-'))
        .unwrap_or(value);
    let mut output = [0_u32; 3];
    let mut count = 0;
    for part in value.split('.') {
        if count == output.len() {
            break;
        }
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            break;
        }
        output[count] = digits.parse().ok()?;
        count += 1;
        if digits.len() != part.len() {
            break;
        }
    }
    (count >= 2).then_some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_source_model_at_the_forge_secure_jar_boundary() {
        let legacy = capabilities_for(LoaderKind::Forge, "1.16.5", "36.2.42").unwrap();
        let modular = capabilities_for(LoaderKind::Forge, "1.17.1", "37.1.1").unwrap();
        assert_eq!(legacy.code_sources, FILE);
        assert_eq!(modular.code_sources, FILE_AND_UNION);
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
    fn accepts_both_neoforge_version_schemes() {
        assert!(capabilities_for(LoaderKind::NeoForge, "1.20.1", "47.1.106").is_ok());
        assert!(capabilities_for(LoaderKind::NeoForge, "1.21.1", "21.1.200").is_ok());
        assert!(capabilities_for(LoaderKind::NeoForge, "26.1.2", "26.1.2.94").is_ok());
    }

    #[test]
    fn strips_a_launcher_style_minecraft_prefix() {
        assert_eq!(
            parse_loader_version("1.20.1", "1.20.1-47.4.22"),
            Some([47, 4, 22])
        );
    }

    #[test]
    fn rejects_unverified_future_loader_lines() {
        let error = capabilities_for(LoaderKind::Forge, "27.1", "65.0.0").unwrap_err();
        assert!(error.to_string().contains("not been verified"));
    }
}
