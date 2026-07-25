use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditProgressStage {
    PrepareInputs,
    ScanArtifacts,
    Readiness,
    AnalyzeMixins,
    AnalyzeTransformers,
    DetectConflicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditProgressEvent {
    StageStarted {
        stage: AuditProgressStage,
        total: Option<usize>,
    },
    Advanced {
        stage: AuditProgressStage,
        completed: usize,
        total: Option<usize>,
    },
    StageFinished {
        stage: AuditProgressStage,
        completed: usize,
    },
}

pub type AuditProgressReporter = Arc<dyn Fn(AuditProgressEvent) + Send + Sync>;

pub(crate) fn emit(progress: Option<&AuditProgressReporter>, event: AuditProgressEvent) {
    if let Some(progress) = progress {
        progress(event);
    }
}
