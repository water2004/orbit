//! Version parsing and comparison.
//!
//! Provides a unified version representation for the PubGrub resolver.

pub mod fabric;

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use pubgrub::Ranges;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Version {
    Lowest,
    Fabric(fabric::SemanticVersion),
    Generic(String),
}

impl Hash for Version {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Lowest => state.write_u8(0),
            Self::Fabric(f) => {
                state.write_u8(1);
                f.hash(state);
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
        if let (Self::Lowest, Self::Lowest) = (self, other) { return Ordering::Equal; }
        if let Self::Lowest = self { return Ordering::Less; }
        if let Self::Lowest = other { return Ordering::Greater; }

        match (self, other) {
            (Self::Fabric(a), Self::Fabric(b)) => a.cmp(b),
            (Self::Generic(a), Self::Generic(b)) => a.cmp(b),
            (Self::Fabric(_), Self::Generic(_)) => Ordering::Less,
            (Self::Generic(_), Self::Fabric(_)) => Ordering::Greater,
            _ => unreachable!(),
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lowest => write!(f, "0.0.0-lowest"),
            Self::Fabric(v) => write!(f, "{}", v.raw),
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
    pub fn parse(raw: &str, loader: &str) -> Self {
        match loader {
            "fabric" | "quilt" => {
                if let Ok(v) = fabric::SemanticVersion::parse(raw, true) {
                    Self::Fabric(v)
                } else {
                    Self::Generic(raw.to_string())
                }
            }
            _ => Self::Generic(raw.to_string()),
        }
    }

    pub fn parse_constraint(raw: &str, loader: &str) -> Ranges<Self> {
        let constraint = raw.trim();
        if constraint.is_empty() || constraint == "*" {
            return Ranges::full();
        }

        match loader {
            "fabric" | "quilt" => fabric::parse_constraint(constraint),
            _ => Ranges::singleton(Self::parse(constraint, loader)),
        }
    }
}
