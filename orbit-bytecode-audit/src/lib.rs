//! Static bytecode compatibility-risk analysis.
//!
//! This crate deliberately knows nothing about Orbit manifests, lockfiles,
//! providers, launchers, or terminal output. Callers provide the exact
//! artifacts that make up one runtime and receive a structured report.

mod classfile;
mod conflict;
mod error;
mod jar;
mod mixin;
mod model;
mod progress;
mod readiness;
mod transformer;

pub use error::AuditError;
pub use model::{
    AccessDelta, Activation, AnalysisLimits, ArtifactInput, ArtifactKind, ArtifactReport,
    AuditEnvironment, AuditReport, AuditRequest, ClassReference, CompositionSemantics, Confidence,
    Coverage, Effect, Evidence, InjectionGroupConstraint, InjectionQuery, InstructionReference,
    LoaderFamily, Mechanism, MemberKind, MemberReference, Mutation, MutationKind, NestedJarPolicy,
    OrderAnalysis, Precision, Readiness, ReadinessStatus, RequirementKind, Risk, Severity,
    ShapeRequirement, SoftReferenceResolution, Target, Warning, WarningKind,
};
pub use progress::{AuditProgressEvent, AuditProgressReporter, AuditProgressStage};
pub use readiness::probe_readiness;

/// Analyze the exact files supplied by the caller.
///
/// No result is persisted. Every invocation opens and parses every artifact
/// again, including artifacts whose path, timestamp, or hash is unchanged.
pub fn analyze(request: &AuditRequest) -> Result<AuditReport, AuditError> {
    analyze_with_progress(request, None)
}

/// Analyze the supplied files while emitting truthful stage and work counts.
pub fn analyze_with_progress(
    request: &AuditRequest,
    progress: Option<&AuditProgressReporter>,
) -> Result<AuditReport, AuditError> {
    use progress::{AuditProgressEvent, AuditProgressStage, emit};

    emit(
        progress,
        AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::Readiness,
            total: Some(1),
        },
    );
    let readiness = probe_readiness(request)?;
    if readiness.status != ReadinessStatus::Ready {
        return Err(AuditError::NotReady(readiness));
    }
    emit(
        progress,
        AuditProgressEvent::StageFinished {
            stage: AuditProgressStage::Readiness,
            completed: 1,
        },
    );
    let mut scanned = jar::scan_artifacts_with_progress(request, progress)?;
    let mut effects = mixin::analyze_with_progress(&mut scanned, progress);
    effects.extend(transformer::analyze_with_progress(
        &mut scanned,
        &readiness,
        progress,
    ));
    Ok(conflict::build_report_with_progress(
        request, readiness, scanned, effects, progress,
    ))
}
