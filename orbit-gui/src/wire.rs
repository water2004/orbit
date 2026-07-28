use anyhow::{Context, Result, bail};
use orbit_machine_protocol::{
    ErrorEnvelope, InteractionEnvelope, ProgressEnvelope, SCHEMA_VERSION, SuccessEnvelope,
};
use serde_json::Value;

use crate::model::{AuditFinding, AuditSummary};

pub fn success_document(document: &str) -> Result<SuccessEnvelope<Value>> {
    let envelope: SuccessEnvelope<Value> =
        serde_json::from_str(document).context("CLI stdout is not a success envelope")?;
    if envelope.schema_version != SCHEMA_VERSION {
        bail!(
            "machine protocol {} is unsupported; expected exactly {}",
            envelope.schema_version,
            SCHEMA_VERSION
        );
    }
    if !envelope.ok {
        bail!("success envelope has ok=false");
    }
    Ok(envelope)
}

pub fn error_line(line: &str) -> Option<Result<ErrorEnvelope>> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "error" {
        return None;
    }
    Some((|| {
        let envelope: ErrorEnvelope = serde_json::from_value(value)?;
        if envelope.schema_version != SCHEMA_VERSION {
            bail!(
                "machine protocol {} is unsupported; expected exactly {}",
                envelope.schema_version,
                SCHEMA_VERSION
            );
        }
        Ok(envelope)
    })())
}

pub fn progress_line(line: &str) -> Option<Result<ProgressEnvelope<Value>>> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "progress" {
        return None;
    }
    Some((|| {
        let envelope: ProgressEnvelope<Value> = serde_json::from_value(value)?;
        if envelope.schema_version != SCHEMA_VERSION {
            bail!(
                "machine protocol {} is unsupported; expected exactly {}",
                envelope.schema_version,
                SCHEMA_VERSION
            );
        }
        Ok(envelope)
    })())
}

pub fn interaction_line(line: &str) -> Option<Result<InteractionEnvelope<Value>>> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "interaction" {
        return None;
    }
    Some((|| {
        let envelope: InteractionEnvelope<Value> = serde_json::from_value(value)?;
        if envelope.schema_version != SCHEMA_VERSION {
            bail!(
                "machine protocol {} is unsupported; expected exactly {}",
                envelope.schema_version,
                SCHEMA_VERSION
            );
        }
        Ok(envelope)
    })())
}

pub fn progress_numbers(data: &Value) -> (Option<u64>, Option<u64>) {
    let completed = data
        .get("completed")
        .or_else(|| data.get("downloaded_bytes"))
        .and_then(Value::as_u64);
    let total = data
        .get("total")
        .or_else(|| data.get("total_bytes"))
        .and_then(Value::as_u64);
    (completed, total)
}

pub fn progress_label(data: &Value) -> String {
    let event = data
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or("working");
    let subject = data
        .get("package")
        .or_else(|| data.get("logical_name"))
        .or_else(|| data.get("runtime_id"))
        .or_else(|| data.get("loader"))
        .or_else(|| data.get("provider"))
        .and_then(Value::as_str);
    let label = progress_event_label(event);
    match subject {
        Some(subject) => format!("{} · {}", label, subject),
        None => label,
    }
}

fn progress_event_label(event: &str) -> String {
    let key = match event {
        "metadata_started" => "Resolving official metadata",
        "minecraft_resolved" => "Minecraft metadata resolved",
        "eula_checked" => "Checking Minecraft EULA",
        "artifact_started" | "artifact_bytes" => "Downloading file",
        "artifact_cached" => "Using cached file",
        "artifact_finished" => "Download complete",
        "java_manifest_started" => "Loading managed Java manifest",
        "java_runtime_resolved" => "Managed Java runtime resolved",
        "java_materialized" => "Assembling managed Java runtime files",
        "java_runtime_verified" => "Managed Java runtime verified",
        "java_runtime_cached" => "Using cached Java runtime",
        "loader_installer_started" => "Running official Loader installer",
        "loader_installer_output" => "Loader installer output",
        "loader_installer_output_suppressed" => "Loader installer output truncated",
        "loader_installer_finished" => "Loader installation complete",
        "staging_verified" => "Verifying staged runtime",
        "committed" => "Committing instance runtime",
        "microsoft_authorization_polling" => "Waiting for Microsoft authorization",
        "microsoft_authorization_received" => "Microsoft authorization received",
        "xbox_authenticated" => "Xbox services authenticated",
        "minecraft_authenticated" => "Minecraft account authenticated",
        "account_session_stored" => "Account session stored",
        "launch_artifact_verified" => "Verifying launch files",
        "launch_java_verified" => "Verifying launch Java runtime",
        "launch_natives_prepared" => "Preparing native libraries",
        "launch_plan_ready" => "Launch plan ready",
        "repository_copying" => "Moving Minecraft repository",
        "repository_verifying" => "Verifying moved repository",
        "repository_switching" => "Switching Minecraft repository",
        "repository_removing_source" => "Removing old repository files",
        "process_spawned" => "Game process started",
        "process_output" => "Game process output",
        "process_exited" => "Game process exited",
        "supervisor_spawned" => "Server supervisor started",
        "supervisor_command_sent" => "Sending supervisor command",
        "supervisor_stop_requested" => "Stopping supervised server",
        "supervisor_exited" => "Supervised server exited",
        "supervisor_backoff" => "Waiting before server restart",
        "supervisor_restarting" => "Restarting server",
        "supervisor_restart_limit_reached" => "Server restart limit reached",
        "supervisor_stopped" => "Server supervisor stopped",
        _ => return orbit_i18n::text(&humanize(event)).into_owned(),
    };
    orbit_i18n::text(key).into_owned()
}

