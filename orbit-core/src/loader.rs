//! Strongly typed loader identity and normalized loader-level semantics.
//!
//! Persistent formats and CLI arguments use strings at their boundaries. Core
//! services parse those strings once and pass `LoaderKind` thereafter.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VersionScheme {
    FabricPredicate,
    MavenRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NestedPriorityPolicy {
    ParentOrder,
    Independent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LoaderSemantics {
    pub version_scheme: VersionScheme,
    pub nested_priority: NestedPriorityPolicy,
    pub canonical_package: &'static str,
}

impl LoaderKind {
    pub const ALL: [Self; 4] = [Self::Fabric, Self::Quilt, Self::Forge, Self::NeoForge];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
            Self::Forge => "forge",
            Self::NeoForge => "neoforge",
        }
    }

    pub(crate) const fn semantics(self) -> LoaderSemantics {
        match self {
            Self::Fabric => LoaderSemantics {
                version_scheme: VersionScheme::FabricPredicate,
                nested_priority: NestedPriorityPolicy::ParentOrder,
                canonical_package: "fabricloader",
            },
            Self::Quilt => LoaderSemantics {
                version_scheme: VersionScheme::FabricPredicate,
                nested_priority: NestedPriorityPolicy::Independent,
                canonical_package: "quilt_loader",
            },
            Self::Forge => LoaderSemantics {
                version_scheme: VersionScheme::MavenRange,
                nested_priority: NestedPriorityPolicy::Independent,
                canonical_package: "forge",
            },
            Self::NeoForge => LoaderSemantics {
                version_scheme: VersionScheme::MavenRange,
                nested_priority: NestedPriorityPolicy::Independent,
                canonical_package: "neoforge",
            },
        }
    }
}

impl std::fmt::Display for LoaderKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LoaderKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fabric" => Ok(Self::Fabric),
            "quilt" => Ok(Self::Quilt),
            "forge" => Ok(Self::Forge),
            "neoforge" => Ok(Self::NeoForge),
            _ => Err(format!("unsupported loader '{value}'")),
        }
    }
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
}
