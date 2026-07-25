use std::collections::{BTreeMap, HashMap};

use crate::jar::ScannedArtifacts;
use crate::mixin_config::MixinRegistry;
use crate::model::{
    Activation, AuditReport, AuditRequest, BehavioralInteraction, BehavioralInteractionKind,
    CompositionSemantics, Confidence, CoverageGap, CoverageGapKind, Effect, InjectionQuery,
    Mutation, MutationKind, OrderAnalysis, Precision, REPORT_SCHEMA_VERSION, Readiness,
    RequirementKind, Risk, Severity, Target, WarningKind,
};

pub(crate) struct RecoveredFindings {
    pub effects: Vec<Effect>,
    pub registry: MixinRegistry,
    pub risks: Vec<Risk>,
    pub interactions: Vec<BehavioralInteraction>,
}

pub(crate) fn build_report_with_progress(
    request: &AuditRequest,
    readiness: Readiness,
    scanned: ScannedArtifacts,
    findings: RecoveredFindings,
    progress: Option<&crate::progress::AuditProgressReporter>,
) -> AuditReport {
    use crate::progress::{AuditProgressEvent, AuditProgressStage, emit};

    let RecoveredFindings {
        effects,
        registry,
        risks: mut precomputed_risks,
        mut interactions,
    } = findings;
    emit(
        progress,
        AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::DetectConflicts,
            total: Some(2),
        },
    );
    precomputed_risks.extend(binary_shape_risks(&scanned));
    let mut risks = precomputed_risks;
    emit(
        progress,
        AuditProgressEvent::Advanced {
            stage: AuditProgressStage::DetectConflicts,
            completed: 1,
            total: Some(2),
        },
    );
    let mut coverage = scanned.coverage;
    for effect in &effects {
        match effect.precision {
            Precision::Instruction | Precision::Pattern => {
                coverage.effects_instruction_precision += 1;
            }
            Precision::Method => coverage.effects_method_precision += 1,
            Precision::Class | Precision::Unknown => coverage.effects_class_precision += 1,
        }
    }
    coverage.instruction_resolution_degraded = effects
        .iter()
        .filter(|effect| {
            !matches!(
                effect.precision,
                Precision::Instruction | Precision::Pattern
            ) && effect.evidence.iter().any(|evidence| {
                evidence
                    .injector_kind
                    .as_deref()
                    .is_some_and(|kind| kind != "WrapMethod")
            })
        })
        .count();
    interactions.extend(behavioral_interactions(&effects));
    interactions = coalesce_interactions(interactions);
    risks.extend(analyze_effects(&effects));
    emit(
        progress,
        AuditProgressEvent::Advanced {
            stage: AuditProgressStage::DetectConflicts,
            completed: 2,
            total: Some(2),
        },
    );
    risks = coalesce_risks(risks);
    risks.sort_by(|left, right| {
        right
            .risk_index
            .cmp(&left.risk_index)
            .then_with(|| left.left_artifact.cmp(&right.left_artifact))
            .then_with(|| left.right_artifact.cmp(&right.right_artifact))
            .then_with(|| left.rule.cmp(&right.rule))
    });
    let registered_mixin_configs = registry.configs;
    let registered_mixins = registry.mixins;
    let inactive_candidates = registry.inactive_candidates;
    let mut coverage_gaps = registry.coverage_gaps;
    let mut warnings = Vec::new();
    for warning in scanned.warnings {
        let gap_kind = match warning.kind {
            WarningKind::KnownUnsupportedInjectionPoint | WarningKind::CustomInjectionPoint => {
                Some(CoverageGapKind::UnsupportedInjectionPoint)
            }
            WarningKind::TransformerPartial => Some(CoverageGapKind::TransformerPartial),
            WarningKind::BudgetExhaustion => Some(CoverageGapKind::BudgetExhaustion),
            WarningKind::UnsupportedMechanism => Some(CoverageGapKind::UnsupportedMechanism),
            _ => None,
        };
        if let Some(kind) = gap_kind {
            coverage_gaps.push(CoverageGap {
                artifact_id: warning.artifact_id,
                scope: warning.scope,
                kind,
                detail: warning.message,
                count: 1,
            });
        } else {
            warnings.push(warning);
        }
    }
    if coverage.future_classfiles > 0 {
        coverage_gaps.push(CoverageGap {
            artifact_id: None,
            scope: "class universe".to_string(),
            kind: CoverageGapKind::FutureClassfile,
            detail: "newer ClassFiles were parsed using the best-effort compatibility path"
                .to_string(),
            count: coverage.future_classfiles,
        });
    }
    if coverage.transformer_effects_unknown > 0 {
        coverage_gaps.push(CoverageGap {
            artifact_id: None,
            scope: "ModLauncher transformers".to_string(),
            kind: CoverageGapKind::TransformerUnknown,
            detail: "registered transformer targets or mutations could not be recovered"
                .to_string(),
            count: coverage.transformer_effects_unknown,
        });
    }
    let report = AuditReport {
        schema_version: REPORT_SCHEMA_VERSION,
        environment: request.environment.clone(),
        readiness,
        artifacts: scanned.artifact_reports,
        registered_mixin_configs,
        registered_mixins,
        transformations: effects,
        risks,
        interactions,
        inactive_candidates,
        coverage_gaps,
        coverage,
        warnings,
    };
    emit(
        progress,
        AuditProgressEvent::StageFinished {
            stage: AuditProgressStage::DetectConflicts,
            completed: report.risks.len(),
        },
    );
    report
}

fn coalesce_interactions(interactions: Vec<BehavioralInteraction>) -> Vec<BehavioralInteraction> {
    let mut grouped = BTreeMap::<String, BehavioralInteraction>::new();
    for interaction in interactions {
        let (left, right) = if interaction.left_artifact <= interaction.right_artifact {
            (
                interaction.left_artifact.clone(),
                interaction.right_artifact.clone(),
            )
        } else {
            (
                interaction.right_artifact.clone(),
                interaction.left_artifact.clone(),
            )
        };
        let key = format!(
            "{left}|{right}|{}|{:?}|{}|{:?}|{:?}|{:?}",
            target_identity(&interaction.target),
            interaction.kind,
            interaction.reason,
            interaction.confidence,
            interaction.activation,
            interaction.order,
        );
        match grouped.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(interaction);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                existing.evidence.extend(interaction.evidence);
                existing.evidence.sort_by_key(evidence_identity);
                existing
                    .evidence
                    .dedup_by(|left, right| evidence_identity(left) == evidence_identity(right));
            }
        }
    }
    grouped.into_values().collect()
}

fn coalesce_risks(risks: Vec<Risk>) -> Vec<Risk> {
    let mut grouped = BTreeMap::<String, Risk>::new();
    for risk in risks {
        let key = format!(
            "{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}",
            risk.left_artifact,
            risk.right_artifact,
            target_identity(&risk.target),
            risk.rule,
            risk.reason,
            risk.order,
            risk.severity,
            risk.confidence,
            risk.activation,
            risk.precision,
        );
        match grouped.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(risk);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                existing.left_mutations.extend(risk.left_mutations);
                existing.right_mutations.extend(risk.right_mutations);
                existing.evidence.extend(risk.evidence);
                existing.left_mutations.sort();
                existing.left_mutations.dedup();
                existing.right_mutations.sort();
                existing.right_mutations.dedup();
                existing.evidence.sort_by_key(evidence_identity);
                existing
                    .evidence
                    .dedup_by(|left, right| evidence_identity(left) == evidence_identity(right));
            }
        }
    }
    grouped.into_values().collect()
}

