//! FabricParser — 解析 fabric.mod.json
//!
//! 采用 per-field fallback 策略：先解析为 `serde_json::Value`，
//! 然后逐个字段提取。任一字段格式异常仅影响该字段，不导致整体失败。

use indexmap::IndexMap;

use super::{ModLoader, ModMetadata, MetadataParser};
use crate::error::OrbitError;

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
        let v: serde_json::Value = serde_json::from_str(content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("invalid JSON in fabric.mod.json: {e}")))?;

        Ok(ModMetadata {
            id: get_str(&v, "id"),
            version: get_str(&v, "version"),
            name: get_str(&v, "name"),
            description: get_str(&v, "description"),
            authors: get_authors(&v),
            license: v.get("license").and_then(|l| l.as_str()).map(String::from),
            environment: map_environment(v.get("environment").and_then(|e| e.as_str()).unwrap_or("*")),
            dependencies: get_depends(&v),
            embedded_jars: get_jars(&v),
            loader: ModLoader::Fabric,
            sha256: String::new(),
        })
    }
}

// ── 逐字段提取（各自失败互不影响） ────────────

/// 提取字符串字段，失败时返回空字符串
fn get_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_default()
}

/// 提取 authors，接受任何 JSON 形式：
/// null / "str" / ["a","b"] / [{"name":"a"},...] / 混合
fn get_authors(v: &serde_json::Value) -> Vec<String> {
    match v.get("authors") {
        None | Some(serde_json::Value::Null) => vec![],
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(arr)) => arr.iter().map(|elem| match elem {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(obj) => {
                obj.get("name")
                    .and_then(|n| n.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| format!("{obj:?}"))
            }
            other => other.to_string(),
        }).collect(),
        _ => vec![],
    }
}

/// 提取 jars 字段 → 内嵌 JAR 的相对路径列表
fn get_jars(v: &serde_json::Value) -> Vec<String> {
    v.get("jars")
        .and_then(|j| j.as_array())
        .map(|arr| arr.iter()
            .filter_map(|entry| entry.get("file").and_then(|f| f.as_str()))
            .map(String::from)
            .collect())
        .unwrap_or_default()
}

