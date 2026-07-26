//! orbit.lock 的 serde 结构体与读写。
//!
//! 格式规格参见 docs/orbit-toml-spec.md §4

use serde::{Deserialize, Serialize};

use crate::error::OrbitError;
use crate::manifest::PackageRemote;
use crate::metadata::{
    DependencyExpression, EmbeddedArtifact, Environment, LanguageLoaderRequirement,
    ModLoadCondition, ProvidedMod,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrbitLockfile {
    pub meta: LockMeta,
    #[serde(rename = "package")]
    pub packages: Vec<PackageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockMeta {
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
}

/// `[[package]]` 条目。`mod_id` 为 loader 元数据声明的模组 ID，是 lockfile 的键。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageEntry {
    /// JAR loader 元数据声明的模组 ID
    pub mod_id: String,
    /// JAR loader 元数据声明的版本
    pub version: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    /// JAR 文件名（不含路径），用于升级/删除时定位旧文件
    #[serde(default)]
    pub filename: String,
    /// Every known candidate-discovery entry for this logical package.
    pub remotes: Vec<PackageRemote>,
    /// Sources that can restore the exact selected content hash.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_sources: Vec<ArtifactSource>,
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
    /// 顶层包内容中声明的其他模组模块。
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub bundled: Vec<BundledMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum ArtifactSource {
    File {
        path: String,
    },
    Modrinth {
        project_id: String,
        version_id: String,
        download_url: String,
    },
    Curseforge {
        project_id: u32,
        file_id: u32,
        download_url: String,
    },
}

impl ArtifactSource {
    pub fn provider(&self) -> &'static str {
        match self {
            Self::File { .. } => "file",
            Self::Modrinth { .. } => "modrinth",
            Self::Curseforge { .. } => "curseforge",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundledMod {
    pub mod_id: String,
    pub version: String,
    pub load_condition: ModLoadCondition,
    pub origin: crate::jar::JarModOrigin,
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
            load_condition: metadata.load_condition,
            origin: metadata.origin.clone(),
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
            load_condition: metadata.load_condition,
            origin: metadata.origin.clone(),
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
        let content = std::fs::read_to_string(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                OrbitError::LockfileNotFound
            } else {
                OrbitError::Io(error)
            }
        })?;
        let lockfile: Self = toml::from_str(&content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to parse orbit.lock: {e}")))?;
        lockfile.validate()?;
        Ok(lockfile)
    }

    pub fn to_toml_string(&self) -> Result<String, OrbitError> {
        self.validate()?;
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

    pub fn validate(&self) -> Result<(), OrbitError> {
        let mut packages = std::collections::BTreeSet::new();
        for entry in &self.packages {
            if entry.mod_id.trim().is_empty()
                || entry.version.trim().is_empty()
                || entry.sha512.trim().is_empty()
            {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "every locked package must contain a mod_id, JAR version, and SHA-512 content identity"
                )));
            }
            if !packages.insert(entry.mod_id.as_str()) {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "orbit.lock contains package '{}' more than once",
                    entry.mod_id
                )));
            }
            if entry.remotes.is_empty() {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "locked package '{}' must declare at least one remote",
                    entry.mod_id
                )));
            }
            let mut remotes = std::collections::BTreeSet::new();
            for remote in &entry.remotes {
                remote.validate(&entry.mod_id)?;
                if !remotes.insert(remote) {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "locked package '{}' declares the same remote more than once",
                        entry.mod_id
                    )));
                }
            }
            if entry.artifact_sources.is_empty() {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "locked package '{}' has no exact source for its selected content",
                    entry.mod_id
                )));
            }
            let mut sources = Vec::new();
            for source in &entry.artifact_sources {
                if !source.is_valid() {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "locked package '{}' contains an invalid exact artifact source",
                        entry.mod_id
                    )));
                }
                if sources.contains(&source) {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "locked package '{}' contains the same exact artifact source more than once",
                        entry.mod_id
                    )));
                }
                sources.push(source);
            }
        }
        Ok(())
    }
}

impl PackageEntry {
    pub fn has_online_remote(&self) -> bool {
        self.remotes
            .iter()
            .any(|remote| remote.provider() != "file")
    }
}

