//! orbit.lock 的 serde 结构体与读写。
//!
//! 格式规格参见 docs/orbit-toml-spec.md §4

use serde::{Deserialize, Serialize};

use crate::error::OrbitError;
use crate::metadata::{
    DependencyExpression, EmbeddedArtifact, Environment, LanguageLoaderRequirement, ProvidedMod,
};

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

/// `[[package]]` 条目。`mod_id` 为 loader 元数据声明的模组 ID，是 lockfile 的键。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    /// JAR loader 元数据声明的模组 ID
    pub mod_id: String,
    /// JAR loader 元数据声明的版本
    pub version: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sha1: String,
    pub sha256: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sha512: String,
    /// JAR 文件名（不含路径），用于升级/删除时定位旧文件
    #[serde(default)]
    pub filename: String,
    /// "modrinth" | "curseforge" | "file"
    pub provider: String,
    /// Modrinth provider 专属字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modrinth: Option<ModrinthInfo>,
    /// CurseForge provider 专属字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curseforge: Option<CurseForgeInfo>,
    /// File provider 专属字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<FileInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dependencies: Vec<DependencyExpression>,
    #[serde(default)]
    pub environment: Environment,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub provides: Vec<ProvidedMod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_loader: Option<LanguageLoaderRequirement>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    /// 同一物理文件提供的其他逻辑模组。
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub bundled: Vec<BundledMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModrinthInfo {
    pub project_id: String,
    pub version_id: String,
    /// Modrinth 的 `version_number`
    pub version: String,
    pub slug: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurseForgeInfo {
    pub project_id: u32,
    pub file_id: u32,
    /// CurseForge 文件的展示名称，不代替 JAR 自声明版本。
    pub display_name: String,
    pub slug: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledMod {
    pub mod_id: String,
    pub version: String,
    #[serde(default)]
    pub environment: Environment,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dependencies: Vec<DependencyExpression>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub provides: Vec<ProvidedMod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_loader: Option<LanguageLoaderRequirement>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub bundled: Vec<BundledMod>,
}

impl BundledMod {
    pub fn from_jar_metadata(metadata: &crate::jar::JarModMetadata) -> Self {
        Self {
            mod_id: metadata.mod_id.clone(),
            version: metadata.version.clone(),
            environment: metadata.environment,
            dependencies: metadata.dependencies.clone(),
            provides: metadata.provides.clone(),
            language_loader: metadata.language_loader.clone(),
            embedded_artifacts: metadata.embedded_artifacts.clone(),
            bundled: metadata
                .bundled_mods
                .iter()
                .map(Self::from_jar_metadata)
                .collect(),
        }
    }

    pub(crate) fn from_candidate(metadata: &crate::resolver::types::BundledCandidate) -> Self {
        Self {
            mod_id: metadata.mod_id.clone(),
            version: metadata.version.clone(),
            environment: metadata.environment,
            dependencies: metadata.dependencies.clone(),
            provides: metadata.provides.clone(),
            language_loader: metadata.language_loader.clone(),
            embedded_artifacts: metadata.embedded_artifacts.clone(),
            bundled: metadata.bundled.iter().map(Self::from_candidate).collect(),
        }
    }
}

impl OrbitLockfile {
    pub fn from_path(path: &std::path::Path) -> Result<Self, OrbitError> {
        let content = std::fs::read_to_string(path).map_err(|_| OrbitError::LockfileNotFound)?;
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

impl PackageEntry {
    pub fn source_slug(&self) -> Option<&str> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.slug.as_str())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.slug.as_str())
            })
    }

    pub fn source_project_id(&self) -> Option<String> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.project_id.clone())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.project_id.to_string())
            })
    }

    pub fn source_version_id(&self) -> Option<String> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.version_id.clone())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.file_id.to_string())
            })
    }

    pub fn source_version(&self) -> Option<&str> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.version.as_str())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.display_name.as_str())
            })
    }

    pub fn source_download_url(&self) -> Option<&str> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.download_url.as_str())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.download_url.as_str())
            })
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
        assert_eq!(
            carpet.file.as_ref().unwrap().path,
            "mods/fabric-carpet-26.1+v260402.jar"
        );
    }

    #[test]
    fn curseforge_metadata_roundtrips_in_its_own_subtable() {
        let source = r#"
[meta]
mc_version = "1.21.1"
modloader = "neoforge"
modloader_version = "21.1.0"

[[package]]
mod_id = "example"
version = "2.0.0"
sha1 = "deadbeef"
sha256 = "cafebabe"
filename = "example.jar"
provider = "curseforge"

[package.curseforge]
project_id = 123
file_id = 456
display_name = "Example 2"
slug = "example"
download_url = "https://example.invalid/example.jar"
"#;

        let lockfile: OrbitLockfile = toml::from_str(source).unwrap();
        let entry = lockfile.find("example").unwrap();
        assert_eq!(entry.source_slug(), Some("example"));
        assert_eq!(entry.source_project_id().as_deref(), Some("123"));
        assert_eq!(entry.source_version_id().as_deref(), Some("456"));

        let serialized = lockfile.to_toml_string().unwrap();
        let roundtrip: OrbitLockfile = toml::from_str(&serialized).unwrap();
        assert_eq!(
            roundtrip.packages[0]
                .curseforge
                .as_ref()
                .unwrap()
                .display_name,
            "Example 2"
        );
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
                filename: String::new(),
                provider: "modrinth".into(),
                modrinth: Some(ModrinthInfo {
                    project_id: "AANobbMI".into(),
                    version_id: "abc123mod".into(),
                    version: "mc1.20.1-0.5.8-fabric".into(),
                    slug: "sodium".into(),
                    download_url: String::new(),
                }),
                curseforge: None,
                file: None,
                dependencies: vec![],
                environment: Environment::Both,
                provides: vec![],
                language_loader: None,
                embedded_artifacts: vec![],
                bundled: vec![],
            }],
        };
        let serialized = lockfile.to_toml_string().unwrap();
        let deserialized: OrbitLockfile = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.packages.len(), 1);
        assert_eq!(deserialized.packages[0].mod_id, "sodium");
    }
}
