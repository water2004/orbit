//! Launcher 版本 JSON 解析器。
//!
//! 解析 Minecraft Launcher 风格的版本清单文件
//!（位于 `versions/<id>/<id>.json`，与同名 JAR 相邻）。
//! 从中提取 libraries 列表，用于识别加载器类型和版本。

use serde::Deserialize;

use crate::error::OrbitError;

/// Maven 坐标：`groupId:artifactId:version`
#[derive(Debug, Clone)]
pub struct MavenCoord {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
}

impl MavenCoord {
    /// 从 `"net.fabricmc:fabric-loader:0.16.10"` 格式解析
    pub fn parse(raw: &str) -> Option<Self> {
        let parts: Vec<&str> = raw.split(':').collect();
        if parts.len() < 3 {
            return None;
        }
        Some(Self {
            group_id: parts[0].to_string(),
            artifact_id: parts[1].to_string(),
            version: parts[2].to_string(),
        })
    }
}

/// Launcher 版本清单 JSON 顶层结构
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionProfile {
    pub id: String,
    #[serde(default)]
    pub inherits_from: Option<String>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub libraries: Vec<LibraryEntry>,
}

/// libraries 数组中的单个条目
#[derive(Debug, Clone, Deserialize)]
pub struct LibraryEntry {
    /// Maven 坐标格式：`"net.fabricmc:fabric-loader:0.16.10"`
    pub name: String,
}

impl VersionProfile {
    /// 从文件路径解析
    pub fn from_path(path: &std::path::Path) -> Result<Self, OrbitError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot read {}: {e}", path.display()))
        })?;
        Self::from_json(&content)
    }

    /// 从 JSON 字符串解析
    pub fn from_json(content: &str) -> Result<Self, OrbitError> {
        serde_json::from_str(content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("invalid version profile JSON: {e}")))
    }

    /// 查找匹配 groupId + artifactId 的 library，返回其版本号
    pub fn find_library(&self, group_id: &str, artifact_id: &str) -> Option<String> {
        self.libraries.iter().find_map(|lib| {
            let coord = MavenCoord::parse(&lib.name)?;
            if coord.group_id == group_id && coord.artifact_id == artifact_id {
                Some(coord.version)
            } else {
                None
            }
        })
    }

    /// 检查 mainClass 是否匹配给定的模式（子串包含）
    pub fn main_class_contains(&self, needle: &str) -> bool {
        self.main_class
            .as_deref()
            .map(|mc| mc.contains(needle))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fabric_profile() {
        let json = r#"{
  "id": "fabric-loader-0.16.10-1.21.11",
  "inheritsFrom": "1.21.11",
  "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
  "libraries": [
    {
      "name": "net.fabricmc:fabric-loader:0.16.10",
      "url": "https://maven.fabricmc.net/"
    },
    {
      "name": "net.fabricmc:intermediary:1.21.11",
      "url": "https://maven.fabricmc.net/"
    }
  ]
}"#;
        let profile = VersionProfile::from_json(json).unwrap();
        assert_eq!(profile.id, "fabric-loader-0.16.10-1.21.11");
        assert_eq!(profile.inherits_from.as_deref(), Some("1.21.11"));

        let loader_ver = profile.find_library("net.fabricmc", "fabric-loader");
        assert_eq!(loader_ver.as_deref(), Some("0.16.10"));

        assert!(profile.main_class_contains("fabricmc"));
    }

    #[test]
    fn parse_maven_coord() {
        let coord = MavenCoord::parse("net.fabricmc:fabric-loader:0.16.10").unwrap();
        assert_eq!(coord.group_id, "net.fabricmc");
        assert_eq!(coord.artifact_id, "fabric-loader");
        assert_eq!(coord.version, "0.16.10");
    }

    #[test]
    fn parse_minimal_profile() {
        let json = r#"{"id": "1.21.11"}"#;
        let profile = VersionProfile::from_json(json).unwrap();
        assert_eq!(profile.id, "1.21.11");
        assert!(profile.libraries.is_empty());
        assert!(profile.main_class.is_none());
    }
}