fn humanize(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_lowercase = false;
    for character in value.chars() {
        if character == '_' {
            output.push(' ');
            previous_lowercase = false;
        } else if character.is_uppercase() && previous_lowercase {
            output.push(' ');
            output.push(character.to_ascii_lowercase());
            previous_lowercase = false;
        } else {
            output.push(character.to_ascii_lowercase());
            previous_lowercase = character.is_lowercase();
        }
    }
    if let Some(first) = output.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    output
}

pub fn audit_summary(result: &Value) -> AuditSummary {
    let readiness = result
        .pointer("/readiness/status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let artifacts = result
        .get("artifacts")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let warnings = result
        .get("warnings")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let coverage_gaps = result
        .get("coverage_gaps")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut findings = Vec::new();

    for risk in result
        .get("risks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let left = risk
            .get("left_artifact")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let right = risk
            .get("right_artifact")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        findings.push(finding(risk, format!("{left} ↔ {right}")));
    }
    for risk in result
        .get("unary_risks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let package = risk
            .get("artifact_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        findings.push(finding(risk, package.to_string()));
    }
    findings.sort_by_key(|risk| std::cmp::Reverse(risk.risk));

    AuditSummary {
        readiness,
        artifacts,
        warnings,
        coverage_gaps,
        findings,
    }
}

fn finding(value: &Value, packages: String) -> AuditFinding {
    AuditFinding {
        packages,
        rule: string_field(value, "rule"),
        reason: string_field(value, "reason"),
        risk: value
            .get("risk_index")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or_default(),
        severity: string_field(value, "severity"),
        confidence: string_field(value, "confidence"),
    }
}

fn string_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_legacy_envelopes_instead_of_falling_back() {
        let error =
            success_document(r#"{"schema_version":1,"command":"search","ok":true,"result":{}}"#)
                .unwrap_err();
        assert!(error.to_string().contains("expected exactly 2"));
    }

    #[test]
    fn reads_both_pairwise_and_unary_audit_findings() {
        let report = serde_json::json!({
            "readiness": {"status": "ready"},
            "artifacts": [{}, {}],
            "warnings": [{}],
            "coverage_gaps": [],
            "risks": [{
                "left_artifact": "a", "right_artifact": "b", "rule": "overlap",
                "reason": "same target", "risk_index": 80, "severity": "high",
                "confidence": "high"
            }],
            "unary_risks": [{
                "artifact_id": "c", "rule": "runtime", "reason": "newer java",
                "risk_index": 40, "severity": "medium", "confidence": "medium"
            }]
        });
        let summary = audit_summary(&report);
        assert_eq!(summary.artifacts, 2);
        assert_eq!(summary.findings.len(), 2);
        assert_eq!(summary.findings[0].packages, "a ↔ b");
    }

    #[test]
    fn parses_progress_error_and_interaction_from_the_single_stderr_stream() {
        let progress = progress_line(
            r#"{"schema_version":2,"type":"progress","command":"install","sequence":7,"phase":"download","data":{"event":"artifact_downloaded","logical_name":"Minecraft client","completed":3,"total":8}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(progress.sequence, 7);
        assert_eq!(progress_numbers(&progress.data), (Some(3), Some(8)));
        assert_eq!(
            progress_label(&progress.data),
            "Artifact downloaded · Minecraft client"
        );

        let interaction = interaction_line(
            r#"{"schema_version":2,"type":"interaction","command":"upgrade","sequence":8,"interaction_id":"resolution-8","interaction":"resolution","prompt":"Choose a solution","choices":[{"id":"1","label":"Option 1","data":{"changes":[]}}],"default_choice":"1","allow_cancel":true}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(interaction.interaction_id, "resolution-8");
        assert_eq!(interaction.choices[0].id, "1");

        let error = error_line(
            r#"{"schema_version":2,"type":"error","command":"install","ok":false,"code":"network","message":"offline"}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(error.code, "network");
    }

    #[test]
    fn java_materialization_is_presented_as_file_assembly_not_archive_extraction() {
        let progress = serde_json::json!({
            "event": "java_materialized",
            "completed": 12,
            "total": 24
        });
        assert_eq!(
            progress_label(&progress),
            orbit_i18n::text("Assembling managed Java runtime files")
        );
    }

    #[test]
    fn rejects_a_legacy_stderr_message_instead_of_treating_it_as_a_log() {
        let error = progress_line(
            r#"{"schema_version":1,"type":"progress","command":"install","sequence":1,"phase":"download","data":{}}"#,
        )
        .unwrap()
        .unwrap_err();
        assert!(error.to_string().contains("expected exactly 2"));
    }
}
