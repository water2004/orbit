//! FabricParser — 解析 fabric.mod.json

use indexmap::IndexMap;
use serde::Deserialize;

use super::{ModLoader, ModMetadata, MetadataParser};
use crate::error::OrbitError;

// ── JSON 结构体（只提取关心的字段） ──────────────────

#[derive(Deserialize)]
struct FabricModJson {
    id: Option<String>,
    version: Option<String>,
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    license: Option<String>,
    #[serde(default = "default_environment")]
    environment: String,
    #[serde(default)]
    depends: IndexMap<String, DependencyValue>,
}

fn default_environment() -> String {
    "*".into()
}

/// `depends` 的值可以是字符串或字符串数组
#[derive(Deserialize)]
#[serde(untagged)]
enum DependencyValue {
    Single(String),
    List(Vec<String>),
}

impl DependencyValue {
    /// 合并为单一版本约束字符串。
    /// 数组元素用 " || " 连接（表示任一满足即可）。
    fn join(&self) -> String {
        match self {
            DependencyValue::Single(s) => s.clone(),
            DependencyValue::List(v) => {
                if v.len() == 1 {
                    v[0].clone()
                } else {
                    v.join(" || ")
                }
            }
        }
    }
}

// ── Parser ──────────────────────────────────────

pub struct FabricParser;

impl MetadataParser for FabricParser {
    fn target_file(&self) -> &str {
        "fabric.mod.json"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Fabric
    }

    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError> {
        let raw: FabricModJson = serde_json::from_str(content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("invalid fabric.mod.json: {e}")))?;

        // 依赖转换为统一格式
        let deps: IndexMap<String, String> = raw.depends
            .into_iter()
            .map(|(k, v)| (k, v.join()))
            .collect();

        Ok(ModMetadata {
            id: raw.id.unwrap_or_default(),
            version: raw.version.unwrap_or_default(),
            name: raw.name.unwrap_or_default(),
            description: raw.description.unwrap_or_default(),
            authors: raw.authors,
            license: raw.license,
            environment: map_environment(&raw.environment),
            dependencies: deps,
            loader: ModLoader::Fabric,
            sha256: String::new(),
        })
    }
}

fn map_environment(raw: &str) -> String {
    match raw {
        "*" => "both",
        "client" => "client",
        "server" => "server",
        other => other,
    }.into()
}

// ── 测试 ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_voxy() {
        let json = r#"{
  "schemaVersion": 1,
  "id": "voxy",
  "version": "0.2.14-alpha",
  "custom": {
    "commit": "41dd201d3d676ce697ada40f5a3c13b25845d32b",
    "buildtime": "1775659995"
  },
  "name": "Voxy",
  "description": "Far distance rendering mod utilising LoDs",
  "authors": [
    "Cortex"
  ],
  "contact": {},
  "license": "All-Rights-Reserved",
  "icon": "assets/voxy/icon.png",
  "environment": "*",
  "entrypoints": {
    "client": [
      "me.cortex.voxy.client.VoxyClient"
    ]
  },
  "depends": {
    "minecraft": ["~26.1"],
    "fabricloader": ">=0.14.22",
    "fabric-api": ">=0.91.1",
    "sodium": [">=0.8.9"]
  },
  "breaks": {
    "voxyworldgenv2": "=2.2.2"
  }
}"#;

        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();

        assert_eq!(meta.id, "voxy");
        assert_eq!(meta.version, "0.2.14-alpha");
        assert_eq!(meta.name, "Voxy");
        assert_eq!(meta.description, "Far distance rendering mod utilising LoDs");
        assert_eq!(meta.authors, vec!["Cortex"]);
        assert_eq!(meta.license.as_deref(), Some("All-Rights-Reserved"));
        assert_eq!(meta.environment, "both");
        assert_eq!(meta.loader, ModLoader::Fabric);

        // depends: > → >, = → = 由 serde_json 自动解码
        assert_eq!(meta.dependencies.get("minecraft").unwrap(), "~26.1");
        assert_eq!(meta.dependencies.get("fabricloader").unwrap(), ">=0.14.22");
        assert_eq!(meta.dependencies.get("fabric-api").unwrap(), ">=0.91.1");
        assert_eq!(meta.dependencies.get("sodium").unwrap(), ">=0.8.9");
        // "breaks" 不在提取范围内
        assert!(!meta.dependencies.contains_key("voxyworldgenv2"));
    }

    #[test]
    fn parse_client_only_mod() {
        let json = r#"{
  "schemaVersion": 1,
  "id": "zoomify",
  "version": "2.11.1",
  "name": "Zoomify",
  "environment": "client",
  "depends": {
    "minecraft": ">=1.20"
  }
}"#;

        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();
        assert_eq!(meta.id, "zoomify");
        assert_eq!(meta.environment, "client");
        assert_eq!(meta.dependencies.get("minecraft").unwrap(), ">=1.20");
    }

    #[test]
    fn parse_missing_optional_fields() {
        let json = r#"{
  "schemaVersion": 1,
  "id": "minimal-mod",
  "version": "1.0.0"
}"#;

        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();
        assert_eq!(meta.id, "minimal-mod");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.name, "");          // 未填 → 空字符串
        assert!(meta.authors.is_empty());
        assert_eq!(meta.environment, "both"); // 默认 "*" → both
    }
}
