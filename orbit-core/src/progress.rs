//! Structured progress events for long-running package operations.
//!
//! Core never renders these events. Frontends may attach a reporter and choose
//! how to display them; callers that do not need progress simply pass `None`.

use std::sync::Arc;

use serde::Serialize;

/// Thread-safe observer used by concurrent candidate download tasks.
pub type ProgressReporter = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactProgressState {
    Started,
    Finished,
    AlreadyPresent,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolutionWork {
    EnumerationRun { run: usize },
    MaximalityProbe { package: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolutionActivity {
    Decision { package: String },
    Propagation { package: String },
    Backtrack { from_level: u32, to_level: u32 },
    Conflict,
    Solution,
}

/// Solver/package-operation progress event.
///
/// Serialized form uses `#[serde(tag = "event")]` so each variant renders as
/// `{"event": "VariantName", ...fields}`. CLI NDJSON wrappers add the outer
/// `type`/`phase` envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "PascalCase")]
pub enum ProgressEvent {
    DiscoveryStarted,
    DiscoveringProject {
        provider: String,
        locator: String,
        pending_projects: usize,
        artifacts_found: usize,
    },
    DiscoveryFinished {
        projects: usize,
        artifacts: usize,
    },
    CandidateDownloadStarted {
        total: usize,
    },
    CandidateArtifact {
        completed: usize,
        total: usize,
        #[serde(skip)]
        filename: String,
        state: ArtifactProgressState,
    },
    CandidateDownloadFinished {
        total: usize,
    },
    ResolutionStarted {
        packages: usize,
        candidates: usize,
    },
    ResolutionWorkStarted {
        work: ResolutionWork,
    },
    ResolutionWorkFinished {
        work: ResolutionWork,
    },
    ResolutionActivity {
        activity: ResolutionActivity,
    },
    ResolutionFinished {
        solutions: usize,
    },
    ApplyStarted {
        total: usize,
    },
    ApplyArtifact {
        completed: usize,
        total: usize,
        #[serde(skip)]
        filename: String,
        state: ArtifactProgressState,
    },
    ApplyFinished {
        total: usize,
    },
    ExportStarted {
        packages: usize,
        total_bytes: u64,
    },
    ExportAdvanced {
        completed: u64,
        total: u64,
        completed_packages: usize,
        packages: usize,
    },
    ExportFinished {
        packages: usize,
        total_bytes: u64,
    },
}

pub(crate) fn emit(progress: Option<&ProgressReporter>, event: ProgressEvent) {
    if let Some(progress) = progress {
        progress(event);
    }
}