fn target_identity(target: &Target) -> String {
    let member = target.member.as_ref().map_or_else(
        || target.class.clone(),
        |member| {
            format!(
                "{}#{}{}:{:?}",
                target.class, member.name, member.descriptor, member.kind
            )
        },
    );
    target
        .instruction
        .as_ref()
        .map_or(member.clone(), |instruction| {
            instruction.identity.as_ref().map_or_else(
                || format!("{member}@offset:{}", instruction.stable_id),
                |identity| {
                    format!(
                        "{member}@{}:{}:{}{}:{}",
                        identity.definition.artifact_id,
                        identity.definition.entry_path,
                        identity.method_name,
                        identity.method_descriptor,
                        identity.instruction_index
                    )
                },
            )
        })
}

fn evidence_identity(evidence: &crate::model::Evidence) -> String {
    let instruction = evidence.instruction.as_ref().map_or_else(
        || "none".to_string(),
        |instruction| {
            instruction.identity.as_ref().map_or_else(
                || format!("offset:{}", instruction.stable_id),
                |identity| {
                    format!(
                        "{}:{}:{}{}:{}",
                        identity.definition.artifact_id,
                        identity.definition.entry_path,
                        identity.method_name,
                        identity.method_descriptor,
                        identity.instruction_index
                    )
                },
            )
        },
    );
    format!(
        "{}|{}|{}|{}|{}|{}",
        evidence.artifact_id,
        evidence.class,
        evidence.method.as_deref().unwrap_or_default(),
        evidence.annotation.as_deref().unwrap_or_default(),
        instruction,
        evidence.detail
    )
}

pub(crate) fn analyze_effects(effects: &[Effect]) -> Vec<Risk> {
    let mut buckets: BTreeMap<String, Vec<&Effect>> = BTreeMap::new();
    for effect in effects {
        if effect.target.class.is_empty() {
            continue;
        }
        buckets
            .entry(target_bucket(&effect.target))
            .or_default()
            .push(effect);
    }
    let mut risks = Vec::new();
    for bucket in buckets.values() {
        for (index, left) in bucket.iter().enumerate() {
            for right in &bucket[index + 1..] {
                if left.artifact_id == right.artifact_id {
                    continue;
                }
                risks.extend(compare(left, right));
            }
        }
    }
    risks
}

pub(crate) fn behavioral_interactions(effects: &[Effect]) -> Vec<BehavioralInteraction> {
    let mut buckets: BTreeMap<String, Vec<&Effect>> = BTreeMap::new();
    for effect in effects {
        if !effect.target.class.is_empty() {
            buckets
                .entry(target_bucket(&effect.target))
                .or_default()
                .push(effect);
        }
    }
    let mut interactions = Vec::new();
    for bucket in buckets.values() {
        for (index, left) in bucket.iter().enumerate() {
            for right in &bucket[index + 1..] {
                if left.artifact_id == right.artifact_id {
                    continue;
                }
                let kind = left.mutations.iter().find_map(|left_mutation| {
                    right.mutations.iter().find_map(|right_mutation| {
                        if !mutations_overlap_instruction(left_mutation, right_mutation) {
                            return None;
                        }
                        match (left_mutation.composition, right_mutation.composition) {
                            (
                                CompositionSemantics::LocalValueDecorator,
                                CompositionSemantics::LocalValueDecorator,
                            )
                            | (
                                CompositionSemantics::ValueDecorator,
                                CompositionSemantics::ValueDecorator,
                            ) => Some(BehavioralInteractionKind::OrderedValueDecorators),
                            (
                                CompositionSemantics::OperationWrapper,
                                CompositionSemantics::OperationWrapper,
                            )
                            | (
                                CompositionSemantics::AdjacentInsertion,
                                CompositionSemantics::AdjacentInsertion,
                            ) => Some(BehavioralInteractionKind::OrderDependentTransformation),
                            _ => None,
                        }
                    })
                });
                let Some(kind) = kind else {
                    continue;
                };
                let mut evidence = left.evidence.clone();
                evidence.extend(right.evidence.clone());
                interactions.push(BehavioralInteraction {
                    left_artifact: left.artifact_id.clone(),
                    right_artifact: right.artifact_id.clone(),
                    target: more_precise_target(&left.target, &right.target),
                    kind,
                    reason: "Both transformations can apply, but their composed behavior depends on Mixin order."
                        .to_string(),
                    evidence,
                    confidence: left.confidence.min(right.confidence),
                    activation: combine_activation(left.activation, right.activation),
                    order: effect_execution_order(left, right),
                });
            }
        }
    }
    interactions
}

fn effect_execution_order(left: &Effect, right: &Effect) -> OrderAnalysis {
    let comparison = left
        .injector_order
        .zip(right.injector_order)
        .and_then(|(left, right)| (left != right).then(|| left.cmp(&right)))
        .or_else(|| {
            left.mixin_priority
                .zip(right.mixin_priority)
                .and_then(|(left, right)| (left != right).then(|| left.cmp(&right)))
        })
        .or_else(|| {
            left.config_priority
                .zip(right.config_priority)
                .and_then(|(left, right)| (left != right).then(|| left.cmp(&right)))
        });
    match comparison {
        Some(std::cmp::Ordering::Less) => OrderAnalysis::LeftMustRunFirst,
        Some(std::cmp::Ordering::Greater) => OrderAnalysis::RightMustRunFirst,
        _ => OrderAnalysis::BothApplyDifferentResult,
    }
}

fn target_bucket(target: &Target) -> String {
    target.member.as_ref().map_or_else(
        || format!("C:{}", target.class),
        |member| {
            format!(
                "M:{}:{}:{}:{:?}",
                target.class, member.name, member.descriptor, member.kind
            )
        },
    )
}

fn compare(left: &Effect, right: &Effect) -> Vec<Risk> {
    let mut matches = Vec::new();
    for left_mutation in &left.mutations {
        for right_mutation in &right.mutations {
            extend_match(&mut matches, write_write(left_mutation, right_mutation));
        }
        for requirement in &right.requirements {
            extend_match(
                &mut matches,
                write_shape(left, left_mutation, right, requirement),
            );
        }
        for query in &right.queries {
            extend_match(&mut matches, write_query(left, left_mutation, right, query));
        }
    }
    for right_mutation in &right.mutations {
        for requirement in &left.requirements {
            if let Some(mut candidate) = write_shape(right, right_mutation, left, requirement) {
                candidate.order = reverse_order(candidate.order);
                extend_match(&mut matches, Some(candidate));
            }
        }
        for query in &left.queries {
            if let Some(mut candidate) = write_query(right, right_mutation, left, query) {
                candidate.order = reverse_order(candidate.order);
                extend_match(&mut matches, Some(candidate));
            }
        }
    }
    matches
        .into_iter()
        .map(|matched| risk_from_match(left, right, matched))
        .collect()
}