impl ArtifactSource {
    fn is_valid(&self) -> bool {
        match self {
            Self::File { path } => !path.trim().is_empty(),
            Self::Modrinth {
                project_id,
                version_id,
                download_url,
            } => {
                !project_id.trim().is_empty()
                    && !version_id.trim().is_empty()
                    && !download_url.trim().is_empty()
            }
            Self::Curseforge {
                project_id,
                file_id,
                download_url,
            } => *project_id > 0 && *file_id > 0 && !download_url.trim().is_empty(),
        }
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
sha512 = "sodium-content"
remotes = [{ type = "modrinth", project_id = "AANobbMI" }]
artifact_sources = [{ type = "modrinth", project_id = "AANobbMI", version_id = "abc123mod", download_url = "https://cdn.modrinth.com/sodium.jar" }]

[[package]]
mod_id = "fabric-api"
version = "0.92.0"
sha1 = "deadbeef"
sha256 = "xyz789"
sha512 = "fabric-api-content"
remotes = [{ type = "modrinth", project_id = "P7dR8mSH" }]
artifact_sources = [{ type = "modrinth", project_id = "P7dR8mSH", version_id = "def456ver", download_url = "https://cdn.modrinth.com/fabric-api.jar" }]
"#;
        let lockfile: OrbitLockfile = toml::from_str(toml_str).unwrap();
        assert_eq!(lockfile.meta.mc_version, "1.20.1");
        assert_eq!(lockfile.packages.len(), 2);
        let sodium = lockfile.find("sodium").unwrap();
        assert_eq!(sodium.version, "0.5.8");
        assert!(matches!(
            sodium.remotes[0],
            PackageRemote::Modrinth { ref project_id } if project_id == "AANobbMI"
        ));

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
sha512 = "carpet-content"
remotes = [{ type = "file", path = "mods/fabric-carpet-26.1+v260402.jar" }]
artifact_sources = [{ type = "file", path = "mods/fabric-carpet-26.1+v260402.jar" }]
"#;
        let lockfile: OrbitLockfile = toml::from_str(toml_str).unwrap();
        let carpet = lockfile.find("carpet").unwrap();
        assert!(matches!(
            &carpet.remotes[0],
            PackageRemote::File { path } if path == "mods/fabric-carpet-26.1+v260402.jar"
        ));
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
sha512 = "example-content"
filename = "example.jar"
remotes = [{ type = "curseforge", project_id = 123 }]
artifact_sources = [{ type = "curseforge", project_id = 123, file_id = 456, download_url = "https://example.invalid/example.jar" }]
"#;

        let lockfile: OrbitLockfile = toml::from_str(source).unwrap();
        let entry = lockfile.find("example").unwrap();
        assert!(matches!(
            entry.artifact_sources[0],
            ArtifactSource::Curseforge { file_id: 456, .. }
        ));

        let serialized = lockfile.to_toml_string().unwrap();
        let roundtrip: OrbitLockfile = toml::from_str(&serialized).unwrap();
        assert_eq!(
            roundtrip.packages[0].artifact_sources,
            entry.artifact_sources
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
                sha512: "sodium-content".into(),
                filename: String::new(),
                remotes: vec![PackageRemote::Modrinth {
                    project_id: "AANobbMI".into(),
                }],
                artifact_sources: vec![ArtifactSource::Modrinth {
                    project_id: "AANobbMI".into(),
                    version_id: "abc123mod".into(),
                    download_url: "https://cdn.modrinth.com/sodium.jar".into(),
                }],
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

    #[test]
    fn obsolete_single_provider_fields_are_rejected() {
        let source = r#"
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "sodium"
version = "0.5.8"
sha256 = "abc123"
sha512 = "sodium-content"
provider = "modrinth"
slug = "sodium"
remotes = [{ type = "modrinth", project_id = "AANobbMI" }]
"#;

        let error = toml::from_str::<OrbitLockfile>(source).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn persisted_lock_requires_a_content_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("orbit.lock");
        std::fs::write(
            &path,
            r#"
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "sodium"
version = "0.5.8"
sha256 = "abc123"
sha512 = ""
remotes = [{ type = "modrinth", project_id = "AANobbMI" }]
artifact_sources = [{ type = "modrinth", project_id = "different-project", version_id = "version", download_url = "https://example.invalid/sodium.jar" }]
"#,
        )
        .unwrap();

        let error = OrbitLockfile::from_path(&path).unwrap_err();
        assert!(error.to_string().contains("content identity"));
    }
}
