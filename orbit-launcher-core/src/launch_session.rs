//! One launch generation's output and failure evidence, shared by clients and servers.
use crate::{
    LauncherError,
    instance::InstanceKind,
    launch::{LaunchOutputStream, LaunchResult},
    launch_diagnostics::DiagnosticSnapshot,
    launch_record::Recording,
};
use std::{path::Path, process::ExitStatus, time::Instant};

pub(crate) struct LaunchSession {
    snapshot: DiagnosticSnapshot,
    recording: Recording,
    started: Instant,
    output_error: Option<LauncherError>,
}
impl LaunchSession {
    pub(crate) fn new(directory: &Path) -> Result<Self, LauncherError> {
        Ok(Self {
            snapshot: DiagnosticSnapshot::capture(directory),
            recording: Recording::new(directory)?,
            started: Instant::now(),
            output_error: None,
        })
    }
    pub(crate) fn started(&mut self, pid: u32) -> Result<(), LauncherError> {
        self.recording.started(pid)
    }
    pub(crate) fn output(
        &mut self,
        stream: LaunchOutputStream,
        mut line: String,
        secrets: &[String],
    ) -> String {
        for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
            line = line.replace(secret, "<redacted>");
        }
        if let Err(error) = self.recording.line(stream, &line) {
            self.output_error.get_or_insert(error);
        }
        line
    }
    pub(crate) fn output_error(&mut self, error: LauncherError) {
        self.output_error.get_or_insert(error);
    }
    pub(crate) fn finish(
        mut self,
        directory: &Path,
        instance_id: uuid::Uuid,
        kind: InstanceKind,
        pid: u32,
        status: ExitStatus,
        secrets: &[String],
    ) -> Result<LaunchResult, LauncherError> {
        let mut diagnostics =
            (!status.success()).then(|| self.snapshot.failure(directory, pid, secrets));
        if let Some(diagnostics) = &mut diagnostics {
            diagnostics.fallback_log(&self.recording.record.stderr_log, secrets);
            diagnostics.fallback_log(&self.recording.record.stdout_log, secrets);
        }
        if let Some(error) = self.output_error {
            if let Some(diagnostics) = &mut diagnostics {
                diagnostics.warnings.push(error.to_string());
            } else {
                return Err(error);
            }
        }
        let mut result = LaunchResult {
            instance_id,
            kind,
            pid,
            exit_code: status.code(),
            success: status.success(),
            elapsed_milliseconds: self.started.elapsed().as_millis(),
            diagnostics,
        };
        if let Err(error) = self.recording.finish(result.clone()) {
            // Never replace a useful crash report with a secondary recording error.
            if let Some(diagnostics) = &mut result.diagnostics {
                diagnostics.warnings.push(error.to_string());
            } else {
                return Err(error);
            }
        }
        Ok(result)
    }
}
