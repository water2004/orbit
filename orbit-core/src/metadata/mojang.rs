//! Mojang `version.json` parser.
//!
//! The embedded format is selected from the immutable Mojang world-version
//! range before `pack_version` is decoded. This deliberately does not infer a
//! schema from the JSON value's shape.

use serde::Deserialize;
use serde_json::Value;

use crate::error::OrbitError;

#[derive(Debug, Clone)]
pub struct McVersion {
    pub id: String,
    pub name: String,
    pub world_version: u32,
    pub protocol_version: u32,
    pub pack_version: PackVersion,
    pub java_version: u32,
    pub stable: bool,
}

#[derive(Debug, Clone)]
pub struct PackVersion {
    pub resource_major: u32,
    pub resource_minor: u32,
    pub data_major: u32,
    pub data_minor: u32,
}

#[derive(Debug, Deserialize)]
struct McVersionWire {
    id: String,
    name: String,
    world_version: u32,
    protocol_version: u32,
    pack_version: Value,
    java_version: Option<u32>,
    stable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackVersionSchema {
    /// `pack_version: 6`
    SharedInteger,
    /// `pack_version: { "resource": 64, "data": 81 }`
    SeparateInteger,
    /// `pack_version: { "resource_major": 69, ... }`
    MajorMinor,
}

#[derive(Debug, Clone, Copy)]
struct PackVersionSchemaRange {
    first_world_version: u32,
    last_world_version: Option<u32>,
    schema: PackVersionSchema,
}

// These ranges come from the version.json files inside Mojang's official
// server JARs. The gaps contain no published Minecraft version:
// - 18w47b (1913) through 1.16.5 (2586): shared integer
// - 20w45a (2681) through 1.21.8 (4440): resource/data integers
// - 25w31a (4534) onward; first release 1.21.9: major/minor pairs
const PACK_VERSION_SCHEMA_RANGES: &[PackVersionSchemaRange] = &[
    PackVersionSchemaRange {
        first_world_version: 1913,
        last_world_version: Some(2586),
        schema: PackVersionSchema::SharedInteger,
    },
    PackVersionSchemaRange {
        first_world_version: 2681,
        last_world_version: Some(4440),
        schema: PackVersionSchema::SeparateInteger,
    },
    PackVersionSchemaRange {
        first_world_version: 4534,
        last_world_version: None,
        schema: PackVersionSchema::MajorMinor,
    },
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeparatePackVersion {
    resource: u32,
    data: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MajorMinorPackVersion {
    resource_major: u32,
    resource_minor: u32,
    data_major: u32,
    data_minor: u32,
}

impl McVersion {
    /// Parse the `version.json` extracted by `jar.rs` or platform detection.
    pub fn from_json(content: &str) -> Result<Self, OrbitError> {
        let wire: McVersionWire = serde_json::from_str(content).map_err(invalid_json)?;
        let schema = pack_version_schema(wire.world_version).ok_or_else(|| {
            invalid_json(format!(
                "Minecraft '{}' has unregistered world version {}; no pack_version schema range contains it",
                wire.id, wire.world_version
            ))
        })?;
        let pack_version =
            parse_pack_version(&wire.id, wire.world_version, schema, wire.pack_version)?;
        let java_version = match wire.java_version {
            Some(version) => version,
            // Mojang did not add java_version until 21w19a (world version
            // 2714). Every official version.json before that range targets
            // Java 8.
            None if wire.world_version < 2714 => 8,
            None => {
                return Err(invalid_json(format!(
                    "Minecraft '{}' (world version {}) must declare java_version",
                    wire.id, wire.world_version
                )));
            }
        };

        Ok(Self {
            id: wire.id,
            name: wire.name,
            world_version: wire.world_version,
            protocol_version: wire.protocol_version,
            pack_version,
            java_version,
            stable: wire.stable,
        })
    }
}

fn pack_version_schema(world_version: u32) -> Option<PackVersionSchema> {
    PACK_VERSION_SCHEMA_RANGES
        .iter()
        .find(|range| {
            world_version >= range.first_world_version
                && range
                    .last_world_version
                    .is_none_or(|last| world_version <= last)
        })
        .map(|range| range.schema)
}

fn parse_pack_version(
    minecraft: &str,
    world_version: u32,
    schema: PackVersionSchema,
    value: Value,
) -> Result<PackVersion, OrbitError> {
    let parsed = match schema {
        PackVersionSchema::SharedInteger => {
            let shared: u32 = serde_json::from_value(value).map_err(|error| {
                pack_version_error(minecraft, world_version, "an integer", error)
            })?;
            PackVersion {
                resource_major: shared,
                resource_minor: 0,
                data_major: shared,
                data_minor: 0,
            }
        }
        PackVersionSchema::SeparateInteger => {
            let separate: SeparatePackVersion = serde_json::from_value(value).map_err(|error| {
                pack_version_error(
                    minecraft,
                    world_version,
                    "an object containing resource and data",
                    error,
                )
            })?;
            PackVersion {
                resource_major: separate.resource,
                resource_minor: 0,
                data_major: separate.data,
                data_minor: 0,
            }
        }
        PackVersionSchema::MajorMinor => {
            let split: MajorMinorPackVersion =
                serde_json::from_value(value).map_err(|error| {
                    pack_version_error(
                        minecraft,
                        world_version,
                        "an object containing resource_major, resource_minor, data_major, and data_minor",
                        error,
                    )
                })?;
            PackVersion {
                resource_major: split.resource_major,
                resource_minor: split.resource_minor,
                data_major: split.data_major,
                data_minor: split.data_minor,
            }
        }
    };
    Ok(parsed)
}

fn pack_version_error(
    minecraft: &str,
    world_version: u32,
    expected: &str,
    error: serde_json::Error,
) -> OrbitError {
    invalid_json(format!(
        "Minecraft '{minecraft}' (world version {world_version}) requires pack_version to be {expected}: {error}"
    ))
}

fn invalid_json(error: impl std::fmt::Display) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!("invalid version.json: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shared_pack_version_for_1_16_5() {
        let version = McVersion::from_json(
            r#"{
                "id":"1.16.5","name":"1.16.5","world_version":2586,
                "protocol_version":754,"pack_version":6,"stable":true
            }"#,
        )
        .unwrap();

        assert_eq!(version.pack_version.resource_major, 6);
        assert_eq!(version.pack_version.data_major, 6);
        assert_eq!(version.java_version, 8);
    }

    #[test]
    fn parses_separate_pack_version_for_1_21_1() {
        let version = McVersion::from_json(
            r#"{
                "id":"1.21.1","name":"1.21.1","world_version":3955,
                "protocol_version":767,"pack_version":{"resource":34,"data":48},
                "java_version":21,"stable":true
            }"#,
        )
        .unwrap();

        assert_eq!(version.pack_version.resource_major, 34);
        assert_eq!(version.pack_version.resource_minor, 0);
        assert_eq!(version.pack_version.data_major, 48);
        assert_eq!(version.pack_version.data_minor, 0);
    }

