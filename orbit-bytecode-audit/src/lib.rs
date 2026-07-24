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
mod readiness;
mod transformer;

pub use error::AuditError;
pub use model::{
    Activation, AnalysisLimits, ArtifactInput, ArtifactKind, ArtifactReport, AuditEnvironment,
    AuditReport, AuditRequest, ClassReference, Confidence, Coverage, Effect, Evidence,
    InstructionReference, LoaderFamily, Mechanism, MemberKind, MemberReference, Mutation,
    MutationKind, NestedJarPolicy, OrderAnalysis, Precision, Readiness, ReadinessStatus,
    RequirementKind, Risk, Severity, ShapeRequirement, Target, Warning,
};
pub use readiness::probe_readiness;

/// Analyze the exact files supplied by the caller.
///
/// No result is persisted. Every invocation opens and parses every artifact
/// again, including artifacts whose path, timestamp, or hash is unchanged.
pub fn analyze(request: &AuditRequest) -> Result<AuditReport, AuditError> {
    let readiness = probe_readiness(request)?;
    if readiness.status != ReadinessStatus::Ready {
        return Err(AuditError::NotReady(readiness));
    }
    let mut scanned = jar::scan_artifacts(request)?;
    let mut effects = mixin::analyze(&mut scanned);
    effects.extend(transformer::analyze(&mut scanned, &readiness));
    Ok(conflict::build_report(request, readiness, scanned, effects))
}