fn risk_from_match(left: &Effect, right: &Effect, matched: ConflictMatch) -> Risk {
    let confidence = left
        .confidence
        .min(right.confidence)
        .min(matched.confidence_cap);
    let activation = combine_activation(left.activation, right.activation);
    let heuristic_transformer = [left, right].iter().any(|effect| {
        matches!(
            effect.mechanism,
            crate::model::Mechanism::ModLauncherTransformer | crate::model::Mechanism::JavaCoremod
        ) && effect.confidence < Confidence::Exact
    });
    let severity = if heuristic_transformer {
        matched.severity.min(Severity::Medium)
    } else {
        matched.severity
    };
    let risk_index = risk_index(severity, confidence, activation);
    let precision = less_precise(left.precision, right.precision);
    let (left_artifact, right_artifact, left_mutations, right_mutations) =
        if left.artifact_id <= right.artifact_id {
            (
                left.artifact_id.clone(),
                right.artifact_id.clone(),
                left.mutations
                    .iter()
                    .map(|mutation| mutation.kind)
                    .collect(),
                right
                    .mutations
                    .iter()
                    .map(|mutation| mutation.kind)
                    .collect(),
            )
        } else {
            (
                right.artifact_id.clone(),
                left.artifact_id.clone(),
                right
                    .mutations
                    .iter()
                    .map(|mutation| mutation.kind)
                    .collect(),
                left.mutations
                    .iter()
                    .map(|mutation| mutation.kind)
                    .collect(),
            )
        };
    let mut evidence = left.evidence.clone();
    evidence.extend(right.evidence.clone());
    Risk {
        left_artifact,
        right_artifact,
        target: more_precise_target(&left.target, &right.target),
        rule: matched.rule.to_string(),
        reason: matched.reason.to_string(),
        left_mutations,
        right_mutations,
        evidence,
        order: matched.order,
        severity,
        confidence,
        precision,
        risk_index,
        activation,
    }
}

#[derive(Clone, Copy)]
struct ConflictMatch {
    rule: &'static str,
    reason: &'static str,
    severity: Severity,
    order: OrderAnalysis,
    confidence_cap: Confidence,
}

fn extend_match(current: &mut Vec<ConflictMatch>, candidate: Option<ConflictMatch>) {
    let Some(candidate) = candidate else {
        return;
    };
    if !current.iter().any(|existing| {
        existing.rule == candidate.rule
            && existing.reason == candidate.reason
            && existing.severity == candidate.severity
            && existing.order == candidate.order
            && existing.confidence_cap == candidate.confidence_cap
    }) {
        current.push(candidate);
    }
}

fn less_precise(left: Precision, right: Precision) -> Precision {
    fn rank(precision: Precision) -> u8 {
        match precision {
            Precision::Instruction => 0,
            Precision::Pattern => 1,
            Precision::Method => 2,
            Precision::Class => 3,
            Precision::Unknown => 4,
        }
    }
    if rank(left) >= rank(right) {
        left
    } else {
        right
    }
}

fn write_write(left: &Mutation, right: &Mutation) -> Option<ConflictMatch> {
    use MutationKind::{
        AddField, AddInterfaces, ChangeAccess, ChangeInterfaces, ChangeSuperclass, RemoveField,
        RemoveMethod, ReplaceMethodBody,
    };
    if mutations_overlap_instruction(left, right) {
        use CompositionSemantics::{
            Destructive, ExclusiveOwner, LocalValueDecorator, OperationWrapper,
        };
        match (left.composition, right.composition) {
            (ExclusiveOwner, ExclusiveOwner) => {
                return Some(ConflictMatch {
                    rule: "exclusive_instruction_write",
                    reason: "Two exclusive owners target the same original instruction.",
                    severity: Severity::Critical,
                    order: OrderAnalysis::Exclusive,
                    confidence_cap: Confidence::Exact,
                });
            }
            (ExclusiveOwner, Destructive) | (Destructive, ExclusiveOwner) => {
                return Some(ConflictMatch {
                    rule: "exclusive_destructive_instruction_write",
                    reason: "An exclusive operation owner and a destructive rewrite target the same original instruction.",
                    severity: Severity::Critical,
                    order: OrderAnalysis::Exclusive,
                    confidence_cap: Confidence::Exact,
                });
            }
            (ExclusiveOwner, OperationWrapper) | (OperationWrapper, ExclusiveOwner) => {
                return Some(ConflictMatch {
                    rule: "exclusive_operation_wrapper",
                    reason: "An exclusive redirect and an operation wrapper compete for the same operation.",
                    severity: Severity::High,
                    order: OrderAnalysis::Exclusive,
                    confidence_cap: Confidence::High,
                });
            }
            (Destructive, Destructive) => {
                return Some(ConflictMatch {
                    rule: "destructive_instruction_write",
                    reason: "Two destructive rewrites target the same original instruction.",
                    severity: Severity::Critical,
                    order: OrderAnalysis::Exclusive,
                    confidence_cap: Confidence::Exact,
                });
            }
            (LocalValueDecorator, LocalValueDecorator) => {}
            _ => {}
        }
    }
    if matches!(
        (left.kind, right.kind),
        (ChangeInterfaces, ChangeInterfaces)
            | (AddInterfaces, ChangeInterfaces)
            | (ChangeInterfaces, AddInterfaces)
    ) {
        return Some(ConflictMatch {
            rule: "interface_shape_overlap",
            reason: "Both artifacts change the interface set; the exact merge behavior is not known.",
            severity: Severity::Medium,
            order: OrderAnalysis::Structural,
            confidence_cap: Confidence::Medium,
        });
    }
    if matches!(
        (left.kind, right.kind),
        (RemoveField, RemoveField)
            | (RemoveMethod, RemoveMethod)
            | (ChangeSuperclass, ChangeSuperclass)
    ) {
        return Some(ConflictMatch {
            rule: "member_shape_write_write",
            reason: "Both artifacts change the same class or member shape.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
            confidence_cap: Confidence::Exact,
        });
    }
    if matches!(
        (left.kind, right.kind),
        (ReplaceMethodBody | ChangeAccess, RemoveMethod)
            | (RemoveMethod, ReplaceMethodBody | ChangeAccess)
            | (AddField | ChangeAccess, RemoveField)
            | (RemoveField, AddField | ChangeAccess)
    ) {
        return Some(ConflictMatch {
            rule: "member_shape_write_write",
            reason: "One artifact removes, replaces, adds, or changes access to a member that the other artifact also changes.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
            confidence_cap: Confidence::Exact,
        });
    }
    if left.kind == ChangeAccess && right.kind == ChangeAccess {
        if let (Some(left), Some(right)) = (left.access_delta, right.access_delta) {
            let contradictory = left.added_flags & right.removed_flags != 0
                || right.added_flags & left.removed_flags != 0;
            return contradictory.then_some(ConflictMatch {
                rule: "contradictory_access_change",
                reason: "The access-flag deltas add and remove at least one of the same flags.",
                severity: Severity::High,
                order: OrderAnalysis::Structural,
                confidence_cap: Confidence::Exact,
            });
        }
        return Some(ConflictMatch {
            rule: "unknown_access_overlap",
            reason: "Both artifacts change access flags, but their exact added/removed flag deltas are unavailable.",
            severity: Severity::Medium,
            order: OrderAnalysis::Structural,
            confidence_cap: Confidence::Medium,
        });
    }
    None
}

