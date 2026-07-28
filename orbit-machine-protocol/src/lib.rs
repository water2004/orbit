//! Canonical machine protocol shared by `orbit`, `orbit-launcher`, and native
//! process clients.
//!
//! This crate describes the existing `--format json` / `--progress-format
//! ndjson` wire path. It is not a second API: both command-line programs emit
//! these envelopes directly and clients accept no legacy envelope.

use serde::{Deserialize, Serialize};

/// Breaking wire revision shared by success, error, and progress messages.
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessEnvelope<T> {
    pub schema_version: u32,
    pub command: String,
    pub ok: bool,
    pub result: T,
}

impl<T> SuccessEnvelope<T> {
    pub fn new(command: impl Into<String>, result: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command: command.into(),
            ok: true,
            result,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
    pub ok: bool,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl ErrorEnvelope {
    pub fn new(
        command: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: "error".to_string(),
            command: command.into(),
            ok: false,
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    Discovery,
    Download,
    Resolution,
    Apply,
    Audit,
    Metadata,
    Eula,
    Java,
    Loader,
    Authentication,
    Launch,
    Process,
    Supervisor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEnvelope<T> {
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
    pub sequence: u64,
    pub phase: ProgressPhase,
    pub data: T,
}

impl<T> ProgressEnvelope<T> {
    pub fn new(command: impl Into<String>, sequence: u64, phase: ProgressPhase, data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: "progress".to_string(),
            command: command.into(),
            sequence,
            phase,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_envelope_uses_one_breaking_schema_revision() {
        let success = SuccessEnvelope::new("search", serde_json::json!({}));
        let error = ErrorEnvelope::new("search", "network", "offline");
        let progress = ProgressEnvelope::new(
            "search",
            1,
            ProgressPhase::Discovery,
            serde_json::json!({ "event": "started" }),
        );

        assert_eq!(success.schema_version, SCHEMA_VERSION);
        assert_eq!(error.schema_version, SCHEMA_VERSION);
        assert_eq!(progress.schema_version, SCHEMA_VERSION);
        assert!(success.ok);
        assert!(!error.ok);
        assert_eq!(progress.sequence, 1);
    }
}
