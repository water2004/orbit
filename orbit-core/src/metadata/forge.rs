//! Forge-family TOML adapter.
//!
//! Forge and NeoForge share one normalized parser. Loader-specific differences
//! are limited to dependency-kind defaults and metadata filenames.

use indexmap::IndexMap;
use serde::Deserialize;

use super::{
    DependencyExpression, DependencyKind, DependencyOrdering, Environment,
    LanguageLoaderRequirement, MetadataParser, ModDependency, ModFileMetadata, ModLoader,
    ModMetadata,
};
use crate::error::OrbitError;

#[derive(Debug, Deserialize)]
struct ModsToml {
    #[serde(default, rename = "modLoader")]
    mod_loader: Option<String>,
    #[serde(default, rename = "loaderVersion")]
    loader_version: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default, rename = "clientSideOnly")]
    client_side_only: bool,
    #[serde(default)]
    properties: IndexMap<String, toml::Value>,
    #[serde(default)]
    mods: Vec<ModInfo>,
    #[serde(default)]
    dependencies: IndexMap<String, Vec<RawDependency>>,
    #[serde(default)]
    features: IndexMap<String, Vec<FeatureSet>>,
}

#[derive(Debug, Deserialize)]
struct ModInfo {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(default = "default_mod_version")]
    version: String,
    #[serde(default, rename = "displayName")]
    display_name: String,
    #[serde(default = "default_description")]
    description: String,
    #[serde(default)]
    authors: Option<Authors>,
    #[serde(default)]
    features: FeatureSet,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Authors {
    One(String),
    Many(Vec<String>),
}

impl Authors {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(authors) => authors
                .split(',')
                .map(str::trim)
                .filter(|author| !author.is_empty())
                .map(str::to_string)
                .collect(),
            Self::Many(authors) => authors,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FeatureSet {
    #[serde(default, rename = "java_version", alias = "javaVersion")]
    java_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDependency {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(default, rename = "versionRange")]
    version_range: String,
    #[serde(default)]
    mandatory: Option<bool>,
    #[serde(default, rename = "type")]
    dependency_type: Option<String>,
    #[serde(default)]
    ordering: Option<String>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

pub(crate) fn parse_for_loader(
    content: &str,
    loader: ModLoader,
    source_name: &str,
) -> Result<ModFileMetadata, OrbitError> {
    let raw: ModsToml = toml::from_str(content)
        .map_err(|error| OrbitError::Other(anyhow::anyhow!("invalid {source_name}: {error}")))?;
    if raw.mods.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "{source_name} has no [[mods]] entries"
        )));
    }
    let license = raw
        .license
        .clone()
        .filter(|license| !license.trim().is_empty())
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "{source_name} requires a non-empty license"
            ))
        })?;
    let language_loader = match raw.mod_loader.as_deref() {
        Some(id) if id.trim().is_empty() => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "{source_name} contains an empty modLoader"
            )));
        }
        Some(id) => Some(LanguageLoaderRequirement {
            id: id.to_string(),
            requirement: any_if_empty(raw.loader_version.clone()),
        }),
        None if loader == ModLoader::NeoForge => Some(LanguageLoaderRequirement {
            id: "javafml".to_string(),
            requirement: "*".to_string(),
        }),
        None => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "{source_name} requires modLoader"
            )));
        }
    };

    let mut mods = Vec::with_capacity(raw.mods.len());
    for info in raw.mods {
        validate_mod_id(&info.mod_id, source_name)?;
        let mut dependencies = raw
            .dependencies
            .get(&info.mod_id)
            .into_iter()
            .flatten()
            .map(|dependency| {
                normalize_dependency(dependency, loader, source_name)
                    .map(DependencyExpression::Only)
            })
            .collect::<Result<Vec<_>, _>>()?;

        if let Some(requirement) = info.features.java_version {
            dependencies.push(java_requirement(requirement).into());
        }
        for feature_set in raw.features.get(&info.mod_id).into_iter().flatten() {
            if let Some(requirement) = &feature_set.java_version {
                dependencies.push(java_requirement(requirement.clone()).into());
            }
        }

        let display_name = if info.display_name.is_empty() {
            info.mod_id.clone()
        } else {
            info.display_name
        };
        mods.push(ModMetadata {
            id: info.mod_id,
            name: display_name,
            version: info.version,
            authors: info.authors.map(Authors::into_vec).unwrap_or_default(),
            description: info.description,
            environment: if raw.client_side_only {
                Environment::Client
            } else {
                Environment::Both
            },
            dependencies,
            provides: Vec::new(),
        });
    }

    let substitution_properties = raw
        .properties
        .into_iter()
        .filter_map(|(key, value)| scalar_to_string(value).map(|value| (key, value)))
        .collect();

    Ok(ModFileMetadata {
        loader,
        license: Some(license),
        language_loader,
        mods,
        embedded_jars: Vec::new(),
        substitution_properties,
    })
}