fn write_shape(
    writer: &Effect,
    mutation: &Mutation,
    _reader: &Effect,
    requirement: &crate::model::ShapeRequirement,
) -> Option<ConflictMatch> {
    use MutationKind::{
        ChangeLocalLayout, InsertInstructions, RemoveField, RemoveInstruction, RemoveMethod,
        ReplaceInstruction,
    };
    use RequirementKind::{InstructionExists, LocalLayout, MemberExists, SliceBoundary};
    let kind = requirement.kind;
    if matches!(mutation.kind, RemoveInstruction | ReplaceInstruction)
        && matches!(kind, InstructionExists | SliceBoundary)
        && requirements_overlap_instruction(mutation, requirement)
    {
        return Some(ConflictMatch {
            rule: "instruction_write_invalidates_selector",
            reason: "A destructive instruction mutation may invalidate the other artifact's selector or slice anchor.",
            severity: Severity::High,
            order: OrderAnalysis::AnchorInvalidated,
            confidence_cap: Confidence::Exact,
        });
    }
    if matches!(mutation.kind, RemoveMethod | RemoveField)
        && kind == MemberExists
        && same_member(&mutation.target, &requirement.target)
    {
        return Some(ConflictMatch {
            rule: "member_removal_invalidates_requirement",
            reason: "A member removal invalidates a member required by the other artifact.",
            severity: Severity::High,
            order: OrderAnalysis::AnchorInvalidated,
            confidence_cap: Confidence::Exact,
        });
    }
    if mutation.kind == InsertInstructions
        && requirement.ordinal.is_some()
        && matches!(
            writer.mechanism,
            crate::model::Mechanism::ModLauncherTransformer | crate::model::Mechanism::JavaCoremod
        )
        && requirements_overlap_instruction(mutation, requirement)
    {
        return Some(ConflictMatch {
            rule: "insertion_changes_selector_ordinal",
            reason: "A heuristic instruction insertion may change ordinal selection.",
            severity: Severity::Medium,
            order: OrderAnalysis::OrdinalChanged,
            confidence_cap: Confidence::Low,
        });
    }
    if mutation.kind == ChangeLocalLayout && kind == LocalLayout {
        return Some(ConflictMatch {
            rule: "local_layout_dependency",
            reason: "A local-layout mutation overlaps a local-capture requirement.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
            confidence_cap: Confidence::Exact,
        });
    }
    None
}

fn write_query(
    writer: &Effect,
    mutation: &Mutation,
    _reader: &Effect,
    query: &InjectionQuery,
) -> Option<ConflictMatch> {
    if mutation.target.member.as_ref().is_some() && !same_member(&mutation.target, &query.method) {
        return None;
    }
    let selected_count = u32::try_from(query.selected.len()).unwrap_or(u32::MAX);
    let impacted_selected = u32::try_from(
        query
            .selected
            .iter()
            .filter(|instruction| mutation_overlaps_reference(mutation, instruction))
            .count(),
    )
    .unwrap_or(u32::MAX);
    let impacted_candidates = query
        .candidates
        .iter()
        .filter(|instruction| mutation_overlaps_reference(mutation, instruction))
        .count();
    let destructive = mutation.composition == CompositionSemantics::Destructive;

    if destructive && impacted_candidates > 0 && query.ordinal.is_some() {
        return Some(ConflictMatch {
            rule: "destructive_write_changes_query_ordinal",
            reason: "Removing or replacing a candidate before ordinal selection can choose a different join point.",
            severity: Severity::Medium,
            order: OrderAnalysis::OrdinalChanged,
            confidence_cap: Confidence::Exact,
        });
    }

    if destructive && impacted_selected > 0 {
        let remaining = selected_count.saturating_sub(impacted_selected);
        if let Some(group) = &query.group {
            let member_was_successful = query
                .minimum_matches
                .is_none_or(|minimum| selected_count >= minimum)
                && query
                    .maximum_matches
                    .is_none_or(|maximum| selected_count <= maximum);
            let member_remains_successful = query
                .minimum_matches
                .is_none_or(|minimum| remaining >= minimum)
                && query
                    .maximum_matches
                    .is_none_or(|maximum| remaining <= maximum);
            let group_after = group.successful_members.saturating_sub(u32::from(
                member_was_successful && !member_remains_successful,
            ));
            if group
                .minimum_successes
                .is_some_and(|minimum| group_after < minimum)
            {
                return Some(ConflictMatch {
                    rule: "injection_group_minimum_invalidated",
                    reason: "A destructive write makes an injector group fall below its aggregate minimum.",
                    severity: Severity::High,
                    order: OrderAnalysis::CardinalityInvalidated,
                    confidence_cap: Confidence::Exact,
                });
            }
        }
        if query
            .minimum_matches
            .is_some_and(|minimum| remaining < minimum)
        {
            return Some(ConflictMatch {
                rule: "injection_query_minimum_invalidated",
                reason: "A destructive write reduces the injector query's total matches below its require value.",
                severity: Severity::High,
                order: OrderAnalysis::CardinalityInvalidated,
                confidence_cap: Confidence::Exact,
            });
        }
    }

    let inserts_matching_return = mutation.kind == MutationKind::InsertConditionalReturn
        && query
            .selector_kind
            .split('+')
            .any(|kind| matches!(kind, "RETURN" | "TAIL"))
        && mutation
            .target
            .instruction
            .as_ref()
            .is_some_and(|instruction| {
                query
                    .slice_start
                    .is_none_or(|start| instruction.stable_id >= start)
                    && query
                        .slice_end
                        .is_none_or(|end| instruction.stable_id <= end)
            });
    let pattern_insertion = mutation.kind == MutationKind::InsertInstructions
        && matches!(
            writer.mechanism,
            crate::model::Mechanism::ModLauncherTransformer | crate::model::Mechanism::JavaCoremod
        )
        && query
            .candidates
            .iter()
            .any(|instruction| mutation_overlaps_reference(mutation, instruction));
    if inserts_matching_return || pattern_insertion {
        let increased = selected_count.saturating_add(1);
        if let Some(group) = &query.group {
            let member_was_successful = query
                .minimum_matches
                .is_none_or(|minimum| selected_count >= minimum)
                && query
                    .maximum_matches
                    .is_none_or(|maximum| selected_count <= maximum);
            let member_becomes_successful = !member_was_successful
                && query
                    .minimum_matches
                    .is_none_or(|minimum| increased >= minimum)
                && query
                    .maximum_matches
                    .is_none_or(|maximum| increased <= maximum);
            let group_after = group
                .successful_members
                .saturating_add(u32::from(member_becomes_successful));
            if group
                .maximum_successes
                .is_some_and(|maximum| group_after > maximum)
            {
                return Some(ConflictMatch {
                    rule: "injection_group_maximum_invalidated",
                    reason: "An inserted match makes an injector group exceed its aggregate maximum.",
                    severity: Severity::High,
                    order: OrderAnalysis::CardinalityInvalidated,
                    confidence_cap: if inserts_matching_return {
                        Confidence::High
                    } else {
                        Confidence::Low
                    },
                });
            }
        }
        if query
            .maximum_matches
            .is_some_and(|maximum| increased > maximum)
        {
            return Some(ConflictMatch {
                rule: "injection_query_maximum_invalidated",
                reason: "An inserted matching instruction raises the injector query above its allow value.",
                severity: Severity::High,
                order: OrderAnalysis::CardinalityInvalidated,
                confidence_cap: if inserts_matching_return {
                    Confidence::High
                } else {
                    Confidence::Low
                },
            });
        }
        if query.ordinal.is_some() {
            return Some(ConflictMatch {
                rule: "insertion_changes_query_ordinal",
                reason: "An inserted matching instruction can change ordinal selection.",
                severity: Severity::Medium,
                order: OrderAnalysis::OrdinalChanged,
                confidence_cap: if inserts_matching_return {
                    Confidence::High
                } else {
                    Confidence::Low
                },
            });
        }
    }
    None
}

