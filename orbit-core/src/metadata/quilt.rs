//! Quilt metadata parser for `quilt.mod.json`.

use indexmap::IndexMap;

use super::{MetadataParser, ModLoader, ModMetadata};
use crate::error::OrbitError;

pub(crate) struct ParsedQuiltMetadata {
    pub metadata: ModMetadata,
    pub dependencies: Vec<(String, String, bool)>,
}

pub(crate) fn parse_quilt(content: &str) -> Result<ParsedQuiltMetadata, OrbitError> {
    let value: serde_json::Value = serde_json::from_str(content).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!("invalid JSON in quilt.mod.json: {error}"))
    })?;
    let loader = value
        .get("quilt_loader")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!("quilt.mod.json has no quilt_loader object"))
        })?;
    let metadata = loader
        .get("metadata")
        .and_then(serde_json::Value::as_object);
    let dependencies = parse_dependencies(loader.get("depends"));
    let dependency_map: IndexMap<_, _> = dependencies
        .iter()
        .map(|(id, version, _)| (id.clone(), version.clone()))
        .collect();

    let authors = metadata
        .and_then(|metadata| metadata.get("contributors"))
        .map(parse_contributors)
        .unwrap_or_default();
    let license = metadata
        .and_then(|metadata| metadata.get("license"))
        .and_then(first_string);
    let environment = value
        .get("minecraft")
        .and_then(|minecraft| minecraft.get("environment"))
        .and_then(serde_json::Value::as_str)
        .map(map_environment)
        .unwrap_or_else(|| "both".to_string());
    let embedded_jars = parse_jars(loader.get("jars").or_else(|| value.get("jars")));

    Ok(ParsedQuiltMetadata {
        metadata: ModMetadata {
            id: string_field(loader, "id"),
            name: metadata
                .map(|metadata| string_field(metadata, "name"))
                .unwrap_or_default(),
            version: string_field(loader, "version"),
            authors,
            description: metadata
                .map(|metadata| string_field(metadata, "description"))
                .unwrap_or_default(),
            license,
            environment,
            dependencies: dependency_map,
            embedded_jars,
            loader: ModLoader::Quilt,
        },
        dependencies,
    })
}

fn string_field(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> String {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_contributors(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Object(contributors) => contributors.keys().cloned().collect(),
        serde_json::Value::Array(contributors) => {
            contributors.iter().filter_map(first_string).collect()
        }
        serde_json::Value::String(contributor) => vec![contributor.clone()],
        _ => Vec::new(),
    }
}

fn first_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Array(values) => values.first().and_then(first_string),
        serde_json::Value::Object(value) => value
            .get("name")
            .or_else(|| value.get("id"))
            .and_then(first_string),
        _ => None,
    }
}

fn parse_dependencies(value: Option<&serde_json::Value>) -> Vec<(String, String, bool)> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(dependencies) => {
            dependencies.iter().filter_map(parse_dependency).collect()
        }
        serde_json::Value::Object(dependencies) => dependencies
            .iter()
            .map(|(id, version)| (id.clone(), version_constraint(Some(version)), true))
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_dependency(value: &serde_json::Value) -> Option<(String, String, bool)> {
    match value {
        serde_json::Value::String(id) => Some((id.clone(), "*".to_string(), true)),
        serde_json::Value::Object(dependency) => {
            let id = dependency.get("id").and_then(first_string)?;
            let version = version_constraint(
                dependency
                    .get("versions")
                    .or_else(|| dependency.get("version")),
            );
            let required = !dependency
                .get("optional")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            Some((id, version, required))
        }
        _ => None,
    }
}

fn version_constraint(value: Option<&serde_json::Value>) -> String {
    let Some(value) = value else {
        return "*".to_string();
    };
    match value {
        serde_json::Value::String(version) => version.clone(),
        serde_json::Value::Array(versions) => {
            let versions: Vec<_> = versions.iter().filter_map(first_string).collect();
            if versions.is_empty() {
                "*".to_string()
            } else {
                versions.join(" || ")
            }
        }
        _ => "*".to_string(),
    }
}

fn parse_jars(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|jar| match jar {
            serde_json::Value::String(path) => Some(path.clone()),
            serde_json::Value::Object(jar) => jar.get("file").and_then(first_string),
            _ => None,
        })
        .collect()
}

fn map_environment(environment: &str) -> String {
    match environment {
        "*" | "universal" => "both",
        "client" => "client",
        "dedicated_server" | "server" => "server",
        other => other,
    }
    .to_string()
}

pub struct QuiltParser;

impl MetadataParser for QuiltParser {
    fn target_file(&self) -> &str {
        "quilt.mod.json"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Quilt
    }

    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError> {
        Ok(parse_quilt(content)?.metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quilt_metadata() {
        let metadata = r#"{
  "schema_version": 1,
  "quilt_loader": {
    "id": "example",
    "version": "1.0.0",
    "metadata": {
      "name": "Example",
      "description": "A Quilt mod",
      "contributors": {"Alice": "Owner", "Bob": "Developer"},
      "license": ["MIT"]
    },
    "depends": [
      {"id": "minecraft", "versions": [">=1.20", "<1.22"]},
      {"id": "optional-api", "optional": true}
    ],
    "jars": ["META-INF/jars/inner.jar"]
  },
  "minecraft": {"environment": "client"}
}"#;
        let parsed = parse_quilt(metadata).unwrap();

        assert_eq!(parsed.metadata.id, "example");
        assert_eq!(parsed.metadata.authors, ["Alice", "Bob"]);
        assert_eq!(parsed.metadata.environment, "client");
        assert_eq!(parsed.metadata.embedded_jars, ["META-INF/jars/inner.jar"]);
        assert_eq!(parsed.dependencies[0].1, ">=1.20 || <1.22");
        assert!(!parsed.dependencies[1].2);
    }

    #[test]
    fn accepts_legacy_dependency_map() {
        let metadata = r#"{
  "quilt_loader": {
    "id": "example",
    "version": "1",
    "depends": {"minecraft": "1.20"}
  }
}"#;
        let parsed = parse_quilt(metadata).unwrap();
        assert_eq!(
            parsed.dependencies,
            [("minecraft".to_string(), "1.20".to_string(), true)]
        );
    }
}
