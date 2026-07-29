//! NDJSON progress protocol.
//!
//! When `--progress-format ndjson` is in effect, every progress event is
//! serialized as one JSON object per line on stderr, wrapped in a
//! `{"type":"progress","phase":...,"event":...,"data":...}` envelope. The
//! final command result still goes to stdout as a single JSON document, so
//! callers can `orbit --output-format json ... | jq` while optionally tailing stderr
//! for progress.
//!
//! Content hashes, physical JAR filenames, and provider secrets never appear
//! in progress output: the core `ProgressEvent`/`AuditProgressEvent` types
//! already skip the `filename` field during serialization, and no extra
//! secrets cross this boundary.

use std::io::Write;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use orbit_core::{AuditProgressEvent, ProgressEvent};
use orbit_machine_protocol::{ProgressEnvelope, ProgressPhase};
use serde::Serialize;

/// Phase of a package operation, used as the `phase` field of the NDJSON
/// envelope. Derived from the event variant rather than stored on the core
/// type so core stays free of presentation concerns.
/// Inner envelope carrying the original event. The core event itself
/// serializes with `#[serde(tag = "event")]`, so the resulting line is
/// `{"type":"progress","phase":"...","data":{"event":"...","...":...}}`.
#[derive(Debug, Serialize)]
struct ProgressData<'a, T: Serialize> {
    #[serde(flatten)]
    event: &'a T,
}

/// Thread-safe stderr writer. `stderr` is line-buffered by default; we wrap a
/// mutex so concurrent candidate-download tasks cannot interleave half-lines.
type StderrWriter = Mutex<std::io::Stderr>;

static STDERR_WRITER: OnceLock<StderrWriter> = OnceLock::new();

pub(crate) fn write_machine_line(line: &str) {
    let writer = STDERR_WRITER.get_or_init(|| Mutex::new(std::io::stderr()));
    if let Ok(mut handle) = writer.lock() {
        let _ = writeln!(handle, "{line}");
    }
}

/// Reporter for package-operation progress (`ProgressEvent`).
pub struct NdjsonProgressReporter {
    command: &'static str,
    sequence: Arc<AtomicU64>,
}

impl NdjsonProgressReporter {
    pub fn new(command: &'static str, sequence: Arc<AtomicU64>) -> Self {
        Self { command, sequence }
    }

    pub fn reporter(self) -> orbit_core::ProgressReporter {
        let command = self.command;
        let sequence = self.sequence;
        std::sync::Arc::new(move |event: ProgressEvent| {
            let phase = phase_for(&event);
            let envelope = ProgressEnvelope::new(
                command,
                sequence.fetch_add(1, Ordering::Relaxed) + 1,
                phase,
                ProgressData { event: &event },
            );
            let line = serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".into());
            write_machine_line(&line);
        })
    }
}

/// Reporter for audit progress (`AuditProgressEvent`).
pub struct NdjsonAuditReporter {
    command: &'static str,
    sequence: Arc<AtomicU64>,
}

impl NdjsonAuditReporter {
    pub fn new(command: &'static str, sequence: Arc<AtomicU64>) -> Self {
        Self { command, sequence }
    }

    pub fn reporter(self) -> orbit_core::AuditProgressReporter {
        let command = self.command;
        let sequence = self.sequence;
        std::sync::Arc::new(move |event: AuditProgressEvent| {
            let envelope = ProgressEnvelope::new(
                command,
                sequence.fetch_add(1, Ordering::Relaxed) + 1,
                ProgressPhase::Audit,
                ProgressData { event: &event },
            );
            let line = serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".into());
            write_machine_line(&line);
        })
    }
}

/// Convenience constructors matching the existing `progress::reporter` shape.
pub fn ndjson_progress_reporter(
    command: &'static str,
    sequence: Arc<AtomicU64>,
) -> orbit_core::ProgressReporter {
    NdjsonProgressReporter::new(command, sequence).reporter()
}