fn requirements_overlap_instruction(
    mutation: &Mutation,
    requirement: &crate::model::ShapeRequirement,
) -> bool {
    match (
        mutation.target.instruction.as_ref(),
        requirement.target.instruction.as_ref(),
    ) {
        (Some(left), Some(right)) => match (mutation.precision, requirement.precision) {
            (Precision::Instruction, Precision::Instruction) => {
                same_instruction_identity(left, right)
            }
            (Precision::Pattern, _) | (_, Precision::Pattern) => {
                instruction_patterns_overlap(Some(left), Some(right))
            }
            _ => false,
        },
        _ => false,
    }
}

fn mutations_overlap_instruction(left: &Mutation, right: &Mutation) -> bool {
    match (
        left.target.instruction.as_ref(),
        right.target.instruction.as_ref(),
    ) {
        (Some(left_instruction), Some(right_instruction)) => {
            match (left.precision, right.precision) {
                (Precision::Instruction, Precision::Instruction) => {
                    same_instruction_identity(left_instruction, right_instruction)
                }
                (Precision::Pattern, _) | (_, Precision::Pattern) => {
                    instruction_patterns_overlap(Some(left_instruction), Some(right_instruction))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn same_instruction_identity(
    left: &crate::model::InstructionReference,
    right: &crate::model::InstructionReference,
) -> bool {
    match (&left.identity, &right.identity) {
        (Some(left), Some(right)) => left == right,
        (None, None) => left.stable_id == right.stable_id,
        _ => false,
    }
}

fn mutation_overlaps_reference(
    mutation: &Mutation,
    reference: &crate::model::InstructionReference,
) -> bool {
    let Some(target) = mutation.target.instruction.as_ref() else {
        return false;
    };
    match mutation.precision {
        Precision::Instruction => same_instruction_identity(target, reference),
        Precision::Pattern => instruction_patterns_overlap(Some(target), Some(reference)),
        Precision::Method | Precision::Class | Precision::Unknown => false,
    }
}

fn instruction_patterns_overlap(
    left: Option<&crate::model::InstructionReference>,
    right: Option<&crate::model::InstructionReference>,
) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    match (&left.member, &right.member) {
        (Some(left), Some(right)) => left == right,
        (None, None) => {
            left.constant.is_some()
                && left.constant == right.constant
                && left.opcode == right.opcode
        }
        _ => false,
    }
}

fn same_member(left: &Target, right: &Target) -> bool {
    match (&left.member, &right.member) {
        (Some(left), Some(right)) => {
            left.owner == right.owner
                && left.name == right.name
                && left.descriptor == right.descriptor
                && left.kind == right.kind
        }
        _ => false,
    }
}

fn reverse_order(order: OrderAnalysis) -> OrderAnalysis {
    match order {
        OrderAnalysis::LeftMustRunFirst => OrderAnalysis::RightMustRunFirst,
        OrderAnalysis::RightMustRunFirst => OrderAnalysis::LeftMustRunFirst,
        other => other,
    }
}

fn combine_activation(left: Activation, right: Activation) -> Activation {
    use Activation::{Candidate, Conditional, Definite, Unknown};
    match (left, right) {
        (Unknown, _) | (_, Unknown) => Unknown,
        (Candidate, _) | (_, Candidate) => Candidate,
        (Conditional, _) | (_, Conditional) => Conditional,
        (Definite, Definite) => Definite,
    }
}

fn risk_index(severity: Severity, confidence: Confidence, activation: Activation) -> u8 {
    let activation_factor = match activation {
        Activation::Definite => 100_u32,
        Activation::Conditional => 80,
        Activation::Candidate => 55,
        Activation::Unknown => 35,
    };
    let product = u32::from(severity.score()) * u32::from(confidence.score()) * activation_factor;
    u8::try_from(((product + 5_000) / 10_000).min(100)).unwrap_or(100)
}

fn more_precise_target(left: &Target, right: &Target) -> Target {
    if left.instruction.is_some() || right.instruction.is_none() {
        left.clone()
    } else {
        right.clone()
    }
}

fn binary_shape_risks(scanned: &ScannedArtifacts) -> Vec<Risk> {
    let mut risks = Vec::new();
    let definitions_by_class: HashMap<_, _> = scanned
        .universe
        .classes
        .iter()
        .filter(|(_, definitions)| definitions.len() > 1)
        .collect();
    for definitions in scanned
        .universe
        .classes
        .values()
        .flatten()
        .filter(|definition| definition.is_mod)
    {
        for reference in &definitions.hard_references {
            let Some(candidates) = definitions_by_class.get(&reference.owner) else {
                continue;
            };
            let valid = candidates
                .iter()
                .filter(|candidate| {
                    scanned
                        .universe
                        .definition_resolves_member(candidate, reference)
                })
                .count();
            if valid == 0 || valid == candidates.len() {
                continue;
            }
            for candidate in candidates.iter().filter(|candidate| {
                candidate.is_mod
                    && !scanned
                        .universe
                        .definition_resolves_member(candidate, reference)
            }) {
                if candidate.artifact_id == definitions.artifact_id {
                    continue;
                }
                if candidate
                    .definition_id
                    .as_ref()
                    .zip(definitions.definition_id.as_ref())
                    .is_some_and(|(left, right)| left.content_hash == right.content_hash)
                {
                    continue;
                }
                let confidence = Confidence::Exact;
                risks.push(Risk {
                    left_artifact: definitions.artifact_id.clone(),
                    right_artifact: candidate.artifact_id.clone(),
                    target: Target {
                        class: reference.owner.clone(),
                        member: Some(reference.clone()),
                        instruction: None,
                    },
                    rule: "duplicate_class_reference_invalidation".to_string(),
                    reason: "Classpath shadowing may select a duplicate class definition that lacks a hard-referenced member.".to_string(),
                    left_mutations: Vec::new(),
                    right_mutations: vec![MutationKind::UnknownClass],
                    evidence: Vec::new(),
                    order: OrderAnalysis::Structural,
                    severity: Severity::High,
                    confidence,
                    precision: Precision::Class,
                    risk_index: risk_index(Severity::High, confidence, Activation::Conditional),
                    activation: Activation::Conditional,
                });
            }
        }
    }
    risks
}

#[cfg(test)]
mod tests {
    use crate::jar::{ClassDefinition, ClassUniverse, ScannedArtifacts};
    use crate::model::{
        Activation, AnalysisLimits, ClassDefinitionId, Confidence, Coverage, Effect, Evidence,
        InjectionGroupConstraint, InjectionQuery, InstructionIdentity, InstructionReference,
        Mechanism, MemberKind, MemberReference, Mutation, MutationKind, Precision, RequirementKind,
        ShapeRequirement, SoftReferenceResolution, Target,
    };

    use super::*;

    fn instruction_target(id: u32) -> Target {
        let mut target = Target::method("game/Foo", "tick", "()V");
        target.instruction = Some(InstructionReference {
            identity: None,
            stable_id: id,
            original_offset: Some(id),
            opcode: 182,
            local_slot: None,
            member: Some(MemberReference {
                owner: "game/Bar".to_string(),
                name: "call".to_string(),
                descriptor: "()V".to_string(),
                kind: MemberKind::Method,
                is_static: Some(false),
            }),
            constant: None,
        });
        target
    }

    fn effect(artifact: &str, mutation: Mutation, requirements: Vec<ShapeRequirement>) -> Effect {
        Effect {
            artifact_id: artifact.to_string(),
            mechanism: Mechanism::Mixin,
            target: mutation.target.clone(),
            requirements,
            queries: Vec::new(),
            mutations: vec![mutation],
            evidence: vec![Evidence::new(artifact, format!("{artifact}/Mixin"), "test")],
            precision: Precision::Instruction,
            confidence: Confidence::Exact,
            activation: Activation::Candidate,
            config_priority: None,
            mixin_priority: None,
            injector_order: None,
        }
    }

    fn mutation(kind: MutationKind, target: Target) -> Mutation {
        let precision = if target.instruction.is_some() {
            Precision::Instruction
        } else if target.member.is_some() {
            Precision::Method
        } else {
            Precision::Class
        };
        Mutation::new(kind, target, precision)
    }

    fn requirement(kind: RequirementKind, target: Target) -> ShapeRequirement {
        ShapeRequirement {
            kind,
            precision: if target.instruction.is_some() {
                Precision::Instruction
            } else if target.member.is_some() {
                Precision::Method
            } else {
                Precision::Class
            },
            target,
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }
    }

    fn query(method: Target, selected: Vec<InstructionReference>) -> InjectionQuery {
        InjectionQuery {
            id: "query".to_string(),
            method_selector: crate::model::MethodSelector::Exact {
                owner: Some("game/Foo".to_string()),
                name: "tick".to_string(),
                descriptor: Some("()V".to_string()),
            },
            selector_kind: "INVOKE".to_string(),
            target_selectors: Vec::new(),
            method,
            candidates: selected.clone(),
            selected,
            minimum_matches: None,
            maximum_matches: None,
            expected_matches: None,
            ordinal: None,
            shift: None,
            slice: None,
            slice_start: None,
            slice_end: None,
            resolution: SoftReferenceResolution::DirectExact,
            local_selector: None,
            group: None,
        }
    }

    #[test]
    fn two_redirects_on_one_instruction_are_critical() {
        let target = instruction_target(7);
        let effects = ["a", "b"].map(|artifact| {
            effect(
                artifact,
                mutation(MutationKind::RedirectOperation, target.clone()),
                Vec::new(),
            )
        });

        let risks = analyze_effects(&effects);

        assert_eq!(risks.len(), 1);
        assert_eq!(risks[0].severity, Severity::Critical);
        assert_eq!(risks[0].order, OrderAnalysis::Exclusive);
        assert_ne!(risks[0].risk_index, 100);
    }

    #[test]
    fn redirect_and_value_decorator_are_composable_on_the_same_instruction() {
        let target = instruction_target(7);
        let redirect = effect(
            "redirect",
            mutation(MutationKind::RedirectOperation, target.clone()),
            Vec::new(),
        );
        let decorator = effect(
            "decorator",
            mutation(MutationKind::TransformExpressionValue, target.clone()),
            vec![requirement(RequirementKind::InstructionExists, target)],
        );

        assert!(analyze_effects(&[redirect, decorator]).is_empty());
    }

    #[test]
    fn value_decorators_and_operation_wrappers_chain() {
        for kind in [
            MutationKind::TransformExpressionValue,
            MutationKind::WrapOperation,
        ] {
            let target = instruction_target(7);
            let effects = ["a", "b"]
                .map(|artifact| effect(artifact, mutation(kind, target.clone()), Vec::new()));
            assert!(analyze_effects(&effects).is_empty());
        }
    }

    #[test]
    fn injector_order_determines_value_decorator_composition_order() {
        let target = instruction_target(7);
        let mut later = effect(
            "later",
            mutation(MutationKind::TransformExpressionValue, target.clone()),
            Vec::new(),
        );
        later.injector_order = Some(2_000);
        let mut earlier = effect(
            "earlier",
            mutation(MutationKind::TransformExpressionValue, target),
            Vec::new(),
        );
        earlier.injector_order = Some(1_000);

        let interactions = behavioral_interactions(&[later, earlier]);

        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].order, OrderAnalysis::RightMustRunFirst);
    }

    #[test]
    fn mixin_interface_additions_are_composable() {
        let target = Target::class("game/Foo");
        let effects = ["a", "b"].map(|artifact| {
            effect(
                artifact,
                mutation(MutationKind::AddInterfaces, target.clone()),
                Vec::new(),
            )
        });

        assert!(analyze_effects(&effects).is_empty());
    }

    #[test]
    fn destructive_write_invalidates_a_value_decorator_anchor() {
        let target = instruction_target(7);
        let removal = effect(
            "removal",
            mutation(MutationKind::RemoveInstruction, target.clone()),
            Vec::new(),
        );
        let decorator = effect(
            "decorator",
            mutation(MutationKind::TransformExpressionValue, target.clone()),
            vec![requirement(RequirementKind::InstructionExists, target)],
        );

        let risks = analyze_effects(&[removal, decorator]);

        assert_eq!(risks.len(), 1);
        assert_eq!(risks[0].rule, "instruction_write_invalidates_selector");
    }

    #[test]
    fn concrete_instructions_with_the_same_pattern_but_different_ids_do_not_overlap() {
        let left = effect(
            "left",
            mutation(MutationKind::RedirectOperation, instruction_target(10)),
            Vec::new(),
        );
        let right = effect(
            "right",
            mutation(MutationKind::RedirectOperation, instruction_target(42)),
            Vec::new(),
        );

        assert!(analyze_effects(&[left, right]).is_empty());
    }

    #[test]
    fn equal_method_offsets_in_different_class_definitions_do_not_collide() {
        let target = |artifact: &str| {
            let mut target = instruction_target(7);
            target.instruction.as_mut().unwrap().identity = Some(InstructionIdentity {
                definition: ClassDefinitionId {
                    artifact_id: artifact.to_string(),
                    entry_path: "game/Foo.class".to_string(),
                    class_name: "game/Foo".to_string(),
                    content_hash: artifact.to_string(),
                },
                method_name: "tick".to_string(),
                method_descriptor: "()V".to_string(),
                instruction_index: 7,
            });
            target
        };
        let left = effect(
            "left",
            mutation(MutationKind::RedirectOperation, target("left-definition")),
            Vec::new(),
        );
        let right = effect(
            "right",
            mutation(MutationKind::RedirectOperation, target("right-definition")),
            Vec::new(),
        );

        assert!(analyze_effects(&[left, right]).is_empty());
    }

    #[test]
    fn overwrite_is_not_a_blanket_internal_anchor_risk() {
        let target = instruction_target(4);
        let overwrite = effect(
            "overwrite",
            mutation(
                MutationKind::ReplaceMethodBody,
                Target::method("game/Foo", "tick", "()V"),
            ),
            Vec::new(),
        );
        let inject = effect(
            "inject",
            mutation(MutationKind::InsertInstructions, target.clone()),
            vec![requirement(RequirementKind::InstructionExists, target)],
        );

        assert!(analyze_effects(&[overwrite, inject]).is_empty());
    }

    #[test]
    fn member_removal_invalidates_a_required_target() {
        let method = Target::method("game/Foo", "tick", "()V");
        let removal = effect(
            "removal",
            mutation(MutationKind::RemoveMethod, method.clone()),
            Vec::new(),
        );
        let overwrite = effect(
            "overwrite",
            mutation(MutationKind::ReplaceMethodBody, method.clone()),
            vec![requirement(RequirementKind::MemberExists, method)],
        );

        let risks = analyze_effects(&[removal, overwrite]);

        assert_eq!(risks[0].rule, "member_shape_write_write");
        assert_eq!(risks[0].severity, Severity::High);
    }

    #[test]
    fn query_cardinality_is_checked_for_the_whole_injector() {
        let targets = [1, 2, 3].map(instruction_target);
        let mut removal = effect(
            "removal",
            mutation(MutationKind::RemoveInstruction, targets[0].clone()),
            Vec::new(),
        );
        removal.precision = Precision::Instruction;
        let mut reader = effect(
            "reader",
            mutation(MutationKind::InsertInstructions, targets[1].clone()),
            Vec::new(),
        );
        let mut injector_query = query(
            Target::method("game/Foo", "tick", "()V"),
            targets
                .iter()
                .map(|target| target.instruction.clone().unwrap())
                .collect(),
        );
        injector_query.minimum_matches = Some(1);
        reader.queries.push(injector_query);

        assert!(analyze_effects(&[removal, reader]).is_empty());
    }

    #[test]
    fn expect_is_not_a_production_cardinality_minimum() {
        let target = instruction_target(1);
        let removal = effect(
            "removal",
            mutation(MutationKind::RemoveInstruction, target.clone()),
            Vec::new(),
        );
        let mut reader = effect(
            "reader",
            mutation(MutationKind::InsertInstructions, target.clone()),
            Vec::new(),
        );
        let mut injector_query = query(
            Target::method("game/Foo", "tick", "()V"),
            vec![target.instruction.unwrap()],
        );
        injector_query.expected_matches = Some(2);
        reader.queries.push(injector_query);

        assert!(analyze_effects(&[removal, reader]).is_empty());
    }

    #[test]
    fn local_layout_changes_conflict_with_capture_but_local_value_changes_do_not() {
        let method = Target::method("game/Foo", "tick", "()V");
        let local = effect(
            "local-writer",
            mutation(MutationKind::ChangeLocalLayout, method.clone()),
            Vec::new(),
        );
        let capture = effect(
            "capture",
            mutation(MutationKind::InsertInstructions, method.clone()),
            vec![requirement(RequirementKind::LocalLayout, method.clone())],
        );

        let risks = analyze_effects(&[local, capture.clone()]);

        assert_eq!(risks[0].rule, "local_layout_dependency");
        assert_eq!(risks[0].severity, Severity::High);
        let value_writer = effect(
            "writer",
            mutation(MutationKind::ModifyLocalValue, method),
            Vec::new(),
        );

        assert!(analyze_effects(&[value_writer, capture]).is_empty());
    }

    #[test]
    fn cancellable_head_does_not_conflict_with_an_unbounded_return_query() {
        let target = instruction_target(0);
        let cancellable = effect(
            "cancellable",
            mutation(MutationKind::InsertConditionalReturn, target.clone()),
            Vec::new(),
        );
        let mut return_injector = effect(
            "return",
            mutation(MutationKind::InsertInstructions, target.clone()),
            Vec::new(),
        );
        let mut return_query = query(
            Target::method("game/Foo", "tick", "()V"),
            vec![target.instruction.unwrap()],
        );
        return_query.selector_kind = "RETURN".to_string();
        return_injector.queries.push(return_query);

        assert!(analyze_effects(&[cancellable, return_injector]).is_empty());
    }

    #[test]
    fn group_minimum_is_aggregate_across_members() {
        let target = instruction_target(1);
        let removal = effect(
            "removal",
            mutation(MutationKind::RemoveInstruction, target.clone()),
            Vec::new(),
        );
        let mut grouped = effect(
            "grouped",
            mutation(MutationKind::InsertInstructions, target.clone()),
            Vec::new(),
        );
        let mut grouped_query = query(
            Target::method("game/Foo", "tick", "()V"),
            vec![target.instruction.unwrap()],
        );
        grouped_query.minimum_matches = Some(1);
        grouped_query.group = Some(InjectionGroupConstraint {
            id: "group".to_string(),
            member_id: "handler".to_string(),
            successful_members: 2,
            minimum_successes: Some(2),
            maximum_successes: None,
        });
        grouped.queries.push(grouped_query);

        let risks = analyze_effects(&[removal, grouped]);
        assert_eq!(risks[0].rule, "injection_group_minimum_invalidated");
    }

    #[test]
    fn allow_fails_only_after_a_matching_instruction_is_added() {
        let insertion_target = instruction_target(0);
        let insertion = effect(
            "insertion",
            mutation(
                MutationKind::InsertConditionalReturn,
                insertion_target.clone(),
            ),
            Vec::new(),
        );
        let mut reader = effect(
            "reader",
            mutation(MutationKind::InsertInstructions, insertion_target),
            Vec::new(),
        );
        let mut return_query = query(
            Target::method("game/Foo", "tick", "()V"),
            vec![InstructionReference {
                identity: None,
                stable_id: 3,
                original_offset: Some(3),
                opcode: 177,
                local_slot: None,
                member: None,
                constant: None,
            }],
        );
        return_query.selector_kind = "RETURN".to_string();
        return_query.maximum_matches = Some(1);
        reader.queries.push(return_query);

        let risks = analyze_effects(&[insertion, reader]);

        assert_eq!(risks[0].rule, "injection_query_maximum_invalidated");
    }

    #[test]
    fn group_maximum_is_checked_after_a_member_becomes_successful() {
        let target = instruction_target(0);
        let insertion = effect(
            "insertion",
            mutation(MutationKind::InsertConditionalReturn, target.clone()),
            Vec::new(),
        );
        let mut reader = effect(
            "reader",
            mutation(MutationKind::InsertInstructions, target),
            Vec::new(),
        );
        let mut return_query = query(Target::method("game/Foo", "tick", "()V"), Vec::new());
        return_query.selector_kind = "RETURN".to_string();
        return_query.minimum_matches = Some(1);
        return_query.group = Some(InjectionGroupConstraint {
            id: "group".to_string(),
            member_id: "new-member".to_string(),
            successful_members: 1,
            minimum_successes: None,
            maximum_successes: Some(1),
        });
        reader.queries.push(return_query);

        let risks = analyze_effects(&[insertion, reader]);

        assert_eq!(risks[0].rule, "injection_group_maximum_invalidated");
    }

    #[test]
    fn access_changes_only_conflict_when_deltas_contradict() {
        let target = Target::method("game/Foo", "tick", "()V");
        let mut public = mutation(MutationKind::ChangeAccess, target.clone());
        public.access_delta = Some(crate::model::AccessDelta {
            added_flags: 0x0001,
            removed_flags: 0x0002,
        });
        assert!(
            analyze_effects(&[
                effect("a", public.clone(), Vec::new()),
                effect("b", public.clone(), Vec::new()),
            ])
            .is_empty()
        );

        let mut private = mutation(MutationKind::ChangeAccess, target.clone());
        private.access_delta = Some(crate::model::AccessDelta {
            added_flags: 0x0002,
            removed_flags: 0x0001,
        });
        let contradiction = analyze_effects(&[
            effect("a", public, Vec::new()),
            effect("b", private, Vec::new()),
        ]);
        assert_eq!(contradiction[0].rule, "contradictory_access_change");
        assert_eq!(contradiction[0].severity, Severity::High);

        let unknown = analyze_effects(&[
            effect(
                "a",
                mutation(MutationKind::ChangeAccess, target.clone()),
                Vec::new(),
            ),
            effect(
                "b",
                mutation(MutationKind::ChangeAccess, target),
                Vec::new(),
            ),
        ]);
        assert_eq!(unknown[0].rule, "unknown_access_overlap");
        assert_eq!(unknown[0].confidence, Confidence::Medium);
    }

    #[test]
    fn risk_index_is_multiplicatively_gated_by_confidence_and_activation() {
        let exact_critical =
            risk_index(Severity::Critical, Confidence::Exact, Activation::Definite);
        let weak_critical = risk_index(Severity::Critical, Confidence::Low, Activation::Candidate);
        let exact_high = risk_index(Severity::High, Confidence::Exact, Activation::Definite);

        assert_eq!(exact_critical, 100);
        assert!(weak_critical < exact_high);
        assert!(weak_critical < 30);
    }

    #[test]
    fn duplicate_class_shape_ignores_member_and_interface_declaration_order() {
        let first = definition(
            "a",
            "lib/Duplicate",
            vec!["api/A", "api/B"],
            vec![member("b", "()V", false), member("a", "()V", false)],
            Vec::new(),
        );
        let second = definition(
            "b",
            "lib/Duplicate",
            vec!["api/B", "api/A"],
            vec![member("a", "()V", false), member("b", "()V", false)],
            Vec::new(),
        );
        let scanned = scanned_with_classes(BTreeMap::from([(
            "lib/Duplicate".to_string(),
            vec![first, second],
        )]));

        assert!(binary_shape_risks(&scanned).is_empty());
    }

    #[test]
    fn duplicate_private_or_helper_shape_differences_are_not_blanket_risks() {
        let first = definition(
            "a",
            "lib/Duplicate",
            Vec::new(),
            vec![member("privateHelperA", "()V", false)],
            Vec::new(),
        );
        let second = definition(
            "b",
            "lib/Duplicate",
            Vec::new(),
            vec![member("privateHelperB", "(I)V", false)],
            Vec::new(),
        );
        let scanned = scanned_with_classes(BTreeMap::from([(
            "lib/Duplicate".to_string(),
            vec![first, second],
        )]));

        assert!(binary_shape_risks(&scanned).is_empty());
    }

    #[test]
    fn hard_member_resolution_walks_superclasses_and_default_interfaces() {
        let inherited = member("inherited", "()V", false);
        let default_method = member("defaultMethod", "()V", false);
        let parent = definition(
            "runtime",
            "api/Parent",
            Vec::new(),
            vec![inherited.clone()],
            Vec::new(),
        );
        let interface = definition(
            "runtime",
            "api/DefaultInterface",
            Vec::new(),
            vec![default_method.clone()],
            Vec::new(),
        );
        let mut child = definition(
            "mod",
            "api/Child",
            vec!["api/DefaultInterface"],
            Vec::new(),
            Vec::new(),
        );
        child.super_name = Some("api/Parent".to_string());
        let universe = ClassUniverse {
            classes: BTreeMap::from([
                ("api/Parent".to_string(), vec![parent]),
                ("api/DefaultInterface".to_string(), vec![interface]),
                ("api/Child".to_string(), vec![child]),
            ]),
        };
        let child = &universe.classes["api/Child"][0];

        assert!(universe.definition_resolves_member(child, &inherited));
        assert!(universe.definition_resolves_member(child, &default_method));
    }

    #[test]
    fn risk_reasons_with_different_confidence_remain_independent() {
        let target = instruction_target(7);
        let risks = analyze_effects(&[
            effect(
                "a",
                mutation(MutationKind::RedirectOperation, target.clone()),
                Vec::new(),
            ),
            effect(
                "b",
                mutation(MutationKind::RedirectOperation, target),
                Vec::new(),
            ),
        ]);
        let exact = risks[0].clone();
        let mut lower_confidence = exact.clone();
        lower_confidence.confidence = Confidence::Low;
        lower_confidence.risk_index = risk_index(
            lower_confidence.severity,
            lower_confidence.confidence,
            lower_confidence.activation,
        );

        let coalesced = coalesce_risks(vec![exact, lower_confidence]);

        assert_eq!(coalesced.len(), 2);
        assert_eq!(
            coalesced
                .iter()
                .map(|risk| risk.confidence)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([Confidence::Low, Confidence::Exact])
        );
    }

    #[test]
    fn duplicate_class_shadowing_reports_hard_reference_staticness_mismatch() {
        let hard_reference = member("run", "()V", false);
        let caller = definition(
            "caller",
            "example/Caller",
            Vec::new(),
            Vec::new(),
            vec![hard_reference.clone()],
        );
        let compatible = definition(
            "provider-good",
            "lib/Duplicate",
            Vec::new(),
            vec![hard_reference],
            Vec::new(),
        );
        let incompatible = definition(
            "provider-bad",
            "lib/Duplicate",
            Vec::new(),
            vec![member("run", "()V", true)],
            Vec::new(),
        );
        let scanned = scanned_with_classes(BTreeMap::from([
            ("example/Caller".to_string(), vec![caller]),
            ("lib/Duplicate".to_string(), vec![compatible, incompatible]),
        ]));

        let risks = binary_shape_risks(&scanned);

        assert!(risks.iter().any(|risk| {
            risk.rule == "duplicate_class_reference_invalidation"
                && risk.left_artifact == "caller"
                && risk.right_artifact == "provider-bad"
                && risk
                    .target
                    .member
                    .as_ref()
                    .is_some_and(|member| member.is_static == Some(false))
                && risk.activation == Activation::Conditional
        }));
    }

    fn member(name: &str, descriptor: &str, is_static: bool) -> MemberReference {
        MemberReference {
            owner: "lib/Duplicate".to_string(),
            name: name.to_string(),
            descriptor: descriptor.to_string(),
            kind: MemberKind::Method,
            is_static: Some(is_static),
        }
    }

    fn definition(
        artifact_id: &str,
        name: &str,
        interfaces: Vec<&str>,
        methods: Vec<MemberReference>,
        hard_references: Vec<MemberReference>,
    ) -> ClassDefinition {
        ClassDefinition {
            definition_id: None,
            artifact_id: artifact_id.to_string(),
            is_mod: true,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: interfaces.into_iter().map(str::to_string).collect(),
            fields: Vec::new(),
            methods,
            hard_references,
        }
    }

    fn scanned_with_classes(classes: BTreeMap<String, Vec<ClassDefinition>>) -> ScannedArtifacts {
        ScannedArtifacts {
            artifact_reports: Vec::new(),
            artifacts: Vec::new(),
            universe: ClassUniverse { classes },
            limits: AnalysisLimits::default(),
            coverage: Coverage::default(),
            warnings: Vec::new(),
        }
    }
}