fn normalize_dependency(
    dependency: &RawDependency,
    loader: ModLoader,
    source_name: &str,
) -> Result<ModDependency, OrbitError> {
    validate_mod_id(&dependency.mod_id, source_name)?;
    let kind = match dependency.dependency_type.as_deref() {
        Some(kind) => parse_dependency_kind(kind, source_name)?,
        None => match dependency.mandatory {
            Some(true) => DependencyKind::Required,
            Some(false) => DependencyKind::Optional,
            None if loader == ModLoader::NeoForge => DependencyKind::Required,
            None => {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "{source_name} dependency '{}' is missing mandatory",
                    dependency.mod_id
                )));
            }
        },
    };
    let environment = match dependency.side.as_deref().unwrap_or("BOTH") {
        side if side.eq_ignore_ascii_case("CLIENT") => Environment::Client,
        side if side.eq_ignore_ascii_case("SERVER") => Environment::Server,
        side if side.eq_ignore_ascii_case("BOTH") => Environment::Both,
        side => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "{source_name} dependency '{}' has invalid side '{side}'",
                dependency.mod_id
            )));
        }
    };
    let ordering = match dependency.ordering.as_deref().unwrap_or("NONE") {
        ordering if ordering.eq_ignore_ascii_case("BEFORE") => DependencyOrdering::Before,
        ordering if ordering.eq_ignore_ascii_case("AFTER") => DependencyOrdering::After,
        ordering if ordering.eq_ignore_ascii_case("NONE") => DependencyOrdering::None,
        ordering => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "{source_name} dependency '{}' has invalid ordering '{ordering}'",
                dependency.mod_id
            )));
        }
    };

    Ok(ModDependency {
        id: dependency.mod_id.clone(),
        requirement: any_if_empty(dependency.version_range.clone()),
        kind,
        environment,
        ordering,
        reason: dependency.reason.clone(),
        unless: None,
    })
}

fn parse_dependency_kind(kind: &str, source_name: &str) -> Result<DependencyKind, OrbitError> {
    match kind.to_ascii_lowercase().as_str() {
        "required" => Ok(DependencyKind::Required),
        "optional" => Ok(DependencyKind::Optional),
        "incompatible" => Ok(DependencyKind::Incompatible),
        "discouraged" => Ok(DependencyKind::Discouraged),
        _ => Err(OrbitError::Other(anyhow::anyhow!(
            "{source_name} has invalid dependency type '{kind}'"
        ))),
    }
}

fn validate_mod_id(id: &str, source_name: &str) -> Result<(), OrbitError> {
    let valid = (2..=64).contains(&id.len())
        && id.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' && index > 0
        })
        && id.as_bytes()[0].is_ascii_lowercase();
    if valid {
        Ok(())
    } else {
        Err(OrbitError::Other(anyhow::anyhow!(
            "{source_name} contains invalid mod id '{id}'"
        )))
    }
}

fn java_requirement(requirement: String) -> ModDependency {
    ModDependency::required("java", any_if_empty(requirement))
}

fn any_if_empty(value: String) -> String {
    if value.trim().is_empty() {
        "*".to_string()
    } else {
        value
    }
}

fn scalar_to_string(value: toml::Value) -> Option<String> {
    match value {
        toml::Value::String(value) => Some(value),
        toml::Value::Integer(value) => Some(value.to_string()),
        toml::Value::Float(value) => Some(value.to_string()),
        toml::Value::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn default_mod_version() -> String {
    "1".to_string()
}

fn default_description() -> String {
    "MISSING DESCRIPTION".to_string()
}

pub struct ForgeParser;

impl MetadataParser for ForgeParser {
    fn target_file(&self) -> &str {
        "META-INF/mods.toml"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Forge
    }

    fn parse(&self, content: &str) -> Result<ModFileMetadata, OrbitError> {
        parse_for_loader(content, ModLoader::Forge, self.target_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_multiple_mods_and_full_dependency_semantics() {
        let parsed = parse_for_loader(
            r#"
modLoader = "javafml"
loaderVersion = "[47,)"
license = "MIT"
properties = { build = "1.2.3" }

[[mods]]
modId = "first"
version = "${file.build}"
features = { java_version = "[17,)" }

[[mods]]
modId = "second"

[[dependencies.first]]
modId = "forge"
mandatory = true
versionRange = "[47,48)"
ordering = "AFTER"
side = "CLIENT"
"#,
            ModLoader::Forge,
            "META-INF/mods.toml",
        )
        .unwrap();

        assert_eq!(parsed.mods.len(), 2);
        assert_eq!(parsed.mods[0].dependencies.len(), 2);
        assert_eq!(
            parsed.mods[0].dependencies[0],
            DependencyExpression::Only(ModDependency {
                ordering: DependencyOrdering::After,
                id: "forge".to_string(),
                requirement: "[47,48)".to_string(),
                kind: DependencyKind::Required,
                environment: Environment::Client,
                reason: None,
                unless: None,
            })
        );
        assert_eq!(parsed.substitution_properties["build"], "1.2.3".to_string());
    }

    #[test]
    fn rejects_missing_forge_mandatory_flag() {
        let error = parse_for_loader(
            r#"
modLoader = "javafml"
loaderVersion = "[47,)"
license = "MIT"
[[mods]]
modId = "example"
[[dependencies.example]]
modId = "forge"
"#,
            ModLoader::Forge,
            "META-INF/mods.toml",
        )
        .unwrap_err();
        assert!(error.to_string().contains("missing mandatory"));
    }
}
