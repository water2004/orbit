//! orbit.lock 的 serde 结构体与读写。
//!
//! 格式规格参见 docs/orbit-toml-spec.md §4

use serde::{Deserialize, Serialize};

use crate::error::OrbitError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitLockfile {
    pub meta: LockMeta,
    #[serde(rename = "package")]
    pub packages: Vec<PackageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMeta {
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
}

/// [[package]] 条目。`mod_id` 为 fabric.mod.json 的 `id` 字段，是 lockfile 的键。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    /// fabric.mod.json 的 `id` 字段
    pub mod_id: String,
    /// fabric.mod.json 的 `version` 字段
    pub version: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sha1: String,
    pub sha256: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sha512: String,
    /// "modrinth" | "file"
    pub provider: String,
    /// Modrinth provider 专属字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modrinth: Option<ModrinthInfo>,
    /// File provider 专属字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<FileInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dependencies: Vec<LockDependency>,
    /// 内嵌子模组
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub implanted: Vec<ImplantedMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthInfo {
    pub project_id: String,
    pub version_id: String,
    /// Modrinth 的 `version_number`
    pub version: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplantedMod {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub filename: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dependencies: Vec<LockDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockDependency {
    pub name: String,
    pub version: String,
}

impl OrbitLockfile {
    pub fn from_path(path: &std::path::Path) -> Result<Self, OrbitError> {
        let content =
            std::fs::read_to_string(path).map_err(|_| OrbitError::LockfileNotFound)?;
        let lockfile: Self = toml::from_str(&content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to parse orbit.lock: {e}")))?;
        Ok(lockfile)
    }

    pub fn to_toml_string(&self) -> Result<String, OrbitError> {
        toml::to_string_pretty(self)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to serialize orbit.lock: {e}")))
    }

    pub fn from_dir(dir: &std::path::Path) -> Result<Self, OrbitError> {
        let path = dir.join("orbit.lock");
        Self::from_path(&path)
    }

    /// 按 mod_id 查找
    pub fn find(&self, mod_id: &str) -> Option<&PackageEntry> {
        self.packages.iter().find(|e| e.mod_id == mod_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lockfile_modrinth() {
        let toml_str = r#"
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "sodium"
version = "0.5.8"
sha256 = "abc123def456"
provider = "modrinth"

[package.modrinth]
project_id = "AANobbMI"
version_id = "abc123mod"
version = "mc1.20.1-0.5.8-fabric"
slug = "sodium"

[[package]]
mod_id = "fabric-api"
version = "0.92.0"
sha1 = "deadbeef"
sha256 = "xyz789"
provider = "modrinth"

[package.modrinth]
project_id = "P7dR8mSH"
version_id = "def456ver"
version = "0.92.0+1.20.1"
slug = "fabric-api"
"#;
        let lockfile: OrbitLockfile = toml::from_str(toml_str).unwrap();
        assert_eq!(lockfile.meta.mc_version, "1.20.1");
        assert_eq!(lockfile.packages.len(), 2);
        let sodium = lockfile.find("sodium").unwrap();
        assert_eq!(sodium.version, "0.5.8");
        assert_eq!(sodium.modrinth.as_ref().unwrap().project_id, "AANobbMI");

        let fa = lockfile.find("fabric-api").unwrap();
        assert_eq!(fa.sha1, "deadbeef");
    }

    #[test]
    fn parse_lockfile_file_type() {
        let toml_str = r#"
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "carpet"
version = "26.1+v260402"
sha256 = "abc123"
provider = "file"

[package.file]
path = "mods/fabric-carpet-26.1+v260402.jar"
"#;
        let lockfile: OrbitLockfile = toml::from_str(toml_str).unwrap();
        let carpet = lockfile.find("carpet").unwrap();
        assert_eq!(carpet.provider, "file");
        assert_eq!(carpet.file.as_ref().unwrap().path, "mods/fabric-carpet-26.1+v260402.jar");
    }

    #[test]
    fn lockfile_roundtrip() {
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".into(),
                modloader: "fabric".into(),
                modloader_version: "0.16.10".into(),
            },
            packages: vec![PackageEntry {
                mod_id: "sodium".into(),
                version: "0.5.8".into(),
                sha1: String::new(),
                sha256: "abc123".into(),
                sha512: String::new(),
                provider: "modrinth".into(),
                modrinth: Some(ModrinthInfo {
                    project_id: "AANobbMI".into(),
                    version_id: "abc123mod".into(),
                    version: "mc1.20.1-0.5.8-fabric".into(),
                    slug: "sodium".into(),
                }),
                file: None,
                dependencies: vec![],
                implanted: vec![],
            }],
        };
        let serialized = lockfile.to_toml_string().unwrap();
        let deserialized: OrbitLockfile = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.packages.len(), 1);
        assert_eq!(deserialized.packages[0].mod_id, "sodium");
    }
}
