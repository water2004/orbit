//! Mojang version.json 解析器。
//!
//! 游戏 JAR 内的 `version.json` 由 `jar.rs` 提取字符串 → 由此模块解析为 `McVersion`。
//! 纯函数，无文件 I/O。

use serde::Deserialize;

use crate::error::OrbitError;

#[derive(Debug, Clone, Deserialize)]
pub struct McVersion {
    pub id: String,
    pub name: String,
    pub world_version: u32,
    pub protocol_version: u32,
    pub pack_version: PackVersion,
    pub java_version: u32,
    pub stable: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackVersion {
    pub resource_major: u32,
    pub resource_minor: u32,
    pub data_major: u32,
    pub data_minor: u32,
}

impl McVersion {
    /// 从 version.json 字符串内容解析。
    /// 调用方 (`detection/` 或 `jar.rs`) 负责从 JAR 中提取此字符串。
    pub fn from_json(content: &str) -> Result<Self, OrbitError> {
        serde_json::from_str(content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("invalid version.json: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_1_21_11() {
        let json = r#"{
            "id": "1.21.11",
            "name": "1.21.11",
            "world_version": 4671,
            "series_id": "main",
            "protocol_version": 774,
            "pack_version": {
                "resource_major": 75,
                "resource_minor": 0,
                "data_major": 94,
                "data_minor": 1
            },
            "build_time": "2025-12-09T12:20:42+00:00",
            "java_component": "java-runtime-delta",
            "java_version": 21,
            "stable": true,
            "use_editor": false
        }"#;

        let version = McVersion::from_json(json).unwrap();
        assert_eq!(version.id, "1.21.11");
        assert_eq!(version.name, "1.21.11");
        assert_eq!(version.world_version, 4671);
        assert_eq!(version.protocol_version, 774);
        assert_eq!(version.pack_version.resource_major, 75);
        assert_eq!(version.pack_version.data_major, 94);
        assert_eq!(version.java_version, 21);
        assert!(version.stable);
    }
}
