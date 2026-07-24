use crate::model::Readiness;

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("bytecode audit is not ready: {message}", message = .0.message)]
    NotReady(Readiness),

    #[error("invalid audit request: {0}")]
    InvalidRequest(String),

    #[error("failed to read artifact '{path}': {source}")]
    ReadArtifact {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
