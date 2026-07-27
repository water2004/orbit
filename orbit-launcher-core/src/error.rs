use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    #[error("I/O operation failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse launcher config.toml: {0}")]
    ConfigParse(#[source] toml::de::Error),

    #[error("failed to edit launcher config.toml: {0}")]
    ConfigDocumentParse(#[source] toml_edit::TomlError),

    #[error("failed to parse orbit-launcher.toml: {0}")]
    ManifestParse(#[source] toml::de::Error),

    #[error("failed to parse instances.toml: {0}")]
    RegistryParse(#[source] toml::de::Error),

    #[error("failed to serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("invalid launcher configuration: {0}")]
    InvalidConfig(String),

    #[error("invalid instance manifest: {0}")]
    InvalidManifest(String),

    #[error("invalid instances registry: {0}")]
    InvalidRegistry(String),

    #[error("orbit-launcher.toml was not found in '{0}'")]
    ManifestNotFound(PathBuf),

    #[error("instance '{0}' is not registered")]
    InstanceNotFound(String),

    #[error("instance name '{0}' is already registered")]
    DuplicateInstanceName(String),

    #[error("instance ID '{0}' is already registered at another path")]
    DuplicateInstanceId(uuid::Uuid),

    #[error("path '{0}' is already registered to another instance")]
    DuplicateInstancePath(PathBuf),

    #[error("instance root must be an absolute path: '{0}'")]
    RelativeInstanceRoot(PathBuf),

    #[error("instance root is not a directory: '{0}'")]
    InstanceRootNotDirectory(PathBuf),

    #[error("instance context is required; change to an instance directory or pass --instance")]
    InstanceContextRequired,

    #[error(
        "refusing to use default instance '{0}' for this operation; change to its directory or pass --instance"
    )]
    ExplicitInstanceRequired(String),

    #[error("instance registry and manifest disagree: {0}")]
    InstanceRegistryMismatch(String),

    #[error("instance transaction failed: {0}")]
    Transaction(String),

    #[error("system data directories are unsupported on this platform; pass explicit directories")]
    UnsupportedPlatform,
}

impl LauncherError {
    /// Stable code consumed by JSON clients.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::ConfigParse(_) => "config_parse",
            Self::ConfigDocumentParse(_) => "config_parse",
            Self::ManifestParse(_) => "manifest_parse",
            Self::RegistryParse(_) => "registry_parse",
            Self::TomlSerialize(_) => "toml_serialize",
            Self::InvalidConfig(_) => "invalid_config",
            Self::InvalidManifest(_) => "invalid_manifest",
            Self::InvalidRegistry(_) => "invalid_registry",
            Self::ManifestNotFound(_) => "manifest_not_found",
            Self::InstanceNotFound(_) => "instance_not_found",
            Self::DuplicateInstanceName(_) => "duplicate_instance_name",
            Self::DuplicateInstanceId(_) => "duplicate_instance_id",
            Self::DuplicateInstancePath(_) => "duplicate_instance_path",
            Self::RelativeInstanceRoot(_) => "relative_instance_root",
            Self::InstanceRootNotDirectory(_) => "instance_root_not_directory",
            Self::InstanceContextRequired => "instance_context_required",
            Self::ExplicitInstanceRequired(_) => "explicit_instance_required",
            Self::InstanceRegistryMismatch(_) => "instance_registry_mismatch",
            Self::Transaction(_) => "transaction",
            Self::UnsupportedPlatform => "unsupported_platform",
        }
    }
}
