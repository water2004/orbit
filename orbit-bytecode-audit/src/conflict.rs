use std::collections::{BTreeMap, HashMap};

use crate::jar::ScannedArtifacts;
use crate::model::{
    Activation, AuditReport, AuditRequest, Confidence, Effect, Mutation, MutationKind,
    OrderAnalysis, Precision, REPORT_SCHEMA_VERSION, Readiness, RequirementKind, Risk, Severity,
    Target,
};

pub(crate) fn build_report(
    request: &AuditRequest,
    readiness: Readiness,
    scanned: ScannedArtifacts,
    effects: Vec<Effect>,
) -> AuditReport {
    let mut risks = binary_shape_risks(&scanned);
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
    risks.extend(analyze_effects(&effects));
    risks = coalesce_risks(risks);
    risks.sort_by(|left, right| {
        right
            .risk_index
            .cmp(&left.risk_index)
            .then_with(|| left.left_artifact.cmp(&right.left_artifact))
            .then_with(|| left.right_artifact.cmp(&right.right_artifact))
            .then_with(|| left.rule.cmp(&right.rule))
    });
    AuditReport {
        schema_version: REPORT_SCHEMA_VERSION.to_string(),
        environment: request.environment.clone(),
        readiness,
        artifacts: scanned.artifact_reports,
        risks,
        coverage,
        warnings: scanned.warnings,
    }
}

fn coalesce_risks(risks: Vec<Risk>) -> Vec<Risk> {
    let mut grouped = BTreeMap::<String, Risk>::new();
    for risk in risks {
        let key = format!(
            "{}|{}|{}|{}|{:?}|{}",
            risk.left_artifact,
            risk.right_artifact,
            target_identity(&risk.target),
            risk.rule,
            risk.order,
            risk.target
                .instruction
                .as_ref()
                .map_or(u32::MAX, |instruction| instruction.stable_id)
        );
        match grouped.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(risk);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                existing.severity = existing.severity.max(risk.severity);
                existing.confidence = existing.confidence.max(risk.confidence);
                existing.activation = combine_activation(existing.activation, risk.activation);
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
                existing.risk_index =
                    risk_index(existing.severity, existing.confidence, existing.activation);
            }
        }
    }
    grouped.into_values().collect()
}

fn target_identity(target: &Target) -> String {
    target.member.as_ref().map_or_else(
        || target.class.clone(),
        |member| {
            format!(
                "{}#{}{}:{:?}",
                target.class, member.name, member.descriptor, member.kind
            )
        },
    )
}

fn evidence_identity(evidence: &crate::model::Evidence) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        evidence.artifact_id,
        evidence.class,
        evidence.method.as_deref().unwrap_or_default(),
        evidence.annotation.as_deref().unwrap_or_default(),
        evidence
            .instruction
            .as_ref()
            .map_or(u32::MAX, |instruction| instruction.stable_id),
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
                if let Some(risk) = compare(left, right) {
                    risks.push(risk);
                }
            }
        }
    }
    risks
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

