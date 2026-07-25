//! Structured progress events for long-running package operations.
//!
//! Core never renders these events. Frontends may attach a reporter and choose
//! how to display them; callers that do not need progress simply pass `None`.

use std::sync::Arc;

/// Thread-safe observer used by concurrent candidate download tasks.
pub type ProgressReporter = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactProgressState {
    Started,
    Finished,
    AlreadyPresent,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionWork {
    EnumerationRun { run: usize },
    MaximalityProbe { package: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionActivity {
    Decision { package: String },
    Propagation { package: String },
    Backtrack { from_level: u32, to_level: u32 },
    Conflict,
    Solution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
        filename: String,
        state: ArtifactProgressState,
    },
    ApplyFinished {
        total: usize,
    },
}

pub(crate) fn emit(progress: Option<&ProgressReporter>, event: ProgressEvent) {
    if let Some(progress) = progress {
        progress(event);
    }
}
