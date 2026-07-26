//! Fabric `fabric.mod.json` adapter.

use serde_json::Value;

use super::{
    DependencyExpression, DependencyKind, DependencyOrdering, Environment, MetadataParser,
    ModDependency, ModFileMetadata, ModLoadCondition, ModLoader, ModMetadata, ProvidedMod,
};
use crate::error::OrbitError;

pub struct FabricParser;

impl MetadataParser for FabricParser {
    fn target_file(&self) -> &str {
        "fabric.mod.json"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Fabric
    }

    fn parse(&self, content: &str) -> Result<ModFileMetadata, OrbitError> {
        let value: Value = orbit_loader_json::from_str(content).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!("invalid fabric.mod.json: {error}"))
        })?;
        let object = value.as_object().ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "fabric.mod.json must contain a JSON object"
            ))
        })?;
        let id = required_string(object, "id", "fabric.mod.json")?;
        validate_mod_id(&id)?;
        let version = required_string(object, "version", "fabric.mod.json")?;

        let mut dependencies = Vec::new();
        append_dependencies(
            &mut dependencies,
            object.get("depends"),
            DependencyKind::Required,
        )?;
        append_dependencies(
            &mut dependencies,
            object.get("recommends"),
            DependencyKind::Recommended,
        )?;
        append_dependencies(
            &mut dependencies,
            object.get("suggests"),
            DependencyKind::Suggested,
        )?;
        append_dependencies(
            &mut dependencies,
            object.get("conflicts"),
            DependencyKind::Discouraged,
        )?;
        append_dependencies(
            &mut dependencies,
            object.get("breaks"),
            DependencyKind::Incompatible,
        )?;
        let provides = parse_provides(object.get("provides"))?;
        let embedded_jars = parse_jars(object.get("jars"))?;

        Ok(ModFileMetadata {
            loader: ModLoader::Fabric,
            license: object.get("license").and_then(first_string),
            language_loader: None,
            mods: vec![ModMetadata {
                id: id.clone(),
                name: optional_string(object, "name").unwrap_or(id),
                version,
                authors: object.get("authors").map(parse_people).unwrap_or_default(),
                description: optional_string(object, "description").unwrap_or_default(),
                environment: parse_environment(object.get("environment"))?,
                dependencies,
                provides,
                // Fabric Loader treats nested candidates as greedy optional mods.
                load_condition: ModLoadCondition::IfPossible,
            }],
            embedded_jars,
            substitution_properties: Default::default(),
        })
    }
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
    source: &str,
) -> Result<String, OrbitError> {
    optional_string(object, key).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "{source} requires a non-empty string field '{key}'"
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

fn parse_people(value: &Value) -> Vec<String> {
    match value {
        Value::String(name) => vec![name.clone()],
        Value::Array(people) => people.iter().filter_map(first_string).collect(),
        _ => Vec::new(),
    }
}

fn first_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Array(values) => values.first().and_then(first_string),
        Value::Object(value) => value.get("name").and_then(first_string),
        _ => None,
    }
}

fn append_dependencies(
    output: &mut Vec<DependencyExpression>,
    value: Option<&Value>,
    kind: DependencyKind,
) -> Result<(), OrbitError> {
    let Some(value) = value else {
        return Ok(());
    };
    let dependencies = value.as_object().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "fabric dependency collection must be an object"
        ))
    })?;
    for (id, requirement) in dependencies {
        validate_mod_id(id)?;
        output.push(DependencyExpression::Only(ModDependency {
            id: id.clone(),
            requirement: version_requirement(requirement)?,
            kind,
            environment: Environment::Both,
            ordering: DependencyOrdering::None,
            reason: None,
            unless: None,
        }));
    }
    Ok(())
}

fn version_requirement(value: &Value) -> Result<String, OrbitError> {
    match value {
        Value::String(requirement) if !requirement.is_empty() => Ok(requirement.clone()),
        Value::Array(requirements) => {
            let requirements: Vec<_> =
                requirements
                    .iter()
                    .map(|requirement| {
                        requirement.as_str().filter(|value| !value.is_empty()).ok_or_else(|| {
                        OrbitError::Other(anyhow::anyhow!(
                            "fabric dependency version arrays must contain non-empty strings"
                        ))
                    })
                    })
                    .collect::<Result<_, _>>()?;
            if requirements.is_empty() {
                Err(OrbitError::Other(anyhow::anyhow!(
                    "fabric dependency version arrays cannot be empty"
                )))
            } else {
                Ok(requirements.join(" || "))
            }
        }
        _ => Err(OrbitError::Other(anyhow::anyhow!(
            "fabric dependency versions must be a string or array of strings"
        ))),
    }
}

