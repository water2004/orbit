//! Quilt `quilt.mod.json` adapter.

use serde_json::Value;

use super::{
    DependencyExpression, DependencyKind, DependencyOrdering, Environment, MetadataParser,
    ModDependency, ModFileMetadata, ModLoader, ModMetadata, ProvidedMod,
};
use crate::error::OrbitError;

pub(crate) fn parse_quilt(content: &str) -> Result<ModFileMetadata, OrbitError> {
    let value: Value = serde_json::from_str(content)
        .map_err(|error| OrbitError::Other(anyhow::anyhow!("invalid quilt.mod.json: {error}")))?;
    let root = value.as_object().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!("quilt.mod.json must contain a JSON object"))
    })?;
    let loader = root
        .get("quilt_loader")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!("quilt.mod.json has no quilt_loader object"))
        })?;
    let id = required_string(loader, "id")?;
    let version = required_string(loader, "version")?;
    let metadata = loader.get("metadata").and_then(Value::as_object);

    let mut dependencies = Vec::new();
    if let Some(depends) = loader.get("depends") {
        dependencies.extend(parse_top_level_dependencies(
            depends,
            DependencyKind::Required,
            GroupMode::Any,
        )?);
    }
    if let Some(breaks) = loader.get("breaks") {
        dependencies.extend(parse_top_level_dependencies(
            breaks,
            DependencyKind::Incompatible,
            GroupMode::All,
        )?);
    }

    Ok(ModFileMetadata {
        loader: ModLoader::Quilt,
        license: metadata
            .and_then(|metadata| metadata.get("license"))
            .and_then(first_string),
        language_loader: None,
        mods: vec![ModMetadata {
            id: id.clone(),
            name: metadata
                .and_then(|metadata| optional_string(metadata, "name"))
                .unwrap_or(id),
            version: version.clone(),
            authors: metadata
                .and_then(|metadata| metadata.get("contributors"))
                .map(parse_contributors)
                .unwrap_or_default(),
            description: metadata
                .and_then(|metadata| optional_string(metadata, "description"))
                .unwrap_or_default(),
            environment: root
                .get("minecraft")
                .and_then(|minecraft| minecraft.get("environment"))
                .and_then(Value::as_str)
                .map(parse_environment)
                .unwrap_or(Environment::Both),
            dependencies,
            provides: parse_provides(loader.get("provides"), &version)?,
        }],
        embedded_jars: parse_jars(loader.get("jars").or_else(|| root.get("jars"))),
        substitution_properties: Default::default(),
    })
}

#[derive(Clone, Copy)]
enum GroupMode {
    Any,
    All,
}

fn parse_top_level_dependencies(
    value: &Value,
    kind: DependencyKind,
    nested_group: GroupMode,
) -> Result<Vec<DependencyExpression>, OrbitError> {
    let dependencies = value.as_array().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "quilt dependency collection must be an array"
        ))
    })?;
    dependencies
        .iter()
        .map(|dependency| parse_dependency(dependency, kind, nested_group))
        .collect()
}

fn parse_dependency(
    value: &Value,
    default_kind: DependencyKind,
    nested_group: GroupMode,
) -> Result<DependencyExpression, OrbitError> {
    match value {
        Value::String(id) => Ok(ModDependency {
            id: strip_group(id).to_string(),
            requirement: "*".to_string(),
            kind: default_kind,
            environment: Environment::Both,
            ordering: DependencyOrdering::None,
            reason: None,
            unless: None,
        }
        .into()),
        Value::Object(dependency) => {
            let id = strip_group(&required_string(dependency, "id")?).to_string();
            let kind = if dependency
                .get("optional")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && default_kind == DependencyKind::Required
            {
                DependencyKind::Optional
            } else {
                default_kind
            };
            let unless = dependency
                .get("unless")
                .map(|unless| parse_dependency(unless, DependencyKind::Required, GroupMode::Any))
                .transpose()?
                .map(Box::new);
            Ok(ModDependency {
                id,
                requirement: version_requirement(
                    dependency
                        .get("versions")
                        .or_else(|| dependency.get("version")),
                ),
                kind,
                environment: Environment::Both,
                ordering: DependencyOrdering::None,
                reason: dependency
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                unless,
            }
            .into())
        }
        Value::Array(group) => {
            let expressions = group
                .iter()
                .map(|dependency| parse_dependency(dependency, default_kind, nested_group))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(match nested_group {
                GroupMode::Any => DependencyExpression::Any(expressions),
                GroupMode::All => DependencyExpression::All(expressions),
            })
        }
        _ => Err(OrbitError::Other(anyhow::anyhow!(
            "quilt dependency must be a string, object, or array"
        ))),
    }
}