fn compare(left: &Effect, right: &Effect) -> Option<Risk> {
    let mut selected: Option<ConflictMatch> = None;
    for left_mutation in &left.mutations {
        for right_mutation in &right.mutations {
            select_stronger(&mut selected, write_write(left_mutation, right_mutation));
        }
        for requirement in &right.requirements {
            select_stronger(
                &mut selected,
                write_shape(left, left_mutation, right, requirement),
            );
        }
    }
    for right_mutation in &right.mutations {
        for requirement in &left.requirements {
            if let Some(mut candidate) = write_shape(right, right_mutation, left, requirement) {
                candidate.order = reverse_order(candidate.order);
                select_stronger(&mut selected, Some(candidate));
            }
        }
    }
    if selected.is_none()
        && (left
            .mutations
            .iter()
            .any(|mutation| mutation.kind == MutationKind::UnknownMethod)
            || right
                .mutations
                .iter()
                .any(|mutation| mutation.kind == MutationKind::UnknownMethod))
        && left.target.member.is_some()
        && right.target.member.is_some()
    {
        selected = Some(ConflictMatch {
            rule: "unknown_method_overlap",
            reason: "An unknown method-level rewrite overlaps another modification of the same method.",
            severity: Severity::High,
            order: OrderAnalysis::Unknown,
        });
    }
    if selected.is_none()
        && (left
            .mutations
            .iter()
            .any(|mutation| mutation.kind == MutationKind::UnknownClass)
            || right
                .mutations
                .iter()
                .any(|mutation| mutation.kind == MutationKind::UnknownClass))
    {
        selected = Some(ConflictMatch {
            rule: "unknown_class_overlap",
            reason: "Both effects overlap the same class, but at least one modification is only known at class precision.",
            severity: Severity::Low,
            order: OrderAnalysis::Unknown,
        });
    }
    let matched = selected?;
    let confidence = left.confidence.min(right.confidence);
    let activation = combine_activation(left.activation, right.activation);
    let risk_index = risk_index(matched.severity, confidence, activation);
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
    Some(Risk {
        left_artifact,
        right_artifact,
        target: more_precise_target(&left.target, &right.target),
        rule: matched.rule.to_string(),
        reason: matched.reason.to_string(),
        left_mutations,
        right_mutations,
        evidence,
        order: matched.order,
        severity: matched.severity,
        confidence,
        risk_index,
        activation,
    })
}

#[derive(Clone, Copy)]
struct ConflictMatch {
    rule: &'static str,
    reason: &'static str,
    severity: Severity,
    order: OrderAnalysis,
}

fn select_stronger(current: &mut Option<ConflictMatch>, candidate: Option<ConflictMatch>) {
    let Some(candidate) = candidate else {
        return;
    };
    if current.is_none_or(|value| candidate.severity > value.severity) {
        *current = Some(candidate);
    }
}

fn write_write(left: &Mutation, right: &Mutation) -> Option<ConflictMatch> {
    use MutationKind::{
        AddField, AddMethod, ChangeAccess, ChangeInterfaces, ChangeSuperclass, RedirectOperation,
        RemoveField, RemoveInstruction, RemoveMethod, ReplaceInstruction, ReplaceMethodBody,
        WrapOperation,
    };
    if left.kind == ReplaceMethodBody && right.kind == ReplaceMethodBody {
        return Some(ConflictMatch {
            rule: "method_body_write_write",
            reason: "Both artifacts replace the complete body of the same method.",
            severity: Severity::Critical,
            order: OrderAnalysis::Exclusive,
        });
    }
    if same_instruction(&left.target, &right.target)
        && (left.exclusive || right.exclusive)
        && matches!(
            left.kind,
            RedirectOperation | WrapOperation | RemoveInstruction | ReplaceInstruction
        )
        && matches!(
            right.kind,
            RedirectOperation | WrapOperation | RemoveInstruction | ReplaceInstruction
        )
    {
        return Some(ConflictMatch {
            rule: "exclusive_instruction_write",
            reason: "Two exclusive modifications target the same original instruction.",
            severity: Severity::Critical,
            order: OrderAnalysis::Exclusive,
        });
    }
    if left.kind == ChangeInterfaces && right.kind == ChangeInterfaces {
        return (left.exclusive || right.exclusive).then_some(ConflictMatch {
            rule: "interface_shape_replacement",
            reason: "At least one artifact replaces the interface set while another artifact changes it.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
        });
    }
    if matches!(
        (left.kind, right.kind),
        (AddField, AddField)
            | (AddMethod, AddMethod)
            | (RemoveField, RemoveField)
            | (RemoveMethod, RemoveMethod)
            | (ChangeSuperclass, ChangeSuperclass)
    ) {
        return Some(ConflictMatch {
            rule: "member_shape_write_write",
            reason: "Both artifacts change the same class or member shape.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
        });
    }
    if matches!(
        (left.kind, right.kind),
        (AddMethod | ReplaceMethodBody | ChangeAccess, RemoveMethod)
            | (RemoveMethod, AddMethod | ReplaceMethodBody | ChangeAccess)
            | (AddField | ChangeAccess, RemoveField)
            | (RemoveField, AddField | ChangeAccess)
            | (ReplaceMethodBody, AddMethod)
            | (AddMethod, ReplaceMethodBody)
            | (ChangeAccess, ChangeAccess)
    ) {
        return Some(ConflictMatch {
            rule: "member_shape_write_write",
            reason: "One artifact removes, replaces, adds, or changes access to a member that the other artifact also changes.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
        });
    }
    if matches!(
        (left.kind, right.kind),
        (RemoveInstruction, ReplaceInstruction) | (ReplaceInstruction, RemoveInstruction)
    ) && same_instruction(&left.target, &right.target)
    {
        return Some(ConflictMatch {
            rule: "remove_replace_instruction",
            reason: "One artifact removes an instruction that the other replaces.",
            severity: Severity::Critical,
            order: OrderAnalysis::Exclusive,
        });
    }
    None
}

