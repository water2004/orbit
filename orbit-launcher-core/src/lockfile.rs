use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;
use crate::eula::EulaAcceptance;
use crate::instance::{InstanceKind, LoaderKind};

pub const INSTANCE_LOCK_FILE: &str = "orbit-launcher.lock";
pub const LOCK_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherLock {
    pub schema: u32,
    pub instance_id: Uuid,
    pub kind: InstanceKind,
    pub minecraft: LockedMinecraft,
    pub loader: LockedLoader,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java: Option<LockedJavaRuntime>,
    pub entrypoint: LockedEntrypoint,
    #[serde(default)]
    pub arguments: LockedArguments,
    pub artifacts: Vec<LockedArtifact>,
    #[serde(default)]
    pub generated_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eula: Option<EulaAcceptance>,
}

impl LauncherLock {
    pub fn validate(&self) -> Result<(), LauncherError> {
        if self.schema != LOCK_SCHEMA {
            return Err(LauncherError::InvalidLock(format!(
                "unsupported schema {}; expected {LOCK_SCHEMA}",
                self.schema
            )));
        }
        if self.instance_id.is_nil() {
            return Err(LauncherError::InvalidLock(
                "instance ID cannot be nil".to_string(),
            ));
        }
        self.minecraft.validate()?;
        self.loader.validate()?;
        if let Some(java) = &self.java {
            java.validate()?;
        }
        self.entrypoint.validate()?;
        let mut paths = HashSet::new();
        for artifact in &self.artifacts {
            artifact.validate()?;
            if !paths.insert(artifact.path.as_str()) {
                return Err(LauncherError::InvalidLock(format!(
                    "duplicate artifact path '{}'",
                    artifact.path
                )));
            }
        }
        for path in &self.generated_files {
            validate_relative_path(path)?;
            if !paths.insert(path.as_str()) {
                return Err(LauncherError::InvalidLock(format!(
                    "duplicate owned path '{path}'"
                )));
            }
        }
        if let Some(eula) = &self.eula
            && (eula.url != crate::eula::MINECRAFT_EULA_URL
                || eula.accepted_at_unix_seconds == 0
                || validate_digest(&eula.digest_sha256, 64, "EULA SHA-256").is_err())
        {
            return Err(LauncherError::InvalidLock(
                "server EULA acceptance receipt is invalid".to_string(),
            ));
        }
        match self.kind {
            InstanceKind::Server if self.eula.is_none() => Err(LauncherError::InvalidLock(
                "server lock requires an EULA acceptance receipt".to_string(),
            )),
            InstanceKind::Client if self.eula.is_some() => Err(LauncherError::InvalidLock(
                "client lock cannot contain a server EULA receipt".to_string(),
            )),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedMinecraft {
    pub version: String,
    pub version_type: String,
    pub version_manifest_url: String,
    pub version_manifest_sha256: String,
    pub version_json_url: String,
    pub version_json_sha1: String,
}

impl LockedMinecraft {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_text(&self.version, "Minecraft version")?;
        validate_text(&self.version_type, "Minecraft version type")?;
        validate_https(&self.version_manifest_url, "Minecraft version manifest")?;
        validate_https(&self.version_json_url, "Minecraft version JSON")?;
        validate_digest(
            &self.version_manifest_sha256,
            64,
            "version manifest SHA-256",
        )?;
        validate_digest(&self.version_json_sha1, 40, "version JSON SHA-1")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedLoader {
    pub kind: LoaderKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_sha256: Option<String>,
}

impl LockedLoader {
    fn validate(&self) -> Result<(), LauncherError> {
        match self.kind {
            LoaderKind::Vanilla
                if self.version.is_some()
                    || self.profile_url.is_some()
                    || self.profile_sha256.is_some() =>
            {
                Err(LauncherError::InvalidLock(
                    "Vanilla lock cannot contain Loader profile fields".to_string(),
                ))
            }
            LoaderKind::Vanilla => Ok(()),
            _ => {
                validate_text(
                    self.version.as_deref().unwrap_or_default(),
                    "Loader version",
                )?;
                validate_https(
                    self.profile_url.as_deref().unwrap_or_default(),
                    "Loader profile",
                )?;
                validate_digest(
                    self.profile_sha256.as_deref().unwrap_or_default(),
                    64,
                    "Loader profile SHA-256",
                )
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedJavaRuntime {
    pub runtime_id: String,
    pub provider: String,
    pub version: String,
    pub major: u32,
    pub platform: String,
    pub executable: String,
}

impl LockedJavaRuntime {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_text(&self.runtime_id, "Java runtime ID")?;
        validate_text(&self.provider, "Java provider")?;
        validate_text(&self.version, "Java version")?;
        validate_text(&self.platform, "Java platform")?;
        validate_relative_path(&self.executable)?;
        if self.major == 0 {
            return Err(LauncherError::InvalidLock(
                "Java major version must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LockedEntrypoint {
    Jar { path: String },
    Classpath { main_class: String },
}

impl LockedEntrypoint {
    fn validate(&self) -> Result<(), LauncherError> {
        match self {
            Self::Jar { path } => validate_relative_path(path),
            Self::Classpath { main_class } => validate_text(main_class, "main class"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LockedArguments {
    pub jvm: Vec<String>,
    pub game: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactOwner {
    Minecraft,
    Loader,
    Java,
    AuthlibInjector,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedArtifact {
    pub logical_name: String,
    pub owner: ArtifactOwner,
    pub source_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_sha1: Option<String>,
    pub sha256: String,
    pub size: u64,
    pub path: String,
}

impl LockedArtifact {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_text(&self.logical_name, "artifact logical name")?;
        validate_https(&self.source_url, "artifact source")?;
        if let Some(sha1) = &self.upstream_sha1 {
            validate_digest(sha1, 40, "artifact SHA-1")?;
        }
        validate_digest(&self.sha256, 64, "artifact SHA-256")?;
        validate_relative_path(&self.path)?;
        if self.size == 0 {
            return Err(LauncherError::InvalidLock(format!(
                "artifact '{}' has zero size",
                self.logical_name
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LockFile {
    root: PathBuf,
    pub inner: LauncherLock,
}

impl LockFile {
    pub fn open(root: &Path) -> Result<Self, LauncherError> {
        let content = std::fs::read_to_string(root.join(INSTANCE_LOCK_FILE))?;
        let inner: LauncherLock = toml::from_str(&content).map_err(LauncherError::LockParse)?;
        inner.validate()?;
        Ok(Self {
            root: root.to_path_buf(),
            inner,
        })
    }

    pub fn new(root: &Path, inner: LauncherLock) -> Self {
        Self {
            root: root.to_path_buf(),
            inner,
        }
    }

    pub fn save(&self) -> Result<(), LauncherError> {
        self.inner.validate()?;
        let content = toml::to_string_pretty(&self.inner)?;
        write_atomic(&self.root.join(INSTANCE_LOCK_FILE), content.as_bytes())
    }
}

pub fn portable_relative_path(path: &Path) -> Result<String, LauncherError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    LauncherError::InvalidLock(format!("path '{}' is not UTF-8", path.display()))
                })?;
                if value.contains(['/', '\\']) {
                    return Err(LauncherError::InvalidLock(format!(
                        "path component '{value}' is not portable"
                    )));
                }
                parts.push(value);
            }
            _ => {
                return Err(LauncherError::InvalidLock(format!(
                    "path '{}' is not a normalized relative path",
                    path.display()
                )));
            }
        }
    }
    if parts.is_empty() {
        return Err(LauncherError::InvalidLock(
            "owned path cannot be empty".to_string(),
        ));
    }
    Ok(parts.join("/"))
}

fn validate_relative_path(value: &str) -> Result<(), LauncherError> {
    if value.is_empty()
        || value.contains('\\')
        || value.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(LauncherError::InvalidLock(format!(
            "'{value}' is not a normalized portable relative path"
        )));
    }
    Ok(())
}

fn validate_text(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(LauncherError::InvalidLock(format!(
            "{subject} '{value}' is invalid"
        )));
    }
    Ok(())
}

fn validate_https(value: &str, subject: &str) -> Result<(), LauncherError> {
    let url = url::Url::parse(value)
        .map_err(|error| LauncherError::InvalidLock(format!("invalid {subject} URL: {error}")))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(LauncherError::InvalidLock(format!(
            "{subject} URL must use HTTPS"
        )));
    }
    Ok(())
}

fn validate_digest(value: &str, length: usize, subject: &str) -> Result<(), LauncherError> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidLock(format!(
            "{subject} '{value}' is invalid"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eula::EulaAcceptanceMethod;

    fn server_lock() -> LauncherLock {
        LauncherLock {
            schema: LOCK_SCHEMA,
            instance_id: Uuid::new_v4(),
            kind: InstanceKind::Server,
            minecraft: LockedMinecraft {
                version: "1.21.1".to_string(),
                version_type: "release".to_string(),
                version_manifest_url:
                    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json".to_string(),
                version_manifest_sha256: "a".repeat(64),
                version_json_url: "https://piston-meta.mojang.com/version.json".to_string(),
                version_json_sha1: "b".repeat(40),
            },
            loader: LockedLoader {
                kind: LoaderKind::Vanilla,
                version: None,
                profile_url: None,
                profile_sha256: None,
            },
            java: None,
            entrypoint: LockedEntrypoint::Jar {
                path: "server.jar".to_string(),
            },
            arguments: LockedArguments::default(),
            artifacts: vec![LockedArtifact {
                logical_name: "Minecraft server".to_string(),
                owner: ArtifactOwner::Minecraft,
                source_url: "https://piston-data.mojang.com/server.jar".to_string(),
                upstream_sha1: Some("c".repeat(40)),
                sha256: "d".repeat(64),
                size: 100,
                path: "server.jar".to_string(),
            }],
            generated_files: vec!["eula.txt".to_string()],
            eula: Some(EulaAcceptance {
                url: crate::eula::MINECRAFT_EULA_URL.to_string(),
                digest_sha256: "e".repeat(64),
                accepted_at_unix_seconds: 1,
                method: EulaAcceptanceMethod::DigestCommand,
            }),
        }
    }

    #[test]
    fn strict_lock_roundtrip_preserves_portable_paths() {
        let directory = tempfile::tempdir().unwrap();
        LockFile::new(directory.path(), server_lock())
            .save()
            .unwrap();
        let lock = LockFile::open(directory.path()).unwrap();
        assert_eq!(lock.inner.artifacts[0].path, "server.jar");
    }

    #[test]
    fn lock_rejects_parent_paths_and_duplicate_owned_files() {
        let mut lock = server_lock();
        lock.generated_files.push("../outside".to_string());
        assert!(lock.validate().is_err());

        let mut lock = server_lock();
        lock.generated_files.push("server.jar".to_string());
        assert!(lock.validate().is_err());
    }
}
