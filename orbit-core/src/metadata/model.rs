//! Loader-independent metadata used by every parser and resolver path.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Physical runtime side used by loader metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Client,
    Server,
    #[default]
    Both,
}

impl Environment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
            Self::Both => "both",
        }
    }

    pub fn applies_to(self, target: Self) -> bool {
        self == Self::Both || target == Self::Both || self == target
    }

    pub fn union(self, other: Self) -> Self {
        if self == other { self } else { Self::Both }
    }
}

impl FromStr for Environment {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "client" => Ok(Self::Client),
            "server" => Ok(Self::Server),
            "both" => Ok(Self::Both),
            _ => Err(format!(
                "invalid dependency environment '{value}'; expected client, server, or both"
            )),
        }
    }
}

/// Semantic role of an edge in loader metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    #[default]
    Required,
    Optional,
    Recommended,
    Suggested,
    Incompatible,
    Discouraged,
}

impl DependencyKind {
    pub fn installs_target(self) -> bool {
        self == Self::Required
    }

    pub fn validates_if_present(self) -> bool {
        matches!(self, Self::Required | Self::Optional)
    }
}

/// Relative loading order requested by a mod dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DependencyOrdering {
    Before,
    After,
    #[default]
    None,
}

/// Loader policy for a mod discovered inside a top-level package JAR.
///
/// Top-level files are selected by Orbit's package graph. This policy only
/// affects contained candidates after their owner has been selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModLoadCondition {
    /// The owner requires one loaded candidate for this mod ID.
    Always,
    /// Prefer loading one provider, but allow omitting it when no candidate is compatible.
    #[default]
    IfPossible,
    /// Make the candidate available only when another loaded mod requires it.
    IfRequired,
}

/// A normalized dependency relation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModDependency {
    pub id: String,
    pub requirement: String,
    #[serde(default)]
    pub kind: DependencyKind,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub ordering: DependencyOrdering,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Quilt condition that disables this relation when it is satisfied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unless: Option<Box<DependencyExpression>>,
}

impl ModDependency {
    pub fn required(id: impl Into<String>, requirement: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            requirement: requirement.into(),
            kind: DependencyKind::Required,
            environment: Environment::Both,
            ordering: DependencyOrdering::None,
            reason: None,
            unless: None,
        }
    }
}

/// Loader-neutral dependency expression.
///
/// Most formats emit `Only`; Quilt additionally supports nested any/all groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "expression", content = "value", rename_all = "snake_case")]
pub enum DependencyExpression {
    Only(ModDependency),
    Any(Vec<DependencyExpression>),
    All(Vec<DependencyExpression>),
}

impl DependencyExpression {
    pub fn relations(&self) -> Vec<&ModDependency> {
        let mut relations = Vec::new();
        self.collect_relations(&mut relations);
        relations
    }

    fn collect_relations<'a>(&'a self, output: &mut Vec<&'a ModDependency>) {
        match self {
            Self::Only(dependency) => output.push(dependency),
            Self::Any(dependencies) | Self::All(dependencies) => {
                for dependency in dependencies {
                    dependency.collect_relations(output);
                }
            }
        }
    }
}

impl From<ModDependency> for DependencyExpression {
    fn from(value: ModDependency) -> Self {
        Self::Only(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvidedMod {
    pub id: String,
    /// `None` inherits the declaring mod's version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Versioned artifact declared by Forge-family Jar-in-Jar metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedArtifact {
    pub id: String,
    pub requirement: String,
    pub version: String,
    pub path: String,
    #[serde(default)]
    pub obfuscated: bool,
}

/// A single mod module declared by a loader metadata file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: String,
    pub environment: Environment,
    pub dependencies: Vec<DependencyExpression>,
    /// Loader-level aliases satisfied by this mod (for example Fabric `provides`).
    pub provides: Vec<ProvidedMod>,
    /// Selection policy when this metadata is discovered as a contained mod.
    pub load_condition: ModLoadCondition,
}

/// The language provider requested by a Forge-family metadata file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageLoaderRequirement {
    pub id: String,
    pub requirement: String,
}

/// Complete normalized contents of one loader metadata file.
#[derive(Debug, Clone)]
pub struct ModFileMetadata {
    pub loader: super::LoaderKind,
    pub license: Option<String>,
    /// Loader-declared archive entry used as the package icon.
    pub icon: Option<String>,
    pub language_loader: Option<LanguageLoaderRequirement>,
    pub mods: Vec<ModMetadata>,
    pub embedded_jars: Vec<String>,
    /// Values available to Forge-family `${file.<key>}` substitutions.
    pub substitution_properties: IndexMap<String, String>,
}
