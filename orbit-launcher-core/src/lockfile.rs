use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;
use crate::eula::EulaAcceptance;
use crate::instance::{InstanceKind, LoaderKind};

pub const INSTANCE_LOCK_FILE: &str = "orbit-launcher.lock";
pub const LOCK_SCHEMA: u32 = 6;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authlib_injector: Option<LockedAuthlibInjector>,
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
        if let Some(authlib_injector) = &self.authlib_injector {
            authlib_injector.validate()?;
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
        let authlib_artifacts: Vec<_> = self
            .artifacts
            .iter()
            .filter(|artifact| artifact.owner == ArtifactOwner::AuthlibInjector)
            .collect();
        match (&self.authlib_injector, authlib_artifacts.as_slice()) {
            (None, []) => {}
            (Some(locked), [artifact]) if artifact.path == locked.path => {}
            (Some(_), _) => {
                return Err(LauncherError::InvalidLock(
                    "locked authlib-injector must identify exactly one authlib-injector artifact"
                        .to_string(),
                ));
            }
            (None, _) => {
                return Err(LauncherError::InvalidLock(
                    "authlib-injector artifact has no corresponding lock metadata".to_string(),
                ));
            }
        }
        if let Some(asset_index) = &self.minecraft.asset_index {
            let expected_path = asset_index.artifact_path();
            let artifact = self
                .artifacts
                .iter()
                .find(|artifact| artifact.path == expected_path)
                .ok_or_else(|| {
                    LauncherError::InvalidLock(format!(
                        "asset index artifact '{expected_path}' is missing"
                    ))
                })?;
            if artifact.owner != ArtifactOwner::Minecraft {
                return Err(LauncherError::InvalidLock(format!(
                    "asset index artifact '{expected_path}' is not owned by Minecraft"
                )));
            }
            match &artifact.source {
                LockedArtifactSource::Download {
                    upstream_sha1: Some(upstream_sha1),
                    ..
                } if upstream_sha1 == &asset_index.runtime_name => {}
                _ => {
                    return Err(LauncherError::InvalidLock(format!(
                        "asset index artifact '{expected_path}' does not match its content-addressed runtime name"
                    )));
                }
            }
        }
        match &self.entrypoint {
            LockedEntrypoint::Jar { path } | LockedEntrypoint::ArgumentFile { path } => {
                if !paths.contains(path.as_str()) {
                    return Err(LauncherError::InvalidLock(format!(
                        "entrypoint file '{path}' is not present in the artifact inventory"
                    )));
                }
            }
            LockedEntrypoint::Classpath { classpath, .. } => {
                for entry in classpath {
                    if !paths.contains(entry.as_str()) {
                        return Err(LauncherError::InvalidLock(format!(
                            "classpath entry '{entry}' is not present in the artifact inventory"
                        )));
                    }
                }
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
            InstanceKind::Server if self.minecraft.asset_index.is_some() => {
                Err(LauncherError::InvalidLock(
                    "server lock cannot contain a client asset index".to_string(),
                ))
            }
            InstanceKind::Client if self.eula.is_some() => Err(LauncherError::InvalidLock(
                "client lock cannot contain a server EULA receipt".to_string(),
            )),
            InstanceKind::Client if self.minecraft.asset_index.is_none() => {
                Err(LauncherError::InvalidLock(
                    "client lock requires a content-addressed asset index".to_string(),
                ))
            }
            InstanceKind::Client if self.authlib_injector.is_none() => {
                Err(LauncherError::InvalidLock(
                    "client lock requires a managed authlib-injector artifact".to_string(),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedMinecraft {
    pub version: String,
    pub version_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<LockedAssetIndex>,
    pub version_manifest_url: String,
    pub version_manifest_sha256: String,
    pub version_json_url: String,
    pub version_json_sha1: String,
}

impl LockedMinecraft {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_text(&self.version, "Minecraft version")?;
        validate_text(&self.version_type, "Minecraft version type")?;
        if let Some(asset_index) = &self.asset_index {
            asset_index.validate()?;
        }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedAssetIndex {
    /// Identifier published by Mojang. It is retained for provenance and is
    /// deliberately not used as a shared filesystem key.
    pub upstream_id: String,
    /// SHA-1 content name used by both `${assets_index_name}` and the index
    /// filename, so mutable upstream IDs cannot collide.
    pub runtime_name: String,
}

impl LockedAssetIndex {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_text(&self.upstream_id, "Minecraft upstream asset index ID")?;
        validate_digest(&self.runtime_name, 40, "Minecraft asset index runtime name")
    }

    pub fn artifact_path(&self) -> String {
        format!("assets/indexes/{}.json", self.runtime_name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedLoader {
    pub kind: LoaderKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub source: LockedLoaderSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LockedLoaderSource {
    Vanilla,
    Profile {
        url: String,
        sha256: String,
    },
    Installer {
        url: String,
        sha256: String,
        install_profile_sha256: String,
    },
}

impl LockedLoader {
    pub const fn vanilla() -> Self {
        Self {
            kind: LoaderKind::Vanilla,
            version: None,
            source: LockedLoaderSource::Vanilla,
        }
    }

    pub fn profile(kind: LoaderKind, version: String, url: String, sha256: String) -> Self {
        Self {
            kind,
            version: Some(version),
            source: LockedLoaderSource::Profile { url, sha256 },
        }
    }

    pub fn installer(
        kind: LoaderKind,
        version: String,
        url: String,
        sha256: String,
        install_profile_sha256: String,
    ) -> Self {
        Self {
            kind,
            version: Some(version),
            source: LockedLoaderSource::Installer {
                url,
                sha256,
                install_profile_sha256,
            },
        }
    }

    fn validate(&self) -> Result<(), LauncherError> {
        match (&self.kind, &self.source) {
            (LoaderKind::Vanilla, LockedLoaderSource::Vanilla) if self.version.is_none() => Ok(()),
            (LoaderKind::Vanilla, _) => Err(LauncherError::InvalidLock(
                "Vanilla lock cannot contain a Loader version or external source".to_string(),
            )),
            (
                LoaderKind::Fabric | LoaderKind::Quilt,
                LockedLoaderSource::Profile { url, sha256 },
            ) => {
                validate_text(
                    self.version.as_deref().unwrap_or_default(),
                    "Loader version",
                )?;
                validate_https(url, "Loader profile")?;
                validate_digest(sha256, 64, "Loader profile SHA-256")
            }
            (
                LoaderKind::Forge | LoaderKind::Neoforge,
                LockedLoaderSource::Installer {
                    url,
                    sha256,
                    install_profile_sha256,
                },
            ) => {
                validate_text(
                    self.version.as_deref().unwrap_or_default(),
                    "Loader version",
                )?;
                validate_https(url, "Loader installer")?;
                validate_digest(sha256, 64, "Loader installer SHA-256")?;
                validate_digest(install_profile_sha256, 64, "Loader install profile SHA-256")
            }
            _ => Err(LauncherError::InvalidLock(format!(
                "Loader '{}' has an incompatible source kind",
                self.kind.as_str()
            ))),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedAuthlibInjector {
    pub version: String,
    pub build_number: u32,
    pub path: String,
}

impl LockedAuthlibInjector {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_text(&self.version, "authlib-injector version")?;
        validate_relative_path(&self.path)?;
        if self.build_number == 0 {
            return Err(LauncherError::InvalidLock(
                "authlib-injector build number must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

impl LockedJavaRuntime {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_identifier(&self.runtime_id, "Java runtime ID")?;
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
    Jar {
        path: String,
    },
    ArgumentFile {
        path: String,
    },
    Classpath {
        main_class: String,
        classpath: Vec<String>,
    },
}

impl LockedEntrypoint {
    fn validate(&self) -> Result<(), LauncherError> {
        match self {
            Self::Jar { path } | Self::ArgumentFile { path } => validate_relative_path(path),
            Self::Classpath {
                main_class,
                classpath,
            } => {
                validate_text(main_class, "main class")?;
                if classpath.is_empty() {
                    return Err(LauncherError::InvalidLock(
                        "client classpath cannot be empty".to_string(),
                    ));
                }
                let mut unique = HashSet::new();
                for path in classpath {
                    validate_relative_path(path)?;
                    if !unique.insert(path) {
                        return Err(LauncherError::InvalidLock(format!(
                            "duplicate classpath entry '{path}'"
                        )));
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LockedArguments {
    pub jvm: Vec<String>,
    pub game: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub source: LockedArtifactSource,
    pub sha256: String,
    pub size: u64,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_extraction: Option<LockedNativeExtraction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedNativeExtraction {
    #[serde(default)]
    pub excludes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LockedArtifactSource {
    Download {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        upstream_sha1: Option<String>,
    },
    InstallerOutput {
        installer_sha256: String,
    },
    Derived {
        source_sha256: String,
        description: String,
    },
}

impl LockedArtifact {
    fn validate(&self) -> Result<(), LauncherError> {
        validate_text(&self.logical_name, "artifact logical name")?;
        match &self.source {
            LockedArtifactSource::Download { url, upstream_sha1 } => {
                validate_https(url, "artifact source")?;
                if let Some(sha1) = upstream_sha1 {
                    validate_digest(sha1, 40, "artifact SHA-1")?;
                }
            }
            LockedArtifactSource::InstallerOutput { installer_sha256 } => {
                validate_digest(installer_sha256, 64, "producer installer SHA-256")?;
            }
            LockedArtifactSource::Derived {
                source_sha256,
                description,
            } => {
                validate_digest(source_sha256, 64, "derived artifact source SHA-256")?;
                validate_text(description, "derived artifact description")?;
            }
        }
        validate_digest(&self.sha256, 64, "artifact SHA-256")?;
        validate_relative_path(&self.path)?;
        if let Some(native) = &self.native_extraction {
            let mut unique = HashSet::new();
            for prefix in &native.excludes {
                validate_archive_prefix(prefix)?;
                if !unique.insert(prefix) {
                    return Err(LauncherError::InvalidLock(format!(
                        "duplicate native extraction exclusion '{prefix}'"
                    )));
                }
            }
        }
        if self.size == 0 && matches!(self.source, LockedArtifactSource::Download { .. }) {
            return Err(LauncherError::InvalidLock(format!(
                "artifact '{}' has zero size",
                self.logical_name
            )));
        }
        Ok(())
    }
}

fn validate_archive_prefix(prefix: &str) -> Result<(), LauncherError> {
    let trimmed = prefix.trim_end_matches('/');
    if trimmed.is_empty()
        || prefix.contains('\\')
        || trimmed
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(LauncherError::InvalidLock(format!(
            "native extraction exclusion '{prefix}' is unsafe"
        )));
    }
    Ok(())
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

    pub fn open_optional(root: &Path) -> Result<Option<Self>, LauncherError> {
        let path = root.join(INSTANCE_LOCK_FILE);
        if !path.exists() {
            return Ok(None);
        }
        Self::open(root).map(Some)
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

fn validate_identifier(value: &str, subject: &str) -> Result<(), LauncherError> {
    validate_text(value, subject)?;
    if value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(LauncherError::InvalidLock(format!(
            "{subject} '{value}' is not a portable identifier"
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
                asset_index: None,
                version_manifest_url:
                    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json".to_string(),
                version_manifest_sha256: "a".repeat(64),
                version_json_url: "https://piston-meta.mojang.com/version.json".to_string(),
                version_json_sha1: "b".repeat(40),
            },
            loader: LockedLoader::vanilla(),
            java: None,
            authlib_injector: None,
            entrypoint: LockedEntrypoint::Jar {
                path: "server.jar".to_string(),
            },
            arguments: LockedArguments::default(),
            artifacts: vec![LockedArtifact {
                logical_name: "Minecraft server".to_string(),
                owner: ArtifactOwner::Minecraft,
                source: LockedArtifactSource::Download {
                    url: "https://piston-data.mojang.com/server.jar".to_string(),
                    upstream_sha1: Some("c".repeat(40)),
                },
                sha256: "d".repeat(64),
                size: 100,
                path: "server.jar".to_string(),
                native_extraction: None,
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

    fn client_lock() -> LauncherLock {
        let runtime_name = "f".repeat(40);
        LauncherLock {
            schema: LOCK_SCHEMA,
            instance_id: Uuid::new_v4(),
            kind: InstanceKind::Client,
            minecraft: LockedMinecraft {
                version: "26.2".to_string(),
                version_type: "release".to_string(),
                asset_index: Some(LockedAssetIndex {
                    upstream_id: "32".to_string(),
                    runtime_name: runtime_name.clone(),
                }),
                version_manifest_url:
                    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json".to_string(),
                version_manifest_sha256: "a".repeat(64),
                version_json_url: "https://piston-meta.mojang.com/version.json".to_string(),
                version_json_sha1: "b".repeat(40),
            },
            loader: LockedLoader::vanilla(),
            java: None,
            authlib_injector: Some(LockedAuthlibInjector {
                version: "1.2.8".to_string(),
                build_number: 1,
                path: "libraries/authlib-injector.jar".to_string(),
            }),
            entrypoint: LockedEntrypoint::Classpath {
                main_class: "net.minecraft.client.main.Main".to_string(),
                classpath: vec!["instances/client/minecraft.jar".to_string()],
            },
            arguments: LockedArguments::default(),
            artifacts: vec![
                LockedArtifact {
                    logical_name: "Minecraft 32 asset index".to_string(),
                    owner: ArtifactOwner::Minecraft,
                    source: LockedArtifactSource::Download {
                        url: "https://piston-meta.mojang.com/32.json".to_string(),
                        upstream_sha1: Some(runtime_name.clone()),
                    },
                    sha256: "c".repeat(64),
                    size: 100,
                    path: format!("assets/indexes/{runtime_name}.json"),
                    native_extraction: None,
                },
                LockedArtifact {
                    logical_name: "Minecraft client".to_string(),
                    owner: ArtifactOwner::Minecraft,
                    source: LockedArtifactSource::Download {
                        url: "https://piston-data.mojang.com/client.jar".to_string(),
                        upstream_sha1: Some("d".repeat(40)),
                    },
                    sha256: "e".repeat(64),
                    size: 100,
                    path: "instances/client/minecraft.jar".to_string(),
                    native_extraction: None,
                },
                LockedArtifact {
                    logical_name: "authlib-injector".to_string(),
                    owner: ArtifactOwner::AuthlibInjector,
                    source: LockedArtifactSource::Download {
                        url: "https://example.invalid/authlib-injector.jar".to_string(),
                        upstream_sha1: Some("1".repeat(40)),
                    },
                    sha256: "2".repeat(64),
                    size: 100,
                    path: "libraries/authlib-injector.jar".to_string(),
                    native_extraction: None,
                },
            ],
            generated_files: Vec::new(),
            eula: None,
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
    fn client_lock_roundtrip_preserves_asset_index_provenance_and_runtime_name() {
        let directory = tempfile::tempdir().unwrap();
        LockFile::new(directory.path(), client_lock())
            .save()
            .unwrap();
        let lock = LockFile::open(directory.path()).unwrap();
        let asset_index = lock.inner.minecraft.asset_index.unwrap();

        assert_eq!(asset_index.upstream_id, "32");
        assert_eq!(asset_index.runtime_name, "f".repeat(40));
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

    #[test]
    fn client_lock_binds_runtime_asset_name_to_artifact_content() {
        let lock = client_lock();
        lock.validate().unwrap();

        let mut mismatched = lock.clone();
        let asset_index = mismatched.minecraft.asset_index.as_mut().unwrap();
        asset_index.runtime_name = "0".repeat(40);
        assert!(mismatched.validate().is_err());
    }
}
