//! Version parsing and comparison.
//!
//! Provides a unified version representation for the PubGrub resolver.

pub mod fabric;
pub mod maven;

use pubgrub::Ranges;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use crate::loader::{LoaderKind, VersionScheme};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Version {
    Fabric(fabric::SemanticVersion),
    Maven(maven::MavenVersion),
    Generic(String),
}

impl Hash for Version {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Fabric(f) => {
                state.write_u8(0);
                f.hash(state);
            }
            Self::Maven(version) => {
                state.write_u8(1);
                version.hash(state);
            }
            Self::Generic(s) => {
                state.write_u8(2);
                s.hash(state);
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Fabric(a), Self::Fabric(b)) => a.cmp(b),
            (Self::Maven(a), Self::Maven(b)) => a.cmp(b),
            (Self::Generic(a), Self::Generic(b)) => a.cmp(b),
            (Self::Fabric(_), _) => Ordering::Less,
            (Self::Maven(_), Self::Fabric(_)) => Ordering::Greater,
            (Self::Maven(_), Self::Generic(_)) => Ordering::Less,
            (Self::Generic(_), _) => Ordering::Greater,
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fabric(v) => write!(f, "{}", v.raw),
            Self::Maven(v) => write!(f, "{v}"),
            Self::Generic(s) => write!(f, "{}", s),
        }
    }
}

impl Version {
    pub fn zero() -> Self {
        Self::Generic("0.0.0".to_string())
    }

    /// Parse a raw version string into a Version.
    /// The version string should come from the mod's own fabric.mod.json, not a platform release name.
    pub fn parse(raw: &str, loader: LoaderKind) -> Self {
        match loader.semantics().version_scheme {
            VersionScheme::FabricPredicate => {
                if let Ok(v) = fabric::SemanticVersion::parse(raw, true) {
                    Self::Fabric(v)
                } else {
                    Self::Generic(raw.to_string())
                }
            }
            VersionScheme::MavenRange => Self::Maven(maven::MavenVersion::parse(raw)),
        }
    }

    pub fn parse_constraint(raw: &str, loader: LoaderKind) -> Ranges<Self> {
        let constraint = raw.trim();
        if constraint.is_empty() || constraint == "*" {
            return Ranges::full();
        }

        match loader.semantics().version_scheme {
            VersionScheme::FabricPredicate => fabric::parse_constraint(constraint),
            VersionScheme::MavenRange => maven::parse_constraint(constraint),
        }
    }
}