pub fn ndjson_audit_reporter(
    command: &'static str,
    sequence: Arc<AtomicU64>,
) -> orbit_core::AuditProgressReporter {
    NdjsonAuditReporter::new(command, sequence).reporter()
}

fn phase_for(event: &ProgressEvent) -> ProgressPhase {
    match event {
        ProgressEvent::DiscoveryStarted
        | ProgressEvent::DiscoveringProject { .. }
        | ProgressEvent::DiscoveryFinished { .. } => ProgressPhase::Discovery,
        ProgressEvent::CandidateDownloadStarted { .. }
        | ProgressEvent::CandidateArtifact { .. }
        | ProgressEvent::CandidateDownloadFinished { .. } => ProgressPhase::Download,
        ProgressEvent::ResolutionStarted { .. }
        | ProgressEvent::ResolutionWorkStarted { .. }
        | ProgressEvent::ResolutionWorkFinished { .. }
        | ProgressEvent::ResolutionActivity { .. }
        | ProgressEvent::ResolutionFinished { .. } => ProgressPhase::Resolution,
        ProgressEvent::ApplyStarted { .. }
        | ProgressEvent::ApplyArtifact { .. }
        | ProgressEvent::ApplyFinished { .. } => ProgressPhase::Apply,
        ProgressEvent::ExportStarted { .. }
        | ProgressEvent::ExportAdvanced { .. }
        | ProgressEvent::ExportFinished { .. } => ProgressPhase::Export,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{ArtifactProgressState, ResolutionActivity, ResolutionWork};

    #[test]
    fn progress_event_serializes_with_event_tag_and_skipped_filename() {
        let event = ProgressEvent::CandidateArtifact {
            completed: 1,
            total: 2,
            filename: "secret-filename.jar".to_string(),
            state: ArtifactProgressState::Finished,
        };
        let envelope = ProgressEnvelope::new(
            "install",
            1,
            ProgressPhase::Download,
            ProgressData { event: &event },
        );
        let json = serde_json::to_string(&envelope).unwrap();

        assert!(json.contains("\"type\":\"progress\""));
        assert!(json.contains("\"schema_version\":2"));
        assert!(json.contains("\"command\":\"install\""));
        assert!(json.contains("\"sequence\":1"));
        assert!(json.contains("\"phase\":\"download\""));
        assert!(json.contains("\"event\":\"CandidateArtifact\""));
        assert!(json.contains("\"state\":\"finished\""));
        assert!(json.contains("\"completed\":1"));
        // filename is skipped per spec.
        assert!(!json.contains("secret-filename"));
        assert!(!json.contains(".jar"));
    }

    #[test]
    fn resolution_work_uses_kind_tag() {
        let work = ResolutionWork::MaximalityProbe {
            package: "sodium".to_string(),
        };
        let json = serde_json::to_string(&work).unwrap();
        assert!(json.contains("\"kind\":\"maximality_probe\""));
        assert!(json.contains("\"package\":\"sodium\""));

        let preference = ResolutionWork::PreferenceProbe {
            package: "iris".to_string(),
        };
        let json = serde_json::to_string(&preference).unwrap();
        assert!(json.contains("\"kind\":\"preference_probe\""));
        assert!(json.contains("\"package\":\"iris\""));
    }

    #[test]
    fn resolution_activity_conflict_has_no_payload_fields() {
        let activity = ResolutionActivity::Conflict;
        let json = serde_json::to_string(&activity).unwrap();
        assert_eq!(json, "{\"kind\":\"conflict\"}");
    }

    #[test]
    fn audit_event_serializes_with_snake_case_stage() {
        let event = AuditProgressEvent::StageStarted {
            stage: orbit_core::AuditProgressStage::ScanArtifacts,
            total: Some(10),
        };
        let envelope = ProgressEnvelope::new(
            "audit",
            1,
            ProgressPhase::Audit,
            ProgressData { event: &event },
        );
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("\"phase\":\"audit\""));
        assert!(json.contains("\"stage\":\"scan_artifacts\""));
        assert!(json.contains("\"event\":\"StageStarted\""));
    }
}
