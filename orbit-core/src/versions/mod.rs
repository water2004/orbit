//! Version parsing and comparison.
//!
//! Provides a unified version representation for the PubGrub resolver.

pub mod fabric;
pub mod maven;

use pubgrub::Ranges;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use crate::loader::{LoaderKind, VersionScheme};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub(super) enum CorePosition {
    Before,
    Concrete,
    After,
}

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

    /// Compare package-version precedence without using a textual suffix as an
    /// upgrade signal. Representation equality remains stricter and is used by
    /// exact constraints.
    pub fn cmp_precedence(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Fabric(left), Self::Fabric(right)) => left.cmp_precedence(right),
            (Self::Maven(left), Self::Maven(right)) => left.cmp_precedence(right),
            _ => self.cmp(other),
        }
    }

    /// All representations with the same numeric core as this version.
    pub(crate) fn precedence_class(&self) -> Ranges<Self> {
        match self {
            Self::Fabric(version) => fabric::precedence_class(version),
            Self::Maven(version) => maven::precedence_class(version),
            Self::Generic(_) => Ranges::singleton(self.clone()),
        }
    }

    /// Every version whose numeric core is strictly greater than this one.
    pub(crate) fn strictly_higher_precedence(&self) -> Ranges<Self> {
        match self {
            Self::Fabric(version) => fabric::strictly_higher_precedence(version),
            Self::Maven(version) => maven::strictly_higher_precedence(version),
            Self::Generic(_) => Ranges::strictly_higher_than(self.clone()),
        }
    }

    /// Raw text following the leading dotted numeric core.
    ///
    /// Orbit does not assign release-stage meaning to this text. Separators
    /// are removed, but `beta`, `mc26`, `fabric`, or any other author-chosen
    /// value remains ordinary text for the suffix expression engine.
    pub fn suffix(&self) -> Option<String> {
        suffix_from_raw(&self.to_string())
    }

    /// Stable, deterministic qualifier choices suitable for a structured UI.
    /// The full qualifier is retained alongside its textual components.
    pub fn suffix_tokens(&self) -> Vec<String> {
        let Some(suffix) = self.suffix() else {
            return Vec::new();
        };
        let mut tokens = vec![suffix.clone()];
        tokens.extend(
            suffix
                .split(|character: char| !character.is_ascii_alphanumeric())
                .filter(|token| {
                    token
                        .chars()
                        .any(|character| character.is_ascii_alphabetic())
                })
                .map(str::to_string),
        );
        tokens.sort();
        tokens.dedup();
        tokens
    }
}

fn suffix_from_raw(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let numeric_end = raw
        .char_indices()
        .take_while(|(_, character)| character.is_ascii_digit() || *character == '.')
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    let suffix = raw[numeric_end..].trim_start_matches(['-', '_', '.', '+']);
    (!suffix.is_empty()).then(|| suffix.to_string())
}

pub(super) fn has_explicit_suffix(raw: &str) -> bool {
    raw.split_once('+')
        .map_or(raw, |(version, _)| version)
        .contains('-')
}

pub(super) fn numeric_core(raw: &str) -> Option<Vec<String>> {
    let without_build = raw.split_once('+').map_or(raw, |(version, _)| version);
    let core = without_build
        .split_once('-')
        .map_or(without_build, |(core, _)| core);
    let mut components = Vec::new();
    for component in core.split('.') {
        if component.is_empty()
            || !component
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return None;
        }
        let normalized = component.trim_start_matches('0');
        components.push(if normalized.is_empty() {
            "0".to_string()
        } else {
            normalized.to_string()
        });
    }
    while components.len() > 1 && components.last().is_some_and(|component| component == "0") {
        components.pop();
    }
    Some(components)
}

pub(super) fn cmp_numeric_core(left: &[String], right: &[String]) -> Ordering {
    let count = left.len().max(right.len());
    for index in 0..count {
        let left = left.get(index).map(String::as_str).unwrap_or("0");
        let right = right.get(index).map(String::as_str).unwrap_or("0");
        let ordering = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod suffix_tests {
    use super::*;

    #[test]
    fn suffix_is_raw_text_after_the_numeric_core() {
        assert_eq!(
            Version::parse("1.2.3-beta.1+mc26", LoaderKind::Fabric).suffix(),
            Some("beta.1+mc26".to_string())
        );
        assert_eq!(
            Version::parse("1.2.3+mc26", LoaderKind::Fabric).suffix(),
            Some("mc26".to_string())
        );
        assert_eq!(
            Version::parse("1.2.3-SNAPSHOT", LoaderKind::Forge).suffix(),
            Some("SNAPSHOT".to_string())
        );
    }

    #[test]
    fn suffix_tokens_include_full_and_textual_components() {
        assert_eq!(
            Version::parse("1.2.3-beta.1", LoaderKind::Fabric).suffix_tokens(),
            ["beta", "beta.1"]
        );
        assert_eq!(
            Version::parse("1.2.3-SNAPSHOT", LoaderKind::Forge).suffix_tokens(),
            ["SNAPSHOT"]
        );
    }
}