    #[test]
    fn parses_major_minor_pack_version_for_1_21_11() {
        let version = McVersion::from_json(
            r#"{
                "id":"1.21.11","name":"1.21.11","world_version":4671,
                "protocol_version":774,
                "pack_version":{"resource_major":75,"resource_minor":0,"data_major":94,"data_minor":1},
                "java_version":21,"stable":true
            }"#,
        )
        .unwrap();

        assert_eq!(version.pack_version.resource_major, 75);
        assert_eq!(version.pack_version.data_major, 94);
        assert_eq!(version.pack_version.data_minor, 1);
    }

    #[test]
    fn rejects_shape_from_another_version_range() {
        let error = McVersion::from_json(
            r#"{
                "id":"1.21.1","name":"1.21.1","world_version":3955,
                "protocol_version":767,
                "pack_version":{"resource_major":34,"resource_minor":0,"data_major":48,"data_minor":0},
                "java_version":21,"stable":true
            }"#,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("requires pack_version to be an object containing resource and data")
        );
    }

    #[test]
    fn rejects_world_versions_outside_published_ranges() {
        let error = McVersion::from_json(
            r#"{
                "id":"unknown","name":"unknown","world_version":2600,
                "protocol_version":0,"pack_version":6,"stable":false
            }"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("no pack_version schema range"));
    }
}