/// 提取 depends → IndexMap<String, String>
/// 值可以是 `"str"` 或 `["str", ...]`
fn get_depends(v: &serde_json::Value) -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    let Some(deps) = v.get("depends").and_then(|d| d.as_object()) else {
        return map;
    };
    for (key, val) in deps {
        let constraint = match val {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(arr) => {
                let parts: Vec<&str> = arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect();
                if parts.len() == 1 {
                    parts[0].to_string()
                } else {
                    parts.join(" || ")
                }
            }
            _ => val.to_string(),
        };
        map.insert(key.clone(), constraint);
    }
    map
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
    fn parse_voxy_full() {
        // 完整 Voxy fabric.mod.json —— 含 mixins、jars 等所有字段
        let json = r#"{
  "schemaVersion": 1,
  "id": "voxy",
  "version": "0.2.14-alpha",
  "custom": {"commit": "41dd201d", "buildtime": "1775659995"},
  "name": "Voxy",
  "description": "Far distance rendering mod utilising LoDs",
  "authors": ["Cortex"],
  "contact": {},
  "license": "All-Rights-Reserved",
  "icon": "assets/voxy/icon.png",
  "environment": "*",
  "entrypoints": {
    "client": ["me.cortex.voxy.client.VoxyClient"],
    "main": ["me.cortex.voxy.commonImpl.VoxyCommon"]
  },
  "mixins": [
    {"config": "client.voxy.mixins.json", "environment": "client"},
    "common.voxy.mixins.json"
  ],
  "depends": {
    "minecraft": ["~26.1"],
    "fabricloader": ">=0.14.22",
    "fabric-api": ">=0.91.1",
    "sodium": [">=0.8.9"]
  },
  "breaks": {"voxyworldgenv2": "=2.2.2"},
  "accessWidener": "voxy.accesswidener",
  "jars": [
    {"file": "META-INF/jars/commons-pool2-2.12.0.jar"},
    {"file": "META-INF/jars/jedis-5.1.0.jar"}
  ]
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
        assert_eq!(meta.dependencies.get("minecraft").unwrap(), "~26.1");
        assert_eq!(meta.dependencies.get("fabricloader").unwrap(), ">=0.14.22");
        assert_eq!(meta.dependencies.get("sodium").unwrap(), ">=0.8.9");
        // breaks 不在提取范围内
        assert!(!meta.dependencies.contains_key("voxyworldgenv2"));
    }

    #[test]
    fn parse_sodium_full() {
        let json = r#"{
  "schemaVersion": 1,
  "id": "sodium",
  "version": "0.8.7+mc1.21.11",
  "name": "Sodium",
  "description": "Sodium is a powerful rendering engine...",
  "authors": [
    {"name": "JellySquid (jellysquid3)", "contact": {"email": "jellysquid@pm.me"}}
  ],
  "contributors": ["IMS212", "bytzo"],
  "contact": {"homepage": "https://github.com/CaffeineMC/sodium"},
  "license": "Polyform-Shield-1.0.0",
  "icon": "sodium-icon.png",
  "environment": "client",
  "entrypoints": {
    "client": ["net.caffeinemc.mods.sodium.fabric.SodiumFabricMod"]
  },
  "custom": {"modmenu": {"links": {"modmenu.discord": "https://..."}}},
  "accessWidener": "sodium-fabric.accesswidener",
  "mixins": [
    "sodium-common.mixins.json",
    "sodium-fabric.mixins.json"
  ],
  "depends": {
    "fabricloader": ">=0.16.0",
    "fabric-block-view-api-v2": "*",
    "fabric-rendering-fluids-v1": ">=2.0.0",
    "fabric-resource-loader-v0": "*"
  },
  "breaks": {
    "embeddium": "*",
    "optifabric": "*"
  },
  "provides": ["indium"],
  "jars": [
    {"file": "META-INF/jars/fabric-api-base-1.0.5.jar"},
    {"file": "META-INF/jars/fabric-block-view-api-v2-1.0.39.jar"}
  ]
}"#;
        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();

        assert_eq!(meta.id, "sodium");
        assert_eq!(meta.version, "0.8.7+mc1.21.11");
        assert_eq!(meta.name, "Sodium");
        assert_eq!(meta.authors, vec!["JellySquid (jellysquid3)"]);
        assert_eq!(meta.environment, "client");
        assert_eq!(meta.dependencies.get("fabricloader").unwrap(), ">=0.16.0");
        assert_eq!(meta.dependencies.get("fabric-rendering-fluids-v1").unwrap(), ">=2.0.0");
    }

    #[test]
    fn parse_minimal() {
        let json = r#"{"schemaVersion": 1, "id": "minimal-mod", "version": "1.0.0"}"#;
        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();
        assert_eq!(meta.id, "minimal-mod");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.name, "");
        assert!(meta.authors.is_empty());
        assert_eq!(meta.environment, "both");
    }

    #[test]
    fn parse_mixed_authors() {
        let json = r#"{
  "id": "mymod", "version": "1.0",
  "authors": [{"name": "DevOne"}, "DevTwo", {"name": "DevThree"}],
  "depends": {}
}"#;
        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();
        assert_eq!(meta.authors, vec!["DevOne", "DevTwo", "DevThree"]);
    }

    #[test]
    fn parse_single_string_author() {
        let json = r#"{"id": "m", "version": "1", "authors": "SoloDev", "depends": {}}"#;
        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();
        assert_eq!(meta.authors, vec!["SoloDev"]);
    }

    #[test]
    fn parse_malformed_authors_does_not_break_other_fields() {
        // authors 是畸形数字，不影响 id/version 的提取
        let json = r#"{
  "id": "survivor",
  "version": "2.0",
  "authors": 42,
  "depends": {"minecraft": "1.20"}
}"#;
        let parser = FabricParser;
        let meta = parser.parse(json).unwrap();
        assert_eq!(meta.id, "survivor");
        assert_eq!(meta.version, "2.0");
        assert_eq!(meta.dependencies.get("minecraft").unwrap(), "1.20");
        assert!(meta.authors.is_empty()); // 畸形数据 → 空数组
    }

    #[test]
    fn parse_completely_broken_json_fails() {
        let json = "not json at all{{{";
        let parser = FabricParser;
        assert!(parser.parse(json).is_err());
    }
}
