//! orbit.lock 的 serde 结构体与读写。
//!
//! 格式规格参见 docs/orbit-toml-spec.md §4

use serde::{Deserialize, Serialize};

use crate::error::OrbitError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitLockfile {
    pub meta: LockMeta,
    #[serde(rename = "lock")]
    pub entries: Vec<LockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMeta {
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub name: String,
    pub version: String,
    pub filename: String,
    pub sha256: String,
    /// SHA-512 校验值（Modrinth 原生，用于哈希反查）
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sha512: String,
    pub dependencies: Vec<LockDependency>,

    /// 内嵌子模组（从父 JAR 的 META-INF/jars/ 解出）
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub implanted: Vec<ImplantedMod>,

    // 平台在线依赖
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mod_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    // 本地/直链依赖
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplantedMod {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockDependency {
    pub name: String,
    pub version: String,
}

impl OrbitLockfile {
    /// 从文件路径解析 orbit.lock
    pub fn from_path(path: &std::path::Path) -> Result<Self, OrbitError> {
        let content =
            std::fs::read_to_string(path).map_err(|_| OrbitError::LockfileNotFound)?;
        let lockfile: Self = toml::from_str(&content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to parse orbit.lock: {e}")))?;
        Ok(lockfile)
    }

    /// 序列化为 TOML 字符串
    pub fn to_toml_string(&self) -> Result<String, OrbitError> {
        toml::to_string_pretty(self)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to serialize orbit.lock: {e}")))
    }

    /// 从项目目录加载 orbit.lock
    pub fn from_dir(dir: &std::path::Path) -> Result<Self, OrbitError> {
        let path = dir.join("orbit.lock");
        Self::from_path(&path)
    }

    /// 按名称查找条目
    pub fn find(&self, name: &str) -> Option<&LockEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lockfile() {
        let toml_str = r#"
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[[lock]]
name = "sodium"
platform = "modrinth"
mod_id = "AANobbMI"
version = "0.5.8"
filename = "sodium-fabric-mc1.20.1-0.5.8.jar"
url = "https://cdn.modrinth.com/data/AANobbMI/versions/abc123/sodium.jar"
sha256 = "abc123def456"
dependencies = []

[[lock]]
name = "my-custom-mod"
type = "file"
version = "1.0"
filename = "mymod.jar"
path = "mods/custom/mymod.jar"
sha256 = "def456abc123"
dependencies = []
"#;
        let lockfile: OrbitLockfile = toml::from_str(toml_str).unwrap();
        assert_eq!(lockfile.meta.mc_version, "1.20.1");
        assert_eq!(lockfile.entries.len(), 2);
        assert_eq!(lockfile.find("sodium").unwrap().platform.as_deref(), Some("modrinth"));
        assert_eq!(lockfile.find("my-custom-mod").unwrap().source_type.as_deref(), Some("file"));
    }

    #[test]
    fn lockfile_roundtrip() {
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".into(),
                modloader: "fabric".into(),
                modloader_version: "0.15.7".into(),
            },
            entries: vec![LockEntry {
                name: "sodium".into(),
                platform: Some("modrinth".into()),
                mod_id: Some("AANobbMI".into()),
                version: "0.5.8".into(),
                filename: "sodium.jar".into(),
                url: Some("https://example.com/sodium.jar".into()),
                sha256: "abc123".into(),
                sha512: String::new(),
                source_type: None,
                path: None,
                dependencies: vec![],
                implanted: vec![],
            }],
        };

        let serialized = lockfile.to_toml_string().unwrap();
        let deserialized: OrbitLockfile = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.entries.len(), 1);
        assert_eq!(deserialized.entries[0].name, "sodium");
    }
}
