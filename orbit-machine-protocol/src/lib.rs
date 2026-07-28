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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    Package,
    Resolution,
    Confirmation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionChoice<T> {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub data: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionEnvelope<T> {
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
    pub sequence: u64,
    pub interaction_id: String,
    pub interaction: InteractionKind,
    pub prompt: String,
    pub choices: Vec<InteractionChoice<T>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_choice: Option<String>,
    pub allow_cancel: bool,
}

impl<T> InteractionEnvelope<T> {
    pub fn new(
        command: impl Into<String>,
        sequence: u64,
        interaction_id: impl Into<String>,
        interaction: InteractionKind,
        prompt: impl Into<String>,
        choices: Vec<InteractionChoice<T>>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: "interaction".to_string(),
            command: command.into(),
            sequence,
            interaction_id: interaction_id.into(),
            interaction,
            prompt: prompt.into(),
            choices,
            default_choice: None,
            allow_cancel: true,
        }
    }

    pub fn with_default(mut self, choice: impl Into<String>) -> Self {
        self.default_choice = Some(choice.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResponse {
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub interaction_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_choice: Option<String>,
    pub cancelled: bool,
}

impl InteractionResponse {
    pub fn selected(interaction_id: impl Into<String>, choice: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: "interaction_response".to_string(),
            interaction_id: interaction_id.into(),
            selected_choice: Some(choice.into()),
            cancelled: false,
        }
    }

    pub fn cancelled(interaction_id: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: "interaction_response".to_string(),
            interaction_id: interaction_id.into(),
            selected_choice: None,
            cancelled: true,
        }
    }
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

    #[test]
    fn interaction_roundtrip_uses_the_same_schema_without_a_second_endpoint() {
        let envelope = InteractionEnvelope::new(
            "upgrade",
            3,
            "resolution-3",
            InteractionKind::Resolution,
            "Choose a solution",
            vec![InteractionChoice {
                id: "1".to_string(),
                label: "Option 1".to_string(),
                description: None,
                data: serde_json::json!({"changes": []}),
            }],
        )
        .with_default("1");
        let encoded = serde_json::to_string(&envelope).unwrap();
        let decoded: InteractionEnvelope<serde_json::Value> =
            serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.schema_version, SCHEMA_VERSION);
        assert_eq!(decoded.kind, "interaction");
        assert_eq!(decoded.interaction, InteractionKind::Resolution);

        let response = InteractionResponse::selected(decoded.interaction_id, "1");
        assert_eq!(response.kind, "interaction_response");
        assert!(!response.cancelled);
    }
}
