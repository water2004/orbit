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
mod mixin_config;
mod model;
mod namespace;
mod progress;
mod readiness;
mod transformer;

pub use error::AuditError;
pub use model::{
    AccessDelta, Activation, AnalysisLimits, ArtifactInput, ArtifactKind, ArtifactReport,
    ArtifactSymbolSpace, AuditEnvironment, AuditReport, AuditRequest, BehavioralInteraction,
    BehavioralInteractionKind, ClassDefinitionId, ClassReference, ClassVisibility,
    CompositionSemantics, Confidence, ConfigActivation, Coverage, CoverageGap, CoverageGapKind,
    CoverageRatio, Effect, Evidence, FramePosition, GlobPattern, InactiveCandidate,
    InactiveCandidateKind, InjectionGroupConstraint, InjectionQuery, InstructionIdentity,
    InstructionReference, LoaderArtifactUnit, LoaderFamily, LocalSelector, MappingSource,
    Mechanism, MemberKind, MemberReference, MethodContributionKind, MethodSelector,
    MixinActivation, Mutation, MutationKind, NamespaceAlignment, NamespaceEvidence,
    NamespaceReport, NestedJarPolicy, OrderAnalysis, ParsedMixinConfig, PhysicalSide,
    PluginDecision, Precision, Readiness, ReadinessStatus, RegisteredMixin, RegisteredMixinConfig,
    RegistrationSource, RequirementKind, Risk, Severity, ShapeRequirement, SideConstraint,
    SoftReferenceResolution, SymbolMappingEvidence, SymbolNamespace, Target,
    UnaryCompatibilityRisk, Warning, WarningKind,
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

    let loader = readiness::preflight(request).map_err(AuditError::NotReady)?;
    let mut scanned = jar::scan_artifacts_with_progress(request, progress)?;
    emit(
        progress,
        AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::Readiness,
            total: Some(1),
        },
    );
    let readiness = jar::probe_runtime_abi(&scanned, loader);
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
    let namespace =
        namespace::align_runtime_namespace(&mut scanned, loader).map_err(AuditError::NotReady)?;
    let mut registry = mixin_config::discover(&mut scanned, request);
    let mixin_analysis = mixin::analyze_with_progress(&mut scanned, &registry, progress);
    registry.coverage_gaps.extend(mixin_analysis.coverage_gaps);
    registry
        .inactive_candidates
        .extend(mixin_analysis.inactive_candidates);
    let precomputed_risks = mixin_analysis.risks;
    let unary_risks = mixin_analysis.unary_risks;
    let interactions = mixin_analysis.interactions;
    let mut effects = mixin_analysis.effects;
    let transformer_analysis =
        transformer::analyze_with_progress(&mut scanned, &readiness, progress);
    effects.extend(transformer_analysis.effects);
    registry
        .inactive_candidates
        .extend(transformer_analysis.inactive_candidates);
    registry
        .coverage_gaps
        .extend(transformer_analysis.coverage_gaps);
    Ok(conflict::build_report_with_progress(
        request,
        readiness,
        namespace,
        scanned,
        conflict::RecoveredFindings {
            effects,
            registry,
            risks: precomputed_risks,
            unary_risks,
            interactions,
        },
        progress,
    ))
}
