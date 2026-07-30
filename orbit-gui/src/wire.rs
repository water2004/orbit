use anyhow::{Context, Result, bail};
use orbit_machine_protocol::{
    ErrorEnvelope, InteractionEnvelope, ProgressEnvelope, SCHEMA_VERSION, SuccessEnvelope,
};
use serde_json::Value;

use crate::model::{AuditFinding, AuditNotice, AuditSummary};

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
    let event = data.get("event").and_then(Value::as_str);
    let mut completed = data
        .get("completed")
        .or_else(|| data.get("downloaded_bytes"))
        .and_then(Value::as_u64);
    let total = data
        .get("total")
        .or_else(|| data.get("total_bytes"))
        .and_then(Value::as_u64);
    match event {
        Some("export_started" | "ExportStarted") if completed.is_none() => {
            completed = total.map(|_| 0);
        }
        Some("export_finished" | "ExportFinished") => {
            completed = total;
        }
        _ => {}
    }
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
        "export_started" | "ExportStarted" => "Preparing export",
        "export_advanced" | "ExportAdvanced" => "Writing export",
        "export_finished" | "ExportFinished" => "Export complete",
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

pub(crate) fn humanize(value: &str) -> String {
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

pub fn audit_summary(result: &Value) -> Result<AuditSummary> {
    let readiness_value = result
        .get("readiness")
        .context("audit result is missing readiness")?;
    let readiness = required_string_field(readiness_value, "status")?;
    let readiness_message = required_string_field(readiness_value, "message")?;
    let loader = optional_string_field(readiness_value, "loader")?;
    let capability_values = match readiness_value.get("capabilities") {
        None => &[][..],
        Some(value) => value
            .as_array()
            .context("audit readiness capabilities must be an array")?
            .as_slice(),
    };
    let capabilities = capability_values
        .iter()
        .enumerate()
        .map(|(index, capability)| {
            capability
                .as_str()
                .map(str::to_string)
                .with_context(|| format!("audit capability {index} must be a string"))
        })
        .collect::<Result<Vec<_>>>()?;
    let namespace = result
        .get("namespace")
        .context("audit result is missing namespace")?;
    let runtime_namespace = optional_string_field(namespace, "runtime_namespace")?;
    let artifacts = required_array(result, "artifacts")?.len();
    let warnings = required_array(result, "warnings")?
        .iter()
        .map(|warning| {
            Ok(AuditNotice {
                artifact: optional_string_field(warning, "artifact_id")?,
                scope: required_string_field(warning, "scope")?,
                kind: required_string_field(warning, "kind")?,
                detail: required_string_field(warning, "message")?,
                count: 1,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let coverage_gaps = required_array(result, "coverage_gaps")?
        .iter()
        .map(|gap| {
            Ok(AuditNotice {
                artifact: optional_string_field(gap, "artifact_id")?,
                scope: required_string_field(gap, "scope")?,
                kind: required_string_field(gap, "kind")?,
                detail: required_string_field(gap, "detail")?,
                count: required_usize_field(gap, "count")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut findings = Vec::new();

    for risk in required_array(result, "risks")? {
        let left = required_string_field(risk, "left_artifact")?;
        let right = required_string_field(risk, "right_artifact")?;
        findings.push(finding(risk, format!("{left} ↔ {right}"))?);
    }
    for risk in required_array(result, "unary_risks")? {
        let package = required_string_field(risk, "artifact_id")?;
        findings.push(finding(risk, package)?);
    }
    findings.sort_by_key(|risk| std::cmp::Reverse(risk.risk));

    Ok(AuditSummary {
        readiness,
        readiness_message,
        loader,
        runtime_namespace,
        capabilities,
        artifacts,
        warnings,
        coverage_gaps,
        findings,
    })
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .with_context(|| format!("audit field '{field}' must be an array"))
}

fn optional_string_field(value: &Value, field: &str) -> Result<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => bail!("audit field '{field}' must be a string or null"),
    }
}

fn required_usize_field(value: &Value, field: &str) -> Result<usize> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .with_context(|| format!("audit field '{field}' must be a non-negative integer"))
}

fn finding(value: &Value, packages: String) -> Result<AuditFinding> {
    Ok(AuditFinding {
        packages,
        rule: required_string_field(value, "rule")?,
        reason: required_string_field(value, "reason")?,
        risk: value
            .get("risk_index")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .context("audit risk_index must be an integer between 0 and 255")?,
        severity: required_string_field(value, "severity")?,
        confidence: required_string_field(value, "confidence")?,
    })
}

fn required_string_field(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("audit field '{field}' must be a string"))
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
            "readiness": {
                "status": "ready",
                "loader": "neoforge",
                "message": "runtime ABI is available",
                "capabilities": ["mixin", "neoforge_class_processor"]
            },
            "namespace": {"runtime_namespace": "official"},
            "artifacts": [{}, {}],
            "warnings": [{
                "artifact_id": "a", "scope": "mixins.json",
                "kind": "duplicate_mixin_config", "message": "duplicate config"
            }],
            "coverage_gaps": [{
                "artifact_id": "b", "scope": "example/Processor",
                "kind": "transformer_unknown", "detail": "dynamic selector", "count": 3
            }],
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
        let summary = audit_summary(&report).unwrap();
        assert_eq!(summary.artifacts, 2);
        assert_eq!(summary.findings.len(), 2);
        assert_eq!(summary.findings[0].packages, "a ↔ b");
        assert_eq!(summary.loader.as_deref(), Some("neoforge"));
        assert_eq!(summary.runtime_namespace.as_deref(), Some("official"));
        assert_eq!(summary.capabilities.len(), 2);
        assert_eq!(summary.warnings[0].kind, "duplicate_mixin_config");
        assert_eq!(summary.coverage_gaps[0].count, 3);
    }

    #[test]
    fn rejects_incomplete_audit_results_instead_of_rendering_unknown_fields() {
        let report = serde_json::json!({
            "readiness": {"status": "ready", "message": "ready"},
            "namespace": {"runtime_namespace": "identity"},
            "artifacts": [],
            "warnings": [],
            "coverage_gaps": [],
            "risks": [{"left_artifact": "a"}],
            "unary_risks": []
        });

        let error = audit_summary(&report).unwrap_err();

        assert!(error.to_string().contains("right_artifact"));
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
    fn export_lifecycle_has_determinate_progress_from_start_to_finish() {
        let started = serde_json::json!({
            "event": "export_started",
            "packages": 40,
            "total_bytes": 80_000_000
        });
        let advanced = serde_json::json!({
            "event": "export_advanced",
            "completed": 20_000_000,
            "total": 80_000_000,
            "completed_packages": 10,
            "packages": 40
        });
        let finished = serde_json::json!({
            "event": "export_finished",
            "packages": 40,
            "total_bytes": 80_000_000
        });

        assert_eq!(progress_numbers(&started), (Some(0), Some(80_000_000)));
        assert_eq!(
            progress_numbers(&advanced),
            (Some(20_000_000), Some(80_000_000))
        );
        assert_eq!(
            progress_numbers(&finished),
            (Some(80_000_000), Some(80_000_000))
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
