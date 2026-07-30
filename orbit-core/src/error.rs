use thiserror::Error;

#[derive(Error, Debug)]
pub enum OrbitError {
    #[error("orbit.toml not found in this directory")]
    ManifestNotFound,

    #[error("failed to parse orbit.toml: {0}")]
    ManifestParse(#[from] toml::de::Error),

    #[error("failed to serialize orbit.toml: {0}")]
    ManifestSerialize(#[from] toml::ser::Error),

    #[error(
        "orbit.lock not found — run 'orbit sync' to record local state or 'orbit fix' to resolve orbit.toml"
    )]
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

    #[error("operation cancelled: {0}")]
    Cancelled(String),

    #[error(
        "content verification failed for '{name}'; downloaded bytes differ from the trusted source"
    )]
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

    #[error(
        "{provider} provider requires an API key; set {environment_variable} or \
         {config_key} in config.toml"
    )]
    ProviderApiKeyRequired {
        provider: &'static str,
        environment_variable: &'static str,
        config_key: &'static str,
    },

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::OrbitError;

    #[test]
    fn content_identity_values_are_not_rendered_to_users() {
        let error = OrbitError::ChecksumMismatch {
            name: "example".to_string(),
            expected: "private-expected-hash".to_string(),
            actual: "private-actual-hash".to_string(),
        };

        let rendered = error.to_string();
        assert!(!rendered.contains("private-expected-hash"));
        assert!(!rendered.contains("private-actual-hash"));
        assert!(rendered.contains("content verification failed"));
    }
}