fn write_shape(
    writer: &Effect,
    mutation: &Mutation,
    reader: &Effect,
    requirement: &crate::model::ShapeRequirement,
) -> Option<ConflictMatch> {
    use MutationKind::{
        ChangeControlFlow, ChangeLocalLayout, InsertInstructions, ModifyConstant, ModifyLocal,
        RedirectOperation, RemoveField, RemoveInstruction, RemoveMethod, ReplaceInstruction,
        ReplaceMethodBody, WrapOperation,
    };
    use RequirementKind::{
        Cardinality, ControlFlow, InstructionExists, LocalLayout, MemberExists, SliceBoundary,
    };
    let kind = requirement.kind;
    if mixinextras_chainable(writer) && mixinextras_chainable(reader) {
        return None;
    }
    if mutation.kind == ReplaceMethodBody
        && matches!(
            kind,
            InstructionExists | SliceBoundary | Cardinality | LocalLayout | ControlFlow
        )
    {
        return Some(ConflictMatch {
            rule: "overwrite_invalidates_internal_shape",
            reason: "A complete method replacement may remove an anchor required by the other artifact.",
            severity: Severity::High,
            order: OrderAnalysis::AnchorInvalidated,
        });
    }
    if matches!(
        mutation.kind,
        RemoveInstruction | ReplaceInstruction | RedirectOperation | WrapOperation | ModifyConstant
    ) && matches!(kind, InstructionExists | SliceBoundary | Cardinality)
        && requirements_overlap_instruction(mutation, requirement)
    {
        return Some(ConflictMatch {
            rule: "instruction_write_invalidates_selector",
            reason: "An instruction mutation may invalidate the other artifact's selector or slice.",
            severity: Severity::High,
            order: if kind == Cardinality {
                OrderAnalysis::CardinalityInvalidated
            } else {
                OrderAnalysis::AnchorInvalidated
            },
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
        });
    }
    if mutation.kind == InsertInstructions
        && (requirement.ordinal.is_some() || kind == Cardinality)
        && matches!(
            writer.mechanism,
            crate::model::Mechanism::ModLauncherTransformer | crate::model::Mechanism::JavaCoremod
        )
        && instruction_patterns_overlap(
            mutation.target.instruction.as_ref(),
            requirement.target.instruction.as_ref(),
        )
    {
        return Some(ConflictMatch {
            rule: "insertion_changes_selector_cardinality",
            reason: "Inserted instructions may change ordinal or cardinality selection.",
            severity: Severity::Medium,
            order: if requirement.ordinal.is_some() {
                OrderAnalysis::OrdinalChanged
            } else {
                OrderAnalysis::CardinalityInvalidated
            },
        });
    }
    if matches!(mutation.kind, ChangeLocalLayout | ModifyLocal) && kind == LocalLayout {
        return Some(ConflictMatch {
            rule: "local_layout_dependency",
            reason: "A local-variable modification overlaps a local-layout requirement.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
        });
    }
    if mutation.kind == ChangeControlFlow && kind == ControlFlow {
        return Some(ConflictMatch {
            rule: "control_flow_dependency",
            reason: "A control-flow change overlaps a RETURN, TAIL, or control-flow requirement.",
            severity: Severity::High,
            order: OrderAnalysis::Structural,
        });
    }
    None
}

