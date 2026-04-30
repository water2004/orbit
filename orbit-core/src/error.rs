use thiserror::Error;

#[derive(Error, Debug)]
pub enum OrbitError {
    #[error("orbit.toml not found in this directory")]
    ManifestNotFound,

    #[error("failed to parse orbit.toml: {0}")]
    ManifestParse(#[from] toml::de::Error),

    #[error("failed to serialize orbit.toml: {0}")]
    ManifestSerialize(#[from] toml::ser::Error),

    #[error("orbit.lock not found — run 'orbit install' first")]
    LockfileNotFound,

    #[error("mod '{0}' not found")]
    ModNotFound(String),

    #[error("no version of '{mod_name}' satisfies constraint '{constraint}'")]
    VersionMismatch {
        mod_name: String,
        constraint: String,
    },

    #[error("dependency conflict: {0}")]
    Conflict(String),

    #[error("checksum mismatch for '{name}': expected {expected}, got {actual}")]
    ChecksumMismatch {
        name: String,
        expected: String,
        actual: String,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
