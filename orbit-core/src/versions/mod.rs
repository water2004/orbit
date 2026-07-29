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

/// Whether a Loader version has enough structure for Orbit's numeric rules.
///
/// Loader-valid opaque versions remain valid package candidates. Orbit does
/// not invent a numeric core for them; the independent string rule still sees
/// the complete JAR-declared version.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NumericVersionAnalysis {
    Filterable {
        /// Normalized dotted numeric components.  Loader formats do not limit
        /// this to major/minor/patch.
        numeric_core: Vec<String>,
    },
    Unfilterable {
        reason: String,
    },
}

impl NumericVersionAnalysis {
    pub fn numeric_filterable(&self) -> bool {
        matches!(self, Self::Filterable { .. })
    }

    pub fn numeric_core(&self) -> Option<&[String]> {
        match self {
            Self::Filterable { numeric_core } => Some(numeric_core),
            Self::Unfilterable { .. } => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Filterable { .. } => None,
            Self::Unfilterable { reason } => Some(reason),
        }
    }
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
                // Fabric and Quilt allow wildcard components in constraints,
                // not in a mod's declared version.  Their Loader parsers fall
                // back to an opaque string when semantic parsing fails.
                if let Ok(v) = fabric::SemanticVersion::parse(raw, false) {
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

    /// Analyze only the numeric core used by a package's `version` rule.
    pub fn numeric_analysis(&self) -> NumericVersionAnalysis {
        match self {
            Self::Fabric(version) => structured_numeric_core(&version.raw),
            Self::Maven(version) => structured_numeric_core(&version.to_string()),
            Self::Generic(raw) => NumericVersionAnalysis::Unfilterable {
                reason: format!(
                    "'{raw}' is an opaque Loader version, not a semantic numeric version"
                ),
            },
        }
    }

    /// Stable, deterministic text choices for the GUI string-rule editor.
    pub fn string_tokens(&self) -> Vec<String> {
        let raw = self.to_string();
        let mut tokens = vec![raw.clone()];
        tokens.extend(
            raw.split(|character: char| !character.is_ascii_alphanumeric())
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

fn structured_numeric_core(raw: &str) -> NumericVersionAnalysis {
    let bytes = raw.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return NumericVersionAnalysis::Unfilterable {
            reason: format!("'{raw}' does not start with a numeric version component"),
        };
    }

    let mut components = Vec::new();
    let mut cursor = 0;
    loop {
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        let component = &raw[start..cursor];
        let normalized = component.trim_start_matches('0');
        components.push(if normalized.is_empty() {
            "0".to_string()
        } else {
            normalized.to_string()
        });

        if bytes.get(cursor) == Some(&b'.') && bytes.get(cursor + 1).is_some_and(u8::is_ascii_digit)
        {
            cursor += 1;
            continue;
        }
        break;
    }

    let remainder = &raw[cursor..];
    if remainder.starts_with("..") || remainder == "." {
        return NumericVersionAnalysis::Unfilterable {
            reason: format!("'{raw}' has an incomplete dotted numeric component"),
        };
    }
    NumericVersionAnalysis::Filterable {
        numeric_core: components,
    }
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
mod numeric_analysis_tests {
    use super::*;

    #[test]
    fn numeric_core_can_have_arbitrarily_many_components() {
        let analysis = Version::parse("1.2.3.4-beta.1+mc26", LoaderKind::Fabric).numeric_analysis();
        assert_eq!(
            analysis,
            NumericVersionAnalysis::Filterable {
                numeric_core: vec!["1".into(), "2".into(), "3".into(), "4".into()],
            }
        );
    }

    #[test]
    fn string_tokens_use_the_complete_raw_version() {
        assert_eq!(
            Version::parse("1.2.3-beta.1", LoaderKind::Fabric).string_tokens(),
            ["1.2.3-beta.1", "beta"]
        );
    }

    #[test]
    fn opaque_loader_versions_have_no_numeric_core() {
        let version = Version::parse("release-vNext", LoaderKind::Fabric);
        let analysis = version.numeric_analysis();
        assert!(matches!(
            analysis,
            NumericVersionAnalysis::Unfilterable { .. }
        ));
        assert!(analysis.reason().unwrap().contains("opaque"));
    }

    #[test]
    fn malformed_numeric_shape_is_not_invented() {
        let analysis = Version::parse("1..2-beta", LoaderKind::Forge).numeric_analysis();
        assert!(matches!(
            analysis,
            NumericVersionAnalysis::Unfilterable { .. }
        ));
    }
}