fn parse_provides(
    value: Option<&Value>,
    own_version: &str,
) -> Result<Vec<ProvidedMod>, OrbitError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("quilt provides must be an array")))?;
    values
        .iter()
        .map(|provided| match provided {
            Value::String(id) => Ok(ProvidedMod {
                id: strip_group(id).to_string(),
                version: None,
            }),
            Value::Object(provided) => {
                let id = required_string(provided, "id")?;
                let explicit_version = provided
                    .get("version")
                    .and_then(Value::as_str)
                    .filter(|version| *version != own_version)
                    .map(str::to_string);
                Ok(ProvidedMod {
                    id: strip_group(&id).to_string(),
                    version: explicit_version,
                })
            }
            _ => Err(OrbitError::Other(anyhow::anyhow!(
                "quilt provides entries must be strings or objects"
            ))),
        })
        .collect()
}

fn strip_group(id: &str) -> &str {
    id.split_once(':').map(|(_, id)| id).unwrap_or(id)
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, OrbitError> {
    optional_string(object, key).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "quilt.mod.json requires a non-empty string field '{key}'"
        ))
    })
}

fn optional_string(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_contributors(value: &Value) -> Vec<String> {
    match value {
        Value::Object(contributors) => contributors.keys().cloned().collect(),
        Value::Array(contributors) => contributors.iter().filter_map(first_string).collect(),
        Value::String(contributor) => vec![contributor.clone()],
        _ => Vec::new(),
    }
}

fn first_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Array(values) => values.first().and_then(first_string),
        Value::Object(value) => value
            .get("name")
            .or_else(|| value.get("id"))
            .and_then(first_string),
        _ => None,
    }
}

fn version_requirement(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "*".to_string();
    };
    match value {
        Value::String(version) => version.clone(),
        Value::Array(versions) => {
            let versions: Vec<_> = versions.iter().filter_map(Value::as_str).collect();
            if versions.is_empty() {
                "*".to_string()
            } else {
                versions.join(" || ")
            }
        }
        _ => "*".to_string(),
    }
}

fn parse_jars(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|jar| match jar {
            Value::String(path) => Some(path.clone()),
            Value::Object(jar) => jar.get("file").and_then(first_string),
            _ => None,
        })
        .collect()
}

fn parse_environment(environment: &str) -> Environment {
    match environment {
        "client" => Environment::Client,
        "dedicated_server" | "server" => Environment::Server,
        _ => Environment::Both,
    }
}

pub struct QuiltParser;

impl MetadataParser for QuiltParser {
    fn target_file(&self) -> &str {
        "quilt.mod.json"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Quilt
    }

    fn parse(&self, content: &str) -> Result<ModFileMetadata, OrbitError> {
        parse_quilt(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_groups_unless_breaks_and_provides() {
        let parsed = parse_quilt(
            r#"{
  "schema_version": 1,
  "quilt_loader": {
    "id": "example",
    "version": "1.0",
    "depends": [
      [{"id": "one", "versions": ">=1"}, {"id": "two", "versions": "*"}],
      {"id": "optional", "optional": true, "unless": "replacement"}
    ],
    "breaks": [
      [{"id": "bad_a"}, {"id": "bad_b"}]
    ],
    "provides": [
      "group:alias",
      {"id": "group:versioned", "version": "2"}
    ]
  }
}"#,
        )
        .unwrap();

        let metadata = &parsed.mods[0];
        assert!(matches!(
            metadata.dependencies[0],
            DependencyExpression::Any(_)
        ));
        assert!(matches!(
            metadata.dependencies[2],
            DependencyExpression::All(_)
        ));
        assert_eq!(metadata.provides[0].id, "alias");
        assert_eq!(metadata.provides[1].version.as_deref(), Some("2"));
    }
}