fn parse_environment(value: Option<&Value>) -> Result<Environment, OrbitError> {
    let values: Vec<&str> = match value {
        None => vec!["*"],
        Some(Value::String(value)) => vec![value],
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    OrbitError::Other(anyhow::anyhow!(
                        "fabric environment arrays must contain strings"
                    ))
                })
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "fabric environment must be a string or array of strings"
            )));
        }
    };
    if values.is_empty()
        || values
            .iter()
            .any(|value| !matches!(*value, "*" | "client" | "server"))
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "fabric environment must contain client, server, or *"
        )));
    }
    if values.contains(&"*") || values.len() > 1 {
        Ok(Environment::Both)
    } else {
        Ok(match values[0] {
            "client" => Environment::Client,
            "server" => Environment::Server,
            _ => Environment::Both,
        })
    }
}

fn parse_provides(value: Option<&Value>) -> Result<Vec<ProvidedMod>, OrbitError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("fabric provides must be an array")))?;
    values
        .iter()
        .map(|value| {
            let id = value.as_str().ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!("fabric provides entries must be strings"))
            })?;
            validate_mod_id(id)?;
            Ok(ProvidedMod {
                id: id.to_string(),
                version: None,
            })
        })
        .collect()
}

fn parse_jars(value: Option<&Value>) -> Result<Vec<String>, OrbitError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("fabric jars must be an array")))?;
    values
        .iter()
        .map(|entry| {
            entry
                .as_object()
                .and_then(|entry| entry.get("file"))
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    OrbitError::Other(anyhow::anyhow!(
                        "fabric jar entries require a non-empty file"
                    ))
                })
        })
        .collect()
}

fn validate_mod_id(id: &str) -> Result<(), OrbitError> {
    let valid = (2..=64).contains(&id.len())
        && id.as_bytes()[0].is_ascii_lowercase()
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(OrbitError::Other(anyhow::anyhow!(
            "fabric.mod.json contains invalid mod id '{id}'"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_every_dependency_kind_and_provides() {
        let parsed = FabricParser
            .parse(
                r#"{
  "schemaVersion": 1,
  "id": "example",
  "version": "1.0",
  "environment": "client",
  "depends": {"fabricloader": ">=0.16"},
  "recommends": {"recommended": "*"},
  "suggests": {"suggested": "*"},
  "conflicts": {"risky": "<2"},
  "breaks": {"broken": "[1,2)"},
  "provides": ["example-api"],
  "jars": [{"file": "META-INF/jars/inner.jar"}]
}"#,
            )
            .unwrap();

        let metadata = &parsed.mods[0];
        assert_eq!(metadata.environment, Environment::Client);
        assert_eq!(
            metadata.provides,
            [ProvidedMod {
                id: "example-api".to_string(),
                version: None,
            }]
        );
        let kinds: Vec<_> = metadata
            .dependencies
            .iter()
            .map(|dependency| match dependency {
                DependencyExpression::Only(dependency) => dependency.kind,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                DependencyKind::Required,
                DependencyKind::Recommended,
                DependencyKind::Suggested,
                DependencyKind::Discouraged,
                DependencyKind::Incompatible,
            ]
        );
        assert_eq!(parsed.embedded_jars, ["META-INF/jars/inner.jar"]);
    }

    #[test]
    fn requires_identity_fields() {
        let error = FabricParser.parse(r#"{"id":"example"}"#).unwrap_err();
        assert!(error.to_string().contains("version"));
    }

    #[test]
    fn accepts_loader_compatible_unescaped_controls_in_strings() {
        let parsed = FabricParser
            .parse(
                "{
  \"schemaVersion\": 1,
  \"id\": \"example\",
  \"version\": \"1.0\",
  \"description\": \"first line
second line\"
}",
            )
            .unwrap();

        assert_eq!(parsed.mods[0].description, "first line\nsecond line");
    }

    #[test]
    fn does_not_accept_unrelated_lenient_json_extensions() {
        let error = FabricParser
            .parse(
                r#"{
  "schemaVersion": 1,
  "id": "example",
  "version": "1.0",
}"#,
            )
            .unwrap_err();

        assert!(error.to_string().contains("invalid fabric.mod.json"));
    }
}
