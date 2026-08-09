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
pub enum ResolutionCurrent {
    Enumeration { run: usize },
    VersionMaximization { package: String },
    PreferencePreservation { package: String },
    Decision { package: String },
}

/// Solver/package-operation progress event.
///
/// Serialized form uses `#[serde(tag = "event")]` so each variant renders as
/// `{"event": "VariantName", ...fields}`. CLI NDJSON wrappers add the outer
/// `type`/`phase` envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "PascalCase")]
pub enum ProgressEvent {
    RepositoryIndexStarted {
        minecraft: String,
        loader: String,
        total: usize,
    },
    RepositoryProjectChecked {
        completed: usize,
        total: usize,
        provider: String,
        project_id: String,
        refreshed: bool,
        artifacts: usize,
    },
    RepositoryIndexFinished {
        completed: usize,
        total: usize,
        refreshed: usize,
        reused: usize,
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
    ResolutionAdvanced {
        work_discovered: u64,
        work_completed: u64,
        decisions: u64,
        propagations: u64,
        backtracks: u64,
        conflicts: u64,
        solutions: usize,
        current: Option<ResolutionCurrent>,
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
    ImportStarted {
        files: usize,
        total_bytes: u64,
    },
    ImportAdvanced {
        completed_bytes: u64,
        total_bytes: u64,
        completed_files: usize,
        files: usize,
    },
    ImportFinished {
        files: usize,
        total_bytes: u64,
    },
}

pub(crate) fn emit(progress: Option<&ProgressReporter>, event: ProgressEvent) {
    if let Some(progress) = progress {
        progress(event);
    }
}