fn mixinextras_chainable(effect: &Effect) -> bool {
    effect.mechanism == crate::model::Mechanism::MixinExtras
        && effect.mutations.iter().all(|mutation| {
            !mutation.exclusive
                && matches!(
                    mutation.kind,
                    MutationKind::WrapOperation
                        | MutationKind::ReplaceInstruction
                        | MutationKind::ChangeControlFlow
                )
        })
}

fn requirements_overlap_instruction(
    mutation: &Mutation,
    requirement: &crate::model::ShapeRequirement,
) -> bool {
    match (
        mutation.target.instruction.as_ref(),
        requirement.target.instruction.as_ref(),
    ) {
        (Some(left), Some(right)) => {
            left.stable_id == right.stable_id
                || instruction_patterns_overlap(Some(left), Some(right))
        }
        _ => false,
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

fn same_instruction(left: &Target, right: &Target) -> bool {
    match (&left.instruction, &right.instruction) {
        (Some(left), Some(right)) => left.stable_id == right.stable_id,
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
    let activation_score = match activation {
        Activation::Definite => 100_u16,
        Activation::Conditional => 85,
        Activation::Candidate => 70,
        Activation::Unknown => 50,
    };
    let weighted = u16::from(severity.score()) * 55
        + u16::from(confidence.score()) * 30
        + activation_score * 15;
    u8::try_from((weighted / 100).min(100)).unwrap_or(100)
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
    for definitions in scanned.universe.classes.values() {
        let mod_definitions = definitions
            .iter()
            .filter(|definition| definition.is_mod)
            .collect::<Vec<_>>();
        for (index, left) in mod_definitions.iter().enumerate() {
            for right in &mod_definitions[index + 1..] {
                if left.artifact_id == right.artifact_id {
                    continue;
                }
                let left_shape = left.member_shape();
                let right_shape = right.member_shape();
                let mut left_interfaces = left.interfaces.clone();
                let mut right_interfaces = right.interfaces.clone();
                left_interfaces.sort();
                right_interfaces.sort();
                if left_shape == right_shape
                    && left.super_name == right.super_name
                    && left_interfaces == right_interfaces
                    && left.is_interface == right.is_interface
                {
                    continue;
                }
                let confidence = Confidence::Exact;
                risks.push(Risk {
                    left_artifact: left.artifact_id.clone(),
                    right_artifact: right.artifact_id.clone(),
                    target: Target::class(&left.name),
                    rule: "duplicate_class_shape".to_string(),
                    reason: "Different Mod JARs provide incompatible shapes for the same class."
                        .to_string(),
                    left_mutations: vec![MutationKind::UnknownClass],
                    right_mutations: vec![MutationKind::UnknownClass],
                    evidence: Vec::new(),
                    order: OrderAnalysis::Structural,
                    severity: Severity::High,
                    confidence,
                    risk_index: risk_index(Severity::High, confidence, Activation::Definite),
                    activation: Activation::Definite,
                });
            }
        }
    }

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
                .filter(|candidate| candidate.has_member(reference))
                .count();
            if valid == 0 || valid == candidates.len() {
                continue;
            }
            for candidate in candidates
                .iter()
                .filter(|candidate| candidate.is_mod && !candidate.has_member(reference))
            {
                if candidate.artifact_id == definitions.artifact_id {
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
                    risk_index: risk_index(Severity::High, confidence, Activation::Definite),
                    activation: Activation::Definite,
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
        Activation, AnalysisLimits, Confidence, Coverage, Effect, Evidence, InstructionReference,
        Mechanism, MemberKind, MemberReference, Mutation, MutationKind, Precision, RequirementKind,
        ShapeRequirement, Target,
    };

    use super::*;

    fn instruction_target(id: u32) -> Target {
        let mut target = Target::method("game/Foo", "tick", "()V");
        target.instruction = Some(InstructionReference {
            stable_id: id,
            original_offset: Some(id),
            opcode: 182,
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
            mutations: vec![mutation],
            evidence: vec![Evidence {
                artifact_id: artifact.to_string(),
                class: format!("{artifact}/Mixin"),
                method: None,
                annotation: None,
                instruction: None,
                detail: "test".to_string(),
            }],
            precision: Precision::Instruction,
            confidence: Confidence::Exact,
            activation: Activation::Candidate,
            priority: None,
        }
    }

    #[test]
    fn two_redirects_on_one_instruction_are_critical() {
        let target = instruction_target(7);
        let effects = ["a", "b"].map(|artifact| {
            effect(
                artifact,
                Mutation {
                    kind: MutationKind::RedirectOperation,
                    target: target.clone(),
                    exclusive: true,
                },
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
    fn mixinextras_wrappers_chain_but_redirect_remains_exclusive() {
        let target = instruction_target(7);
        let requirement = ShapeRequirement {
            kind: RequirementKind::InstructionExists,
            target: target.clone(),
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        };
        let mut first = effect(
            "first",
            Mutation {
                kind: MutationKind::WrapOperation,
                target: target.clone(),
                exclusive: false,
            },
            vec![requirement.clone()],
        );
        first.mechanism = Mechanism::MixinExtras;
        let mut second = effect(
            "second",
            Mutation {
                kind: MutationKind::WrapOperation,
                target: target.clone(),
                exclusive: false,
            },
            vec![requirement.clone()],
        );
        second.mechanism = Mechanism::MixinExtras;
        assert!(analyze_effects(&[first.clone(), second]).is_empty());

        let redirect = effect(
            "redirect",
            Mutation {
                kind: MutationKind::RedirectOperation,
                target,
                exclusive: true,
            },
            vec![requirement],
        );
        let risks = analyze_effects(&[first, redirect]);
        assert_eq!(risks[0].rule, "exclusive_instruction_write");
        assert_eq!(risks[0].severity, Severity::Critical);
    }

    #[test]
    fn overwrite_invalidates_an_internal_inject_anchor() {
        let target = instruction_target(4);
        let overwrite = effect(
            "overwrite",
            Mutation {
                kind: MutationKind::ReplaceMethodBody,
                target: Target::method("game/Foo", "tick", "()V"),
                exclusive: true,
            },
            Vec::new(),
        );
        let inject = effect(
            "inject",
            Mutation {
                kind: MutationKind::InsertInstructions,
                target: target.clone(),
                exclusive: false,
            },
            vec![ShapeRequirement {
                kind: RequirementKind::InstructionExists,
                target,
                minimum_matches: Some(1),
                maximum_matches: None,
                ordinal: None,
                slice: None,
            }],
        );

        let risks = analyze_effects(&[overwrite, inject]);

        assert_eq!(risks[0].rule, "overwrite_invalidates_internal_shape");
        assert_eq!(risks[0].order, OrderAnalysis::AnchorInvalidated);
    }

    #[test]
    fn member_removal_invalidates_a_required_target() {
        let method = Target::method("game/Foo", "tick", "()V");
        let removal = effect(
            "removal",
            Mutation {
                kind: MutationKind::RemoveMethod,
                target: method.clone(),
                exclusive: true,
            },
            Vec::new(),
        );
        let overwrite = effect(
            "overwrite",
            Mutation {
                kind: MutationKind::ReplaceMethodBody,
                target: method.clone(),
                exclusive: true,
            },
            vec![ShapeRequirement {
                kind: RequirementKind::MemberExists,
                target: method,
                minimum_matches: Some(1),
                maximum_matches: None,
                ordinal: None,
                slice: None,
            }],
        );

        let risks = analyze_effects(&[removal, overwrite]);

        assert_eq!(risks[0].rule, "member_shape_write_write");
        assert_eq!(risks[0].severity, Severity::High);
    }

    #[test]
    fn insertion_reports_ordinal_drift() {
        let target = instruction_target(2);
        let mut insertion = effect(
            "insertion",
            Mutation {
                kind: MutationKind::InsertInstructions,
                target: target.clone(),
                exclusive: false,
            },
            Vec::new(),
        );
        insertion.mechanism = Mechanism::ModLauncherTransformer;
        let ordinal = effect(
            "ordinal",
            Mutation {
                kind: MutationKind::ModifyArgument,
                target: target.clone(),
                exclusive: false,
            },
            vec![ShapeRequirement {
                kind: RequirementKind::InstructionExists,
                target,
                minimum_matches: Some(1),
                maximum_matches: None,
                ordinal: Some(1),
                slice: None,
            }],
        );

        let risks = analyze_effects(&[insertion, ordinal]);

        assert_eq!(risks[0].order, OrderAnalysis::OrdinalChanged);
        assert_eq!(risks[0].severity, Severity::Medium);
    }

    #[test]
    fn local_layout_and_cardinality_conflicts_are_distinct() {
        let method = Target::method("game/Foo", "tick", "()V");
        let local = effect(
            "local-writer",
            Mutation {
                kind: MutationKind::ChangeLocalLayout,
                target: method.clone(),
                exclusive: false,
            },
            Vec::new(),
        );
        let capture = effect(
            "capture",
            Mutation {
                kind: MutationKind::InsertInstructions,
                target: method.clone(),
                exclusive: false,
            },
            vec![ShapeRequirement {
                kind: RequirementKind::LocalLayout,
                target: method,
                minimum_matches: None,
                maximum_matches: None,
                ordinal: None,
                slice: None,
            }],
        );

        let risks = analyze_effects(&[local, capture]);

        assert_eq!(risks[0].rule, "local_layout_dependency");
        assert_eq!(risks[0].severity, Severity::High);
    }

    #[test]
    fn removal_invalidates_slice_and_allow_cardinality() {
        let target = instruction_target(3);
        let removal = effect(
            "removal",
            Mutation {
                kind: MutationKind::RemoveInstruction,
                target: target.clone(),
                exclusive: true,
            },
            Vec::new(),
        );
        let slice = effect(
            "slice",
            Mutation {
                kind: MutationKind::InsertInstructions,
                target: target.clone(),
                exclusive: false,
            },
            vec![
                ShapeRequirement {
                    kind: RequirementKind::SliceBoundary,
                    target: target.clone(),
                    minimum_matches: Some(1),
                    maximum_matches: None,
                    ordinal: None,
                    slice: Some("region".to_string()),
                },
                ShapeRequirement {
                    kind: RequirementKind::Cardinality,
                    target,
                    minimum_matches: Some(1),
                    maximum_matches: Some(1),
                    ordinal: None,
                    slice: Some("region".to_string()),
                },
            ],
        );

        let risks = analyze_effects(&[removal, slice]);

        assert_eq!(risks.len(), 1);
        assert_eq!(risks[0].order, OrderAnalysis::AnchorInvalidated);
        assert_eq!(risks[0].severity, Severity::High);
    }

    #[test]
    fn control_flow_change_conflicts_with_return_shape() {
        let method = Target::method("game/Foo", "tick", "()V");
        let writer = effect(
            "writer",
            Mutation {
                kind: MutationKind::ChangeControlFlow,
                target: method.clone(),
                exclusive: false,
            },
            Vec::new(),
        );
        let reader = effect(
            "reader",
            Mutation {
                kind: MutationKind::InsertInstructions,
                target: method.clone(),
                exclusive: false,
            },
            vec![ShapeRequirement {
                kind: RequirementKind::ControlFlow,
                target: method,
                minimum_matches: Some(1),
                maximum_matches: None,
                ordinal: None,
                slice: None,
            }],
        );

        let risks = analyze_effects(&[writer, reader]);

        assert_eq!(risks[0].rule, "control_flow_dependency");
        assert_eq!(risks[0].order, OrderAnalysis::Structural);
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
            artifact_id: artifact_id.to_string(),
            is_mod: true,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: interfaces.into_iter().map(str::to_string).collect(),
            is_interface: false,
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
