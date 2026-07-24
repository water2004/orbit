//! Forge/NeoForge TOML metadata parsing.

use indexmap::IndexMap;
use serde::Deserialize;

use super::{MetadataParser, ModLoader, ModMetadata};
use crate::error::OrbitError;

#[derive(Debug, Deserialize)]
struct ModsToml {
    #[serde(default)]
    license: Option<String>,
    #[serde(default, rename = "clientSideOnly")]
    client_side_only: bool,
    #[serde(default)]
    mods: Vec<ModInfo>,
    #[serde(default)]
    dependencies: IndexMap<String, Vec<Dependency>>,
}

#[derive(Debug, Deserialize)]
struct ModInfo {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(default)]
    version: String,
    #[serde(default, rename = "displayName")]
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    authors: Option<Authors>,
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

#[derive(Debug, Deserialize)]
struct Dependency {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(default, rename = "versionRange")]
    version_range: String,
    #[serde(default)]
    mandatory: Option<bool>,
    #[serde(default, rename = "type")]
    dependency_type: Option<String>,
}

impl Dependency {
    fn required(&self) -> bool {
        self.dependency_type
            .as_deref()
            .map(|kind| kind.eq_ignore_ascii_case("required"))
            .unwrap_or_else(|| self.mandatory.unwrap_or(true))
    }
}

pub(crate) struct ParsedForgeMetadata {
    pub metadata: ModMetadata,
    pub dependencies: Vec<(String, String, bool)>,
}

pub(crate) fn parse_for_loader(
    content: &str,
    loader: ModLoader,
    source_name: &str,
) -> Result<ParsedForgeMetadata, OrbitError> {
    let raw: ModsToml = toml::from_str(content)
        .map_err(|error| OrbitError::Other(anyhow::anyhow!("invalid {source_name}: {error}")))?;
    let primary = raw.mods.into_iter().next().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!("{source_name} has no [[mods]] entries"))
    })?;

    let dependencies: Vec<_> = raw
        .dependencies
        .get(&primary.mod_id)
        .into_iter()
        .flatten()
        .map(|dependency| {
            (
                dependency.mod_id.clone(),
                if dependency.version_range.is_empty() {
                    "*".to_string()
                } else {
                    dependency.version_range.clone()
                },
                dependency.required(),
            )
        })
        .collect();
    let dependency_map = dependencies
        .iter()
        .map(|(id, version, _)| (id.clone(), version.clone()))
        .collect();

    Ok(ParsedForgeMetadata {
        metadata: ModMetadata {
            id: primary.mod_id,
            name: primary.display_name,
            version: primary.version,
            authors: primary.authors.map(Authors::into_vec).unwrap_or_default(),
            description: primary.description,
            license: raw.license,
            environment: if raw.client_side_only {
                "client".to_string()
            } else {
                "both".to_string()
            },
            dependencies: dependency_map,
            embedded_jars: Vec::new(),
            loader,
            sha256: String::new(),
        },
        dependencies,
    })
}

pub struct ForgeParser;

impl MetadataParser for ForgeParser {
    fn target_file(&self) -> &str {
        "META-INF/mods.toml"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Forge
    }

    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError> {
        Ok(parse_for_loader(content, ModLoader::Forge, self.target_file())?.metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forge_metadata_and_optional_dependencies() {
        let metadata = r#"
modLoader = "javafml"
loaderVersion = "[47,)"
license = "LGPL-2.1"

[[mods]]
modId = "example"
version = "1.2.3"
displayName = "Example Mod"
authors = "Alice, Bob"
description = "An example"

[[dependencies.example]]
modId = "forge"
mandatory = true
versionRange = "[47,)"

[[dependencies.example]]
modId = "jei"
mandatory = false
versionRange = "[15,)"
"#;
        let parsed = parse_for_loader(metadata, ModLoader::Forge, "META-INF/mods.toml").unwrap();

        assert_eq!(parsed.metadata.id, "example");
        assert_eq!(parsed.metadata.authors, ["Alice", "Bob"]);
        assert_eq!(parsed.metadata.license.as_deref(), Some("LGPL-2.1"));
        assert_eq!(
            parsed.dependencies,
            [
                ("forge".to_string(), "[47,)".to_string(), true),
                ("jei".to_string(), "[15,)".to_string(), false),
            ]
        );
    }

    #[test]
    fn only_uses_dependencies_of_primary_mod() {
        let metadata = r#"
[[mods]]
modId = "first"
[[mods]]
modId = "second"
[[dependencies.second]]
modId = "not-for-first"
"#;
        let parsed = parse_for_loader(metadata, ModLoader::Forge, "META-INF/mods.toml").unwrap();
        assert!(parsed.dependencies.is_empty());
    }
}
