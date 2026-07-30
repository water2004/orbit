use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;

pub const INSTANCE_MANIFEST_FILE: &str = "orbit-launcher.toml";
pub const INSTANCE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstanceKind {
    Client,
    Server,
}

impl InstanceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

impl FromStr for InstanceKind {
    type Err = LauncherError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "client" => Ok(Self::Client),
            "server" => Ok(Self::Server),
            _ => Err(LauncherError::InvalidManifest(format!(
                "unknown instance kind '{value}'; expected client or server"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoaderKind {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    Neoforge,
}

impl LoaderKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
            Self::Forge => "forge",
            Self::Neoforge => "neoforge",
        }
    }
}

impl FromStr for LoaderKind {
    type Err = LauncherError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "vanilla" => Ok(Self::Vanilla),
            "fabric" => Ok(Self::Fabric),
            "quilt" => Ok(Self::Quilt),
            "forge" => Ok(Self::Forge),
            "neoforge" => Ok(Self::Neoforge),
            _ => Err(LauncherError::InvalidManifest(format!(
                "unknown loader '{value}'; expected vanilla, fabric, quilt, forge, or neoforge"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum JavaPolicy {
    #[default]
    Auto,
    Managed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JavaProvider {
    Mojang,
}

impl JavaPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Managed => "managed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Never,
    #[default]
    OnUnexpectedExit,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceManifest {
    pub schema: u32,
    pub id: Uuid,
    pub name: String,
    pub kind: InstanceKind,
    pub minecraft: MinecraftConfig,
    pub loader: LoaderConfig,
    #[serde(default)]
    pub java: JavaConfig,
    #[serde(default)]
    pub launch: LaunchConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerConfig>,
}

impl InstanceManifest {
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        kind: InstanceKind,
        minecraft_requirement: impl Into<String>,
        loader_kind: LoaderKind,
        loader_requirement: Option<String>,
    ) -> Result<Self, LauncherError> {
        let manifest = Self {
            schema: INSTANCE_SCHEMA,
            id,
            name: name.into(),
            kind,
            minecraft: MinecraftConfig {
                requirement: minecraft_requirement.into(),
            },
            loader: LoaderConfig {
                kind: loader_kind,
                requirement: loader_requirement,
            },
            java: JavaConfig::default(),
            launch: LaunchConfig::default(),
            server: (kind == InstanceKind::Server).then(ServerConfig::default),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), LauncherError> {
        if self.schema != INSTANCE_SCHEMA {
            return Err(LauncherError::InvalidManifest(format!(
                "unsupported schema {}; expected {INSTANCE_SCHEMA}",
                self.schema
            )));
        }
        if self.id.is_nil() {
            return Err(LauncherError::InvalidManifest(
                "instance ID cannot be nil".to_string(),
            ));
        }
        validate_instance_name(&self.name)?;
        validate_requirement(&self.minecraft.requirement, "Minecraft")?;
        match self.loader.kind {
            LoaderKind::Vanilla if self.loader.requirement.is_some() => {
                return Err(LauncherError::InvalidManifest(
                    "vanilla loader cannot have a loader version requirement".to_string(),
                ));
            }
            LoaderKind::Vanilla => {}
            _ => validate_requirement(
                self.loader.requirement.as_deref().unwrap_or_default(),
                "loader",
            )?,
        }
        if self.launch.min_memory_mib == 0
            || self.launch.max_memory_mib < self.launch.min_memory_mib
        {
            return Err(LauncherError::InvalidManifest(
                "launch memory must be non-zero and max_memory_mib must be at least min_memory_mib"
                    .to_string(),
            ));
        }
        match (self.kind, &self.server) {
            (InstanceKind::Client, Some(_)) => {
                return Err(LauncherError::InvalidManifest(
                    "client instance cannot contain a [server] section".to_string(),
                ));
            }
            (InstanceKind::Server, None) => {
                return Err(LauncherError::InvalidManifest(
                    "server instance requires a [server] section".to_string(),
                ));
            }
            (_, _) => {}
        }
        if let Some(server) = &self.server {
            server.validate()?;
        }
        self.java.validate()?;
        Ok(())
    }
}

pub fn validate_instance_name(name: &str) -> Result<(), LauncherError> {
    if name.is_empty()
        || name.trim() != name
        || name.len() > 80
        || name.chars().any(char::is_control)
    {
        return Err(LauncherError::InvalidManifest(format!(
            "instance name '{name}' is invalid"
        )));
    }
    if Uuid::parse_str(name).is_ok() {
        return Err(LauncherError::InvalidManifest(
            "instance name cannot itself be a UUID because --instance accepts both names and IDs"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_requirement(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(LauncherError::InvalidManifest(format!(
            "{subject} requirement '{value}' is invalid"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftConfig {
    pub requirement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoaderConfig {
    pub kind: LoaderKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JavaConfig {
    pub policy: JavaPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<JavaProvider>,
}

impl JavaConfig {
    fn validate(&self) -> Result<(), LauncherError> {
        match self.policy {
            JavaPolicy::Auto if self.provider.is_some() => Err(LauncherError::InvalidManifest(
                "java policy auto cannot specify a provider".to_string(),
            )),
            _ => Ok(()),
        }
    }
}

impl Default for JavaConfig {
    fn default() -> Self {
        Self {
            policy: JavaPolicy::Auto,
            provider: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchConfig {
    pub min_memory_mib: u32,
    pub max_memory_mib: u32,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Uuid>,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            min_memory_mib: 512,
            max_memory_mib: 4096,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            account: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub restart: RestartPolicy,
    pub restart_limit: u32,
    pub restart_window_seconds: u64,
    pub restart_backoff_max_seconds: u64,
    pub graceful_stop_timeout_seconds: u64,
    pub kill_timeout_seconds: u64,
    pub authentication: ServerAuthenticationConfig,
}

impl ServerConfig {
    fn validate(&self) -> Result<(), LauncherError> {
        if self.restart_limit == 0
            || self.restart_window_seconds == 0
            || self.restart_backoff_max_seconds == 0
            || self.graceful_stop_timeout_seconds == 0
            || self.kill_timeout_seconds == 0
        {
            return Err(LauncherError::InvalidManifest(
                "server restart limits and timeouts must be greater than zero".to_string(),
            ));
        }
        self.authentication.validate()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            restart: RestartPolicy::OnUnexpectedExit,
            restart_limit: 5,
            restart_window_seconds: 600,
            restart_backoff_max_seconds: 60,
            graceful_stop_timeout_seconds: 30,
            kill_timeout_seconds: 10,
            authentication: ServerAuthenticationConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ServerAuthenticationProvider {
    #[default]
    Mojang,
    ExternalYggdrasil,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerAuthenticationConfig {
    pub provider: ServerAuthenticationProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yggdrasil_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authlib_injector: Option<AuthlibInjectorPolicy>,
}

impl ServerAuthenticationConfig {
    fn validate(&self) -> Result<(), LauncherError> {
        match self.provider {
            ServerAuthenticationProvider::Mojang
                if self.yggdrasil_provider.is_some() || self.authlib_injector.is_some() =>
            {
                Err(LauncherError::InvalidManifest(
                    "Mojang server authentication cannot configure External Yggdrasil fields"
                        .to_string(),
                ))
            }
            ServerAuthenticationProvider::ExternalYggdrasil
                if self.yggdrasil_provider.is_none()
                    || self.authlib_injector != Some(AuthlibInjectorPolicy::Managed) =>
            {
                Err(LauncherError::InvalidManifest(
                    "External Yggdrasil server authentication requires yggdrasil_provider and managed authlib-injector"
                        .to_string(),
                ))
            }
            ServerAuthenticationProvider::ExternalYggdrasil
                if self.yggdrasil_provider.as_ref().is_some_and(|provider| {
                    provider.is_empty()
                        || provider.trim() != provider
                        || provider.chars().any(char::is_control)
                }) =>
            {
                Err(LauncherError::InvalidManifest(
                    "External Yggdrasil provider ID is invalid".to_string(),
                ))
            }
            _ => Ok(()),
        }
    }
}

impl Default for ServerAuthenticationConfig {
    fn default() -> Self {
        Self {
            provider: ServerAuthenticationProvider::Mojang,
            yggdrasil_provider: None,
            authlib_injector: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthlibInjectorPolicy {
    Managed,
}

#[derive(Debug, Clone)]
pub struct ManifestFile {
    root: PathBuf,
    pub inner: InstanceManifest,
}

impl ManifestFile {
    pub fn open(root: &Path) -> Result<Self, LauncherError> {
        let path = root.join(INSTANCE_MANIFEST_FILE);
        if !path.is_file() {
            return Err(LauncherError::ManifestNotFound(root.to_path_buf()));
        }
        let content = std::fs::read_to_string(&path)?;
        let inner: InstanceManifest =
            toml::from_str(&content).map_err(LauncherError::ManifestParse)?;
        inner.validate()?;
        Ok(Self {
            root: root.to_path_buf(),
            inner,
        })
    }

    pub fn new(root: &Path, inner: InstanceManifest) -> Self {
        Self {
            root: root.to_path_buf(),
            inner,
        }
    }

    pub fn save(&self) -> Result<(), LauncherError> {
        self.inner.validate()?;
        let content = toml::to_string_pretty(&self.inner)?;
        write_atomic(&self.root.join(INSTANCE_MANIFEST_FILE), content.as_bytes())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_manifest_roundtrip_keeps_restart_and_authentication_policy() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = InstanceManifest::new(
            Uuid::new_v4(),
            "server",
            InstanceKind::Server,
            "1.21.1",
            LoaderKind::Fabric,
            Some("stable".to_string()),
        )
        .unwrap();
        ManifestFile::new(directory.path(), manifest)
            .save()
            .unwrap();

        let loaded = ManifestFile::open(directory.path()).unwrap();
        assert_eq!(
            loaded.inner.server.unwrap().restart,
            RestartPolicy::OnUnexpectedExit
        );
    }

    #[test]
    fn rejects_loader_and_instance_shape_mismatches() {
        let id = Uuid::new_v4();
        assert!(
            InstanceManifest::new(
                id,
                "vanilla",
                InstanceKind::Client,
                "1.21.1",
                LoaderKind::Vanilla,
                Some("latest".to_string()),
            )
            .is_err()
        );

        let mut client = InstanceManifest::new(
            id,
            "client",
            InstanceKind::Client,
            "1.21.1",
            LoaderKind::Vanilla,
            None,
        )
        .unwrap();
        client.server = Some(ServerConfig::default());
        assert!(client.validate().is_err());
    }

    #[test]
    fn rejects_uuid_shaped_names_to_keep_selector_unambiguous() {
        let name = Uuid::new_v4().to_string();
        assert!(validate_instance_name(&name).is_err());
    }

    #[test]
    fn unsupported_java_branches_are_not_in_the_manifest_schema() {
        let manifest = InstanceManifest::new(
            Uuid::new_v4(),
            "client",
            InstanceKind::Client,
            "1.21.1",
            LoaderKind::Vanilla,
            None,
        )
        .unwrap();
        let document = toml::to_string(&manifest).unwrap().replace(
            "policy = \"auto\"",
            "policy = \"system\"\npath = \"C:/java/bin/java.exe\"",
        );

        assert!(toml::from_str::<InstanceManifest>(&document).is_err());

        let unsupported_provider = toml::to_string(&manifest).unwrap().replace(
            "policy = \"auto\"",
            "policy = \"managed\"\nprovider = \"temurin\"",
        );
        assert!(toml::from_str::<InstanceManifest>(&unsupported_provider).is_err());
    }
}
