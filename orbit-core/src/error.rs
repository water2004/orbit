use thiserror::Error;

#[derive(Error, Debug)]
pub enum RuntimeDataError {
    #[error("observation snapshot has a non-Unicode name at '{path}'")]
    NonUnicodeSnapshotName { path: String },
    #[error("invalid observation at {path}:{line}: {detail}")]
    InvalidObservation {
        path: String,
        line: usize,
        detail: String,
    },
    #[error("package '{package}' changed after the purge plan was created; request a new plan")]
    PackageChanged { package: String },
    #[error(
        "package '{package}' was removed, but deleting '{path}' failed after {completed} path(s): {detail}"
    )]
    DeleteAfterPackageRemoval {
        package: String,
        path: String,
        completed: usize,
        detail: String,
    },
    #[error("failed to parse runtime ownership ledger '{path}': {detail}")]
    LedgerParse { path: String, detail: String },
    #[error("unsupported runtime ownership schema {schema} in '{path}'")]
    UnsupportedLedgerSchema { schema: u32, path: String },
    #[error("failed to serialize runtime ownership: {detail}")]
    LedgerSerialize { detail: String },
    #[error("unsafe instance-relative data path '{path}'")]
    UnsafeRelativePath { path: String },
    #[error("refusing to remove the instance root")]
    InstanceRoot,
    #[error("refusing to remove Orbit control data")]
    ControlData,
    #[error("refusing to remove shared instance root '{path}' as a tree")]
    SharedInstanceRoot { path: String },
    #[error("external path is not a safe absolute child path: '{path}'")]
    UnsafeExternalPath { path: String },
    #[error("server joint launch does not support --dry-run")]
    ServerDryRun,
    #[error("{component} path must be absolute: '{path}'")]
    ComponentPathNotAbsolute {
        component: RuntimeComponent,
        path: String,
    },
    #[error("{component} was not found at '{path}'")]
    ComponentNotFound {
        component: RuntimeComponent,
        path: String,
    },
    #[error("Orbit Runtime Agent path contains a quote")]
    AgentPathContainsQuote,
    #[error("JAVA_TOOL_OPTIONS already contains the Orbit Runtime Agent")]
    AgentAlreadyPresent,
    #[error(
        "Orbit Runtime Agent support has not been verified for {loader} loader version '{version}'"
    )]
    UnsupportedRuntimeAgent { loader: String, version: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeComponent {
    Launcher,
    Agent,
}

impl std::fmt::Display for RuntimeComponent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Launcher => "Orbit Launcher executable",
            Self::Agent => "Orbit Runtime Agent",
        })
    }
}

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

    #[error("runtime data ownership failed: {0}")]
    RuntimeData(#[from] RuntimeDataError),

    #[error("launcher process exited with status {0}")]
    ForwardedProcessExit(i32),

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
