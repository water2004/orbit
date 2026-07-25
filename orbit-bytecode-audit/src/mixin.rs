use std::collections::{BTreeMap, BTreeSet};

use crate::classfile::{
    AnnotationValue, InstructionKind, ParsedAnnotation, ParsedClass, ParsedInstruction,
    ParsedMethod,
};
use crate::jar::{ParsedArtifact, ScannedArtifacts};
use crate::model::{
    Activation, Confidence, Effect, Evidence, InjectionGroupConstraint, InjectionQuery, Mechanism,
    MemberKind, MemberReference, Mutation, MutationKind, Precision, RequirementKind,
    ShapeRequirement, SoftReferenceResolution, Target, Warning, WarningKind,
};

const MIXIN: &str = "Lorg/spongepowered/asm/mixin/Mixin;";
const SHADOW: &str = "Lorg/spongepowered/asm/mixin/Shadow;";
const OVERWRITE: &str = "Lorg/spongepowered/asm/mixin/Overwrite;";
const UNIQUE: &str = "Lorg/spongepowered/asm/mixin/Unique;";
const ACCESSOR: &str = "Lorg/spongepowered/asm/mixin/gen/Accessor;";
const INVOKER: &str = "Lorg/spongepowered/asm/mixin/gen/Invoker;";

#[cfg(test)]
pub(crate) fn analyze(scanned: &mut ScannedArtifacts) -> Vec<Effect> {
    analyze_with_progress(scanned, None)
}

pub(crate) fn analyze_with_progress(
    scanned: &mut ScannedArtifacts,
    progress: Option<&crate::progress::AuditProgressReporter>,
) -> Vec<Effect> {
    use crate::progress::{AuditProgressEvent, AuditProgressStage, emit};

    let total = scanned
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == crate::model::ArtifactKind::Mod)
        .flat_map(|artifact| &artifact.classes)
        .filter(|class| annotation(&class.annotations, MIXIN).is_some())
        .count();
    emit(
        progress,
        AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::AnalyzeMixins,
            total: Some(total),
        },
    );
    let mut effects = Vec::new();
    let mut warnings = Vec::new();
    let mut completed = 0;
    let mod_indexes = scanned
        .artifacts
        .iter()
        .enumerate()
        .filter_map(|(index, artifact)| {
            (artifact.kind == crate::model::ArtifactKind::Mod).then_some(index)
        })
        .collect::<Vec<_>>();
    for index in mod_indexes {
        let artifact = &scanned.artifacts[index];
        for mixin in &artifact.classes {
            let Some(mixin_annotation) = annotation(&mixin.annotations, MIXIN) else {
                continue;
            };
            scanned.coverage.mixins_discovered += 1;
            let target_classes = mixin_targets(mixin_annotation);
            if target_classes.is_empty() {
                warnings.push(warning(
                    artifact,
                    &mixin.name,
                    WarningKind::Other,
                    "@Mixin contains no recoverable target class",
                ));
                completed += 1;
                emit(
                    progress,
                    AuditProgressEvent::Advanced {
                        stage: AuditProgressStage::AnalyzeMixins,
                        completed,
                        total: Some(total),
                    },
                );
                continue;
            }
            let priority = mixin_annotation
                .value("priority")
                .and_then(AnnotationValue::integer)
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(1000);
            analyze_mixin_structure(artifact, mixin, &target_classes, priority, &mut effects);
            for method in &mixin.methods {
                for injector in method
                    .annotations
                    .iter()
                    .filter(|annotation| injector_kind(&annotation.descriptor).is_some())
                {
                    analyze_injector(
                        scanned,
                        artifact,
                        mixin,
                        method,
                        injector,
                        &target_classes,
                        priority,
                        &mut effects,
                        &mut warnings,
                    );
                }
            }
            completed += 1;
            emit(
                progress,
                AuditProgressEvent::Advanced {
                    stage: AuditProgressStage::AnalyzeMixins,
                    completed,
                    total: Some(total),
                },
            );
        }
    }
    finalize_injection_groups(&mut effects);
    scanned.warnings.extend(warnings);
    emit(
        progress,
        AuditProgressEvent::StageFinished {
            stage: AuditProgressStage::AnalyzeMixins,
            completed,
        },
    );
    effects
}

fn analyze_mixin_structure(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    targets: &[String],
    priority: i32,
    effects: &mut Vec<Effect>,
) {
    let mixin_is_unique = annotation(&mixin.annotations, UNIQUE).is_some();
    for target_class in targets {
        for field in &mixin.fields {
            let target = Target {
                class: target_class.clone(),
                member: Some(MemberReference {
                    owner: target_class.clone(),
                    name: field.name.clone(),
                    descriptor: field.descriptor.clone(),
                    kind: MemberKind::Field,
                    is_static: Some(field.is_static),
                }),
                instruction: None,
            };
            if annotation(&field.annotations, SHADOW).is_some() {
                effects.push(shape_only_effect(
                    artifact,
                    mixin,
                    target,
                    RequirementKind::MemberExists,
                    SHADOW,
                    priority,
                ));
            } else if !(annotation(&field.annotations, UNIQUE).is_some()
                && field.is_private_or_protected)
            {
                // MixinPreProcessorStandard renames private/protected
                // @Unique fields before they are merged into the target.
                effects.push(structural_effect(
                    artifact,
                    mixin,
                    target,
                    MutationKind::AddField,
                    if annotation(&field.annotations, UNIQUE).is_some() {
                        UNIQUE
                    } else {
                        "mixin field merge"
                    },
                    priority,
                ));
            }
        }
        for method in &mixin.methods {
            if method.name == "<init>" || method.name == "<clinit>" {
                continue;
            }
            if method
                .annotations
                .iter()
                .any(|annotation| injector_kind(&annotation.descriptor).is_some())
            {
                // Injector handlers are copied under Mixin-generated unique
                // names; their declared handler name is not a target member.
                continue;
            }
            let target = Target::method(
                target_class.clone(),
                method.name.clone(),
                method.descriptor.clone(),
            );
            if annotation(&method.annotations, SHADOW).is_some() {
                effects.push(shape_only_effect(
                    artifact,
                    mixin,
                    target,
                    RequirementKind::MemberExists,
                    SHADOW,
                    priority,
                ));
            } else if annotation(&method.annotations, OVERWRITE).is_some() {
                effects.push(structural_effect(
                    artifact,
                    mixin,
                    target,
                    MutationKind::ReplaceMethodBody,
                    OVERWRITE,
                    priority,
                ));
            } else if let Some(accessor) = annotation(&method.annotations, ACCESSOR)
                .or_else(|| annotation(&method.annotations, INVOKER))
            {
                let kind = if accessor.descriptor == ACCESSOR {
                    MemberKind::Field
                } else {
                    MemberKind::Method
                };
                let explicit = accessor
                    .value("value")
                    .and_then(|value| value.strings().into_iter().next());
                let member_name = explicit.unwrap_or_else(|| derived_accessor_name(&method.name));
                let member = MemberReference {
                    owner: target_class.clone(),
                    name: member_name,
                    descriptor: if kind == MemberKind::Method {
                        method.descriptor.clone()
                    } else {
                        accessor_field_descriptor(&method.descriptor)
                    },
                    kind,
                    is_static: None,
                };
                effects.push(shape_only_effect(
                    artifact,
                    mixin,
                    Target {
                        class: target_class.clone(),
                        member: Some(member),
                        instruction: None,
                    },
                    RequirementKind::MemberExists,
                    &accessor.descriptor,
                    priority,
                ));
            } else if !((method.is_synthetic
                || annotation(&method.annotations, UNIQUE).is_some()
                || mixin_is_unique)
                && !method.is_public)
            {
                // MixinPreProcessorStandard gives synthetic and non-public
                // @Unique methods target-specific unique names. Public unique
                // methods can still be discarded or fail on a collision and
                // therefore remain structural effects.
                effects.push(structural_effect(
                    artifact,
                    mixin,
                    target,
                    MutationKind::AddMethod,
                    if annotation(&method.annotations, UNIQUE).is_some() {
                        UNIQUE
                    } else {
                        "mixin method merge"
                    },
                    priority,
                ));
            }
        }
        if !mixin.interfaces.is_empty() {
            effects.push(Effect {
                artifact_id: artifact.id.clone(),
                mechanism: Mechanism::Mixin,
                target: Target::class(target_class),
                requirements: vec![ShapeRequirement {
                    kind: RequirementKind::ClassExists,
                    target: Target::class(target_class),
                    precision: Precision::Class,
                    minimum_matches: Some(1),
                    maximum_matches: None,
                    ordinal: None,
                    slice: None,
                }],
                queries: Vec::new(),
                mutations: vec![Mutation::new(
                    MutationKind::AddInterfaces,
                    Target::class(target_class),
                    Precision::Class,
                )],
                evidence: vec![evidence(
                    artifact,
                    mixin,
                    None,
                    MIXIN,
                    format!("adds interfaces {}", mixin.interfaces.join(", ")),
                )],
                precision: Precision::Class,
                confidence: Confidence::High,
                activation: Activation::Candidate,
                priority: Some(priority),
            });
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn analyze_injector(
    scanned: &ScannedArtifacts,
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    handler: &ParsedMethod,
    injector: &ParsedAnnotation,
    mixin_targets: &[String],
    priority: i32,
    effects: &mut Vec<Effect>,
    warnings: &mut Vec<Warning>,
) {
    let Some((mutation_kind, mechanism)) = injector_kind(&injector.descriptor) else {
        return;
    };
    let raw_selectors = injector
        .value("method")
        .or_else(|| injector.value("target"))
        .map(selector_values)
        .unwrap_or_default();
    if raw_selectors.is_empty() {
        warnings.push(warning(
            artifact,
            &mixin.name,
            WarningKind::UnresolvedSoftReference,
            &format!("{} has no recoverable method selector", injector.descriptor),
        ));
        return;
    }
    let ats = injector
        .value("at")
        .map(AnnotationValue::annotations)
        .unwrap_or_default();
    let require = positive_u32(injector.value("require"));
    let expect = positive_u32(injector.value("expect"));
    let allow = positive_u32(injector.value("allow"));
    let group_annotation = annotation(
        &handler.annotations,
        "Lorg/spongepowered/asm/mixin/injection/Group;",
    );
    let group_name = group_annotation.and_then(|group| {
        group
            .value("name")
            .or_else(|| group.value("value"))
            .and_then(|value| value.strings().into_iter().next())
    });
    let group_minimum = group_annotation.and_then(|group| positive_u32(group.value("min")));
    let group_maximum = group_annotation.and_then(|group| positive_u32(group.value("max")));
    let locals = injector
        .value("locals")
        .is_some_and(|value| !enum_is(value, "NO_CAPTURE"));
    let cancellable = injector
        .value("cancellable")
        .and_then(AnnotationValue::boolean)
        .unwrap_or(false);
    let slices = injector
        .value("slice")
        .map(AnnotationValue::annotations)
        .unwrap_or_default();

    for target_class in mixin_targets {
        for raw_selector in &raw_selectors {
            let method_resolution = resolve_method_reference(
                scanned,
                artifact,
                &mixin.name,
                target_class,
                raw_selector,
            );
            warn_for_soft_reference(
                warnings,
                artifact,
                &mixin.name,
                "method selector",
                raw_selector,
                method_resolution.resolution,
            );
            if method_resolution.methods.is_empty() {
                let parsed = parse_selector(raw_selector);
                let method_target = Target::method(
                    target_class,
                    parsed.name,
                    parsed.descriptor.unwrap_or_default(),
                );
                effects.push(degraded_injector_effect(
                    artifact,
                    mixin,
                    handler,
                    injector,
                    method_target,
                    mutation_kind,
                    mechanism,
                    priority,
                    "method selector could not be resolved in the actual class universe",
                ));
                continue;
            }
            for (_, target_method) in &method_resolution.methods {
                let method_target = Target::method(
                    target_class,
                    target_method.name.clone(),
                    target_method.descriptor.clone(),
                );
                if ats.is_empty() {
                    if injector_simple_name(&injector.descriptor) == "ModifyConstant" {
                        let matches = match_modify_constants(target_method, injector, handler);
                        if matches.is_empty() {
                            effects.push(degraded_injector_effect(
                                artifact,
                                mixin,
                                handler,
                                injector,
                                method_target,
                                mutation_kind,
                                mechanism,
                                priority,
                                "ModifyConstant discriminator has no match in the original target method",
                            ));
                            continue;
                        }
                        let selected = matches
                            .into_iter()
                            .map(|instruction| instruction.reference.clone())
                            .collect::<Vec<_>>();
                        let first_instruction = selected.first().cloned();
                        let requirements = selected
                            .iter()
                            .map(|instruction| {
                                let mut target = method_target.clone();
                                target.instruction = Some(instruction.clone());
                                ShapeRequirement {
                                    kind: RequirementKind::InstructionExists,
                                    target,
                                    precision: Precision::Instruction,
                                    minimum_matches: Some(1),
                                    maximum_matches: None,
                                    ordinal: None,
                                    slice: None,
                                }
                            })
                            .collect::<Vec<_>>();
                        let mutations = selected
                            .iter()
                            .map(|instruction| {
                                let mut target = method_target.clone();
                                target.instruction = Some(instruction.clone());
                                Mutation::new(
                                    MutationKind::ModifyConstant,
                                    target,
                                    Precision::Instruction,
                                )
                            })
                            .collect::<Vec<_>>();
                        let query = InjectionQuery {
                            id: query_id(
                                artifact,
                                mixin,
                                handler,
                                target_class,
                                target_method,
                                "CONSTANT",
                            ),
                            selector_kind: "CONSTANT".to_string(),
                            method: method_target.clone(),
                            candidates: selected.clone(),
                            selected: selected.clone(),
                            minimum_matches: require,
                            maximum_matches: allow,
                            expected_matches: expect,
                            ordinal: None,
                            slice: None,
                            slice_start: None,
                            slice_end: None,
                            resolution: method_resolution.resolution,
                            group: injection_group_constraint(
                                artifact,
                                mixin,
                                handler,
                                group_name.as_deref(),
                                group_minimum,
                                group_maximum,
                            ),
                        };
                        effects.push(Effect {
                            artifact_id: artifact.id.clone(),
                            mechanism,
                            target: method_target.clone(),
                            requirements,
                            queries: vec![query],
                            mutations,
                            evidence: vec![{
                                let mut evidence = injector_evidence(
                                    artifact,
                                    mixin,
                                    handler,
                                    injector,
                                    mechanism,
                                    mutation_kind,
                                    raw_selector,
                                    None,
                                    None,
                                    method_resolution.resolution,
                                    &method_resolution.sources,
                                    &method_resolution.candidates,
                                    Precision::Instruction,
                                    "ModifyConstant resolved against the complete target query",
                                );
                                evidence.instruction = first_instruction;
                                evidence
                            }],
                            precision: Precision::Instruction,
                            confidence: confidence_for_resolution(method_resolution.resolution),
                            activation: Activation::Candidate,
                            priority: Some(priority),
                        });
                        continue;
                    }
                    if injector_simple_name(&injector.descriptor) == "WrapMethod" {
                        effects.push(Effect {
                            artifact_id: artifact.id.clone(),
                            mechanism,
                            target: method_target.clone(),
                            requirements: vec![ShapeRequirement {
                                kind: RequirementKind::MemberExists,
                                target: method_target.clone(),
                                precision: Precision::Method,
                                minimum_matches: Some(1),
                                maximum_matches: None,
                                ordinal: None,
                                slice: None,
                            }],
                            queries: Vec::new(),
                            mutations: vec![Mutation::new(
                                MutationKind::WrapOperation,
                                method_target.clone(),
                                Precision::Method,
                            )],
                            evidence: vec![injector_evidence(
                                artifact,
                                mixin,
                                handler,
                                injector,
                                mechanism,
                                mutation_kind,
                                raw_selector,
                                None,
                                None,
                                method_resolution.resolution,
                                &method_resolution.sources,
                                &method_resolution.candidates,
                                Precision::Method,
                                "WrapMethod resolved to the target method",
                            )],
                            precision: Precision::Method,
                            confidence: confidence_for_resolution(method_resolution.resolution),
                            activation: Activation::Candidate,
                            priority: Some(priority),
                        });
                        continue;
                    }
                    effects.push(degraded_injector_effect(
                        artifact,
                        mixin,
                        handler,
                        injector,
                        method_target,
                        mutation_kind,
                        mechanism,
                        priority,
                        "injector has no recoverable @At",
                    ));
                    continue;
                }

                let mut candidates = Vec::new();
                let mut selected = Vec::new();
                let mut requirements = Vec::new();
                let mut at_kinds = Vec::new();
                let mut at_targets = Vec::new();
                let mut refmap_sources = method_resolution.sources.clone();
                let mut target_candidates = method_resolution.candidates.clone();
                let mut slices_used = BTreeSet::new();
                let mut slice_ranges = BTreeSet::new();
                let mut ordinals = BTreeSet::new();
                let mut shifts = BTreeSet::new();
                let mut resolution = method_resolution.resolution;
                let mut degradation = None;

                for at in &ats {
                    let at_kind = at
                        .value("value")
                        .and_then(|value| value.strings().into_iter().next())
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    let support = injection_point_support(&at_kind);
                    if support != InjectionPointSupport::Supported {
                        let kind = if support == InjectionPointSupport::KnownUnsupported {
                            WarningKind::KnownUnsupportedInjectionPoint
                        } else {
                            WarningKind::CustomInjectionPoint
                        };
                        let label = if support == InjectionPointSupport::KnownUnsupported {
                            "known but unsupported"
                        } else {
                            "custom"
                        };
                        warnings.push(warning(
                            artifact,
                            &mixin.name,
                            kind,
                            &format!(
                                "{label} InjectionPoint '{at_kind}' kept the declared \
                                 {mutation_kind:?} semantics at method precision"
                            ),
                        ));
                        degradation = Some(format!(
                            "{label} InjectionPoint '{at_kind}' cannot be resolved precisely"
                        ));
                        break;
                    }

                    let active_slice = resolve_active_slice(
                        target_method,
                        &method_target,
                        &slices,
                        at,
                        artifact,
                        &mixin.name,
                        target_class,
                    );
                    if let Some(reason) = &active_slice.unresolved {
                        warnings.push(warning(
                            artifact,
                            &mixin.name,
                            WarningKind::Other,
                            &format!(
                                "{reason}; {}{} was degraded instead of searching the whole method",
                                target_method.name, target_method.descriptor
                            ),
                        ));
                        degradation = Some(reason.clone());
                        break;
                    }
                    resolution = weaker_resolution(resolution, active_slice.resolution);
                    requirements.extend(active_slice.requirements);
                    if let Some(slice) = &active_slice.id {
                        slices_used.insert(slice.clone());
                    }
                    if let Some((start, end)) = active_slice.range {
                        let start_id = target_method.instructions[start].reference.stable_id;
                        let end_id = target_method.instructions[end].reference.stable_id;
                        slice_ranges.insert((start_id, end_id));
                    }

                    let (at_candidates, at_sources, at_resolution) = resolve_at_reference(
                        target_method,
                        artifact,
                        &mixin.name,
                        target_class,
                        at,
                        &at_kind,
                        active_slice.range,
                    );
                    if let Some(raw_at_target) = at
                        .value("target")
                        .and_then(|value| value.strings().into_iter().next())
                    {
                        warn_for_soft_reference(
                            warnings,
                            artifact,
                            &mixin.name,
                            &format!("@At({at_kind}) target"),
                            &raw_at_target,
                            at_resolution,
                        );
                        at_targets.push(raw_at_target);
                    }
                    resolution = weaker_resolution(resolution, at_resolution);
                    refmap_sources.extend(at_sources);
                    target_candidates.extend(at_candidates.clone());
                    at_kinds.push(at_kind.clone());

                    let ordinal = at
                        .value("ordinal")
                        .and_then(AnnotationValue::integer)
                        .filter(|value| *value >= 0)
                        .and_then(|value| u32::try_from(value).ok());
                    if let Some(ordinal) = ordinal {
                        ordinals.insert(ordinal);
                    }
                    if let Some(shift) = render_shift(at) {
                        shifts.insert(shift);
                    }
                    let base_matches = match_instructions(
                        target_method,
                        at,
                        &at_kind,
                        &at_candidates,
                        active_slice.range,
                    );
                    candidates.extend(
                        base_matches
                            .iter()
                            .map(|instruction| instruction.reference.clone()),
                    );
                    let ordinal_matches = ordinal.map_or_else(
                        || base_matches.clone(),
                        |ordinal| {
                            base_matches
                                .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
                                .copied()
                                .into_iter()
                                .collect()
                        },
                    );
                    selected.extend(apply_shift(target_method, at, ordinal_matches));
                }

                if let Some(reason) = degradation {
                    effects.push(degraded_injector_effect(
                        artifact,
                        mixin,
                        handler,
                        injector,
                        method_target,
                        mutation_kind,
                        mechanism,
                        priority,
                        &reason,
                    ));
                    continue;
                }

                candidates.sort_by_key(|instruction| instruction.stable_id);
                candidates.dedup_by_key(|instruction| instruction.stable_id);
                selected.sort_by_key(|instruction| instruction.reference.stable_id);
                selected.dedup_by_key(|instruction| instruction.reference.stable_id);
                if selected.is_empty() {
                    effects.push(degraded_injector_effect(
                        artifact,
                        mixin,
                        handler,
                        injector,
                        method_target,
                        mutation_kind,
                        mechanism,
                        priority,
                        &format!(
                            "{} has no match inside its selected slice in the original target method",
                            at_kinds.join("+")
                        ),
                    ));
                    continue;
                }

                let selected_references = selected
                    .iter()
                    .map(|instruction| instruction.reference.clone())
                    .collect::<Vec<_>>();
                let first_instruction = selected_references.first().cloned();
                let mut mutations = Vec::new();
                for instruction in &selected_references {
                    let mut target = method_target.clone();
                    target.instruction = Some(instruction.clone());
                    requirements.push(ShapeRequirement {
                        kind: RequirementKind::InstructionExists,
                        target: target.clone(),
                        precision: Precision::Instruction,
                        minimum_matches: Some(1),
                        maximum_matches: None,
                        ordinal: ordinals.iter().next().copied(),
                        slice: slices_used.iter().next().cloned(),
                    });
                    mutations.push(Mutation::new(
                        mutation_kind,
                        target.clone(),
                        Precision::Instruction,
                    ));
                    if cancellable {
                        mutations.push(Mutation::new(
                            MutationKind::InsertConditionalReturn,
                            target,
                            Precision::Instruction,
                        ));
                    }
                }
                if locals {
                    requirements.push(ShapeRequirement {
                        kind: RequirementKind::LocalLayout,
                        target: method_target.clone(),
                        precision: Precision::Method,
                        minimum_matches: None,
                        maximum_matches: None,
                        ordinal: None,
                        slice: None,
                    });
                }
                refmap_sources.sort();
                refmap_sources.dedup();
                target_candidates.sort();
                target_candidates.dedup();
                at_kinds.sort();
                at_kinds.dedup();
                let selector_kind = at_kinds.join("+");
                let query = InjectionQuery {
                    id: query_id(
                        artifact,
                        mixin,
                        handler,
                        target_class,
                        target_method,
                        &selector_kind,
                    ),
                    selector_kind: selector_kind.clone(),
                    method: method_target.clone(),
                    candidates,
                    selected: selected_references.clone(),
                    minimum_matches: require,
                    maximum_matches: allow,
                    expected_matches: expect,
                    ordinal: (ordinals.len() == 1)
                        .then(|| ordinals.iter().next().copied())
                        .flatten(),
                    slice: (slices_used.len() == 1)
                        .then(|| slices_used.iter().next().cloned())
                        .flatten(),
                    slice_start: (slice_ranges.len() == 1)
                        .then(|| slice_ranges.iter().next().map(|(start, _)| *start))
                        .flatten(),
                    slice_end: (slice_ranges.len() == 1)
                        .then(|| slice_ranges.iter().next().map(|(_, end)| *end))
                        .flatten(),
                    resolution,
                    group: injection_group_constraint(
                        artifact,
                        mixin,
                        handler,
                        group_name.as_deref(),
                        group_minimum,
                        group_maximum,
                    ),
                };
                effects.push(Effect {
                    artifact_id: artifact.id.clone(),
                    mechanism,
                    target: method_target,
                    requirements,
                    queries: vec![query],
                    mutations,
                    evidence: vec![{
                        let mut evidence = injector_evidence(
                            artifact,
                            mixin,
                            handler,
                            injector,
                            mechanism,
                            mutation_kind,
                            raw_selector,
                            Some(&selector_kind),
                            at_targets.first().map(String::as_str),
                            resolution,
                            &refmap_sources,
                            &target_candidates,
                            Precision::Instruction,
                            "injector query resolved after slice, ordinal, and shift",
                        );
                        evidence.instruction = first_instruction;
                        evidence.slice = (slices_used.len() == 1)
                            .then(|| slices_used.iter().next().cloned())
                            .flatten();
                        evidence.ordinal = (ordinals.len() == 1)
                            .then(|| ordinals.iter().next().copied())
                            .flatten();
                        evidence.shift = (shifts.len() == 1)
                            .then(|| shifts.iter().next().cloned())
                            .flatten();
                        evidence
                    }],
                    precision: Precision::Instruction,
                    confidence: confidence_for_resolution(resolution),
                    activation: Activation::Candidate,
                    priority: Some(priority),
                });
            }
        }
    }
}

fn shape_only_effect(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    target: Target,
    requirement: RequirementKind,
    annotation: &str,
    priority: i32,
) -> Effect {
    Effect {
        artifact_id: artifact.id.clone(),
        mechanism: Mechanism::Mixin,
        target: target.clone(),
        requirements: vec![ShapeRequirement {
            kind: requirement,
            target,
            precision: Precision::Method,
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
        queries: Vec::new(),
        mutations: Vec::new(),
        evidence: vec![evidence(
            artifact,
            mixin,
            None,
            annotation,
            "member shape requirement".to_string(),
        )],
        precision: Precision::Method,
        confidence: Confidence::High,
        activation: Activation::Candidate,
        priority: Some(priority),
    }
}

fn structural_effect(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    target: Target,
    kind: MutationKind,
    annotation: &str,
    priority: i32,
) -> Effect {
    let (requirement, requirement_target) = if kind == MutationKind::ReplaceMethodBody {
        (RequirementKind::MemberExists, target.clone())
    } else {
        (RequirementKind::ClassExists, Target::class(&target.class))
    };
    Effect {
        artifact_id: artifact.id.clone(),
        mechanism: Mechanism::Mixin,
        target: target.clone(),
        requirements: vec![ShapeRequirement {
            kind: requirement,
            target: requirement_target,
            precision: if kind == MutationKind::ReplaceMethodBody {
                Precision::Method
            } else {
                Precision::Class
            },
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
        queries: Vec::new(),
        mutations: vec![Mutation::new(
            kind,
            target,
            if kind == MutationKind::ReplaceMethodBody {
                Precision::Method
            } else {
                Precision::Class
            },
        )],
        evidence: vec![evidence(
            artifact,
            mixin,
            None,
            annotation,
            format!("{kind:?}"),
        )],
        precision: Precision::Method,
        confidence: Confidence::High,
        activation: Activation::Candidate,
        priority: Some(priority),
    }
}

#[expect(clippy::too_many_arguments)]
fn degraded_injector_effect(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    handler: &ParsedMethod,
    injector: &ParsedAnnotation,
    target: Target,
    mutation: MutationKind,
    mechanism: Mechanism,
    priority: i32,
    reason: &str,
) -> Effect {
    let precision = if target.member.is_some() {
        Precision::Method
    } else {
        Precision::Class
    };
    Effect {
        artifact_id: artifact.id.clone(),
        mechanism,
        target: target.clone(),
        requirements: vec![ShapeRequirement {
            kind: RequirementKind::UnknownShape,
            target: target.clone(),
            precision,
            minimum_matches: None,
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
        queries: Vec::new(),
        mutations: vec![Mutation::new(mutation, target, precision)],
        evidence: vec![{
            let mut evidence = Evidence::new(
                &artifact.id,
                &mixin.name,
                format!("{reason}; declared mutation {mutation:?}"),
            );
            evidence.method = Some(format!("{}{}", handler.name, handler.descriptor));
            evidence.annotation = Some(injector.descriptor.clone());
            evidence.mechanism = Some(mechanism);
            evidence.injector_kind = Some(injector_simple_name(&injector.descriptor).to_string());
            evidence.composition_semantics = Some(mutation.default_composition());
            evidence.analysis_precision = Some(precision);
            evidence
        }],
        precision,
        confidence: Confidence::Low,
        activation: Activation::Candidate,
        priority: Some(priority),
    }
}

fn injector_kind(descriptor: &str) -> Option<(MutationKind, Mechanism)> {
    let simple = injector_simple_name(descriptor);
    let mixin_extras = descriptor.contains("mixinextras");
    let mechanism = if mixin_extras {
        Mechanism::MixinExtras
    } else {
        Mechanism::Mixin
    };
    Some(match simple {
        "Inject" => (MutationKind::InsertInstructions, mechanism),
        "Redirect" => (MutationKind::RedirectOperation, mechanism),
        "ModifyArg" | "ModifyArgs" => (MutationKind::ModifyArgument, mechanism),
        "ModifyVariable" => (MutationKind::ModifyLocalValue, mechanism),
        "ModifyConstant" => (MutationKind::ModifyConstant, mechanism),
        "WrapOperation" | "WrapMethod" => (MutationKind::WrapOperation, mechanism),
        "ModifyExpressionValue" | "ModifyReturnValue" => {
            (MutationKind::TransformExpressionValue, mechanism)
        }
        "WrapWithCondition" => (MutationKind::InsertInstructions, mechanism),
        _ => return None,
    })
}

fn injector_simple_name(descriptor: &str) -> &str {
    descriptor
        .trim_end_matches(';')
        .rsplit('/')
        .next()
        .unwrap_or_default()
}

fn match_instructions<'a>(
    method: &'a ParsedMethod,
    at: &ParsedAnnotation,
    kind: &str,
    target_candidates: &[String],
    range: Option<(usize, usize)>,
) -> Vec<&'a ParsedInstruction> {
    let targets = target_candidates
        .iter()
        .map(|value| parse_selector(value))
        .collect::<Vec<_>>();
    let (start, end) = range.unwrap_or_else(|| (0, method.instructions.len().saturating_sub(1)));
    if method.instructions.is_empty() || start > end || end >= method.instructions.len() {
        return Vec::new();
    }
    let indexed = method.instructions[start..=end].iter().enumerate();
    let mut matches = match kind {
        "HEAD" => method.instructions.get(start).into_iter().collect(),
        "TAIL" => method.instructions[start..=end]
            .iter()
            .rev()
            .find(|instruction| matches!(instruction.kind, InstructionKind::Return))
            .into_iter()
            .collect(),
        "RETURN" => method.instructions[start..=end]
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Return))
            .collect(),
        "INVOKE" => method.instructions[start..=end]
            .iter()
            .filter(|instruction| {
                matches!(&instruction.kind, InstructionKind::MethodCall(member) if targets.is_empty() || targets.iter().any(|target| selector_matches_member(target, member)))
            })
            .collect(),
        "INVOKE_STRING" => {
            let ldc = at_argument(at, "ldc");
            indexed
                .filter_map(|(relative_index, instruction)| {
                    let member_matches = matches!(
                        &instruction.kind,
                        InstructionKind::MethodCall(member)
                            if targets.is_empty()
                                || targets
                                    .iter()
                                    .any(|target| selector_matches_member(target, member))
                    );
                    if !member_matches {
                        return None;
                    }
                    let absolute_index = start + relative_index;
                    let preceding_matches = absolute_index.checked_sub(1).is_some_and(|index| {
                        matches!(
                            &method.instructions[index].kind,
                            InstructionKind::StringConstant(value)
                                if ldc.as_deref().is_none_or(|expected| expected == value)
                        )
                    });
                    preceding_matches.then_some(instruction)
                })
                .collect()
        }
        "FIELD" => method.instructions[start..=end]
            .iter()
            .filter(|instruction| {
                matches!(&instruction.kind, InstructionKind::FieldRead(member) | InstructionKind::FieldWrite(member) if targets.is_empty() || targets.iter().any(|target| selector_matches_member(target, member)))
            })
            .collect(),
        "NEW" => method.instructions[start..=end]
            .iter()
            .filter(|instruction| {
                instruction.reference.opcode == 187
                    && matches!(&instruction.kind, InstructionKind::Type(class) if targets.is_empty() || targets.iter().any(|target| target.owner.as_deref().is_none_or(|owner| owner == class)))
            })
            .collect(),
        "CONSTANT" => {
            let arguments = at
                .value("args")
                .map(AnnotationValue::strings)
                .unwrap_or_default();
            method.instructions[start..=end]
                .iter()
                .filter(|instruction| constant_at_arguments_match(instruction, &arguments))
                .collect()
        }
        "JUMP" => method.instructions[start..=end]
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Jump))
            .collect(),
        "LOAD" => method.instructions[start..=end]
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Load(_)))
            .collect(),
        "STORE" => method.instructions[start..=end]
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Store(_)))
            .collect(),
        _ => Vec::new(),
    };
    if let Some(opcode) = at
        .value("opcode")
        .and_then(AnnotationValue::integer)
        .filter(|opcode| *opcode >= 0)
        .and_then(|opcode| u8::try_from(opcode).ok())
    {
        matches.retain(|instruction| instruction.reference.opcode == opcode);
    }
    matches
}

fn at_argument(at: &ParsedAnnotation, key: &str) -> Option<String> {
    at.value("args")
        .map(AnnotationValue::strings)
        .unwrap_or_default()
        .into_iter()
        .find_map(|argument| {
            let (name, value) = argument.split_once('=')?;
            (name.trim() == key).then(|| value.trim().to_string())
        })
}

fn constant_at_arguments_match(instruction: &ParsedInstruction, arguments: &[String]) -> bool {
    if arguments.is_empty() {
        return matches!(
            instruction.kind,
            InstructionKind::StringConstant(_)
                | InstructionKind::IntegerConstant(_)
                | InstructionKind::DecimalConstant(_)
                | InstructionKind::NullConstant
        ) || matches!(instruction.kind, InstructionKind::Type(_))
            && matches!(instruction.reference.opcode, 18..=20);
    }
    arguments.iter().any(|argument| {
        let Some((key, value)) = argument.split_once('=') else {
            return false;
        };
        let key = key.trim();
        let value = value.trim();
        match (key, &instruction.kind) {
            ("nullValue", InstructionKind::NullConstant) => value.eq_ignore_ascii_case("true"),
            ("intValue" | "longValue", InstructionKind::IntegerConstant(candidate)) => {
                value.parse::<i64>() == Ok(*candidate)
            }
            ("floatValue" | "doubleValue", InstructionKind::DecimalConstant(candidate)) => {
                decimal_constants_equal(candidate, value)
            }
            ("stringValue", InstructionKind::StringConstant(candidate)) => candidate == value,
            ("classValue", InstructionKind::Type(candidate)) => {
                matches!(instruction.reference.opcode, 18..=20)
                    && normalize_class_name(value).as_deref() == Some(candidate)
            }
            _ => false,
        }
    })
}

fn match_modify_constants<'a>(
    method: &'a ParsedMethod,
    injector: &ParsedAnnotation,
    handler: &ParsedMethod,
) -> Vec<&'a ParsedInstruction> {
    let discriminators = injector
        .value("constant")
        .map(AnnotationValue::annotations)
        .unwrap_or_default();
    if discriminators.is_empty() {
        return method
            .instructions
            .iter()
            .filter(|instruction| constant_matches_handler_type(instruction, handler))
            .collect();
    }

    let mut matches = Vec::new();
    for discriminator in discriminators {
        let mut candidates = method
            .instructions
            .iter()
            .filter(|instruction| constant_matches_discriminator(instruction, discriminator))
            .collect::<Vec<_>>();
        if let Some(ordinal) = positive_u32(discriminator.value("ordinal")) {
            candidates = candidates
                .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
                .copied()
                .into_iter()
                .collect();
        }
        matches.extend(candidates);
    }
    matches.sort_by_key(|instruction| instruction.reference.stable_id);
    matches.dedup_by_key(|instruction| instruction.reference.stable_id);
    matches
}

fn constant_matches_handler_type(instruction: &ParsedInstruction, handler: &ParsedMethod) -> bool {
    let result = handler
        .descriptor
        .rsplit_once(')')
        .map(|(_, result)| result);
    match result {
        Some("B" | "C" | "I" | "J" | "S" | "Z") => {
            matches!(&instruction.kind, InstructionKind::IntegerConstant(_))
        }
        Some("F" | "D") => matches!(&instruction.kind, InstructionKind::DecimalConstant(_)),
        Some("Ljava/lang/String;") => {
            matches!(&instruction.kind, InstructionKind::StringConstant(_))
        }
        Some("Ljava/lang/Class;") => matches!(&instruction.kind, InstructionKind::Type(_)),
        Some(result) if result.starts_with('L') || result.starts_with('[') => {
            matches!(&instruction.kind, InstructionKind::NullConstant)
        }
        _ => false,
    }
}

fn constant_matches_discriminator(
    instruction: &ParsedInstruction,
    discriminator: &ParsedAnnotation,
) -> bool {
    if discriminator
        .value("nullValue")
        .and_then(AnnotationValue::boolean)
        == Some(true)
    {
        return matches!(&instruction.kind, InstructionKind::NullConstant);
    }
    for name in ["intValue", "longValue"] {
        if let Some(value) = discriminator.value(name).and_then(AnnotationValue::integer) {
            return matches!(&instruction.kind, InstructionKind::IntegerConstant(candidate) if *candidate == value);
        }
    }
    for name in ["floatValue", "doubleValue"] {
        if let Some(AnnotationValue::Float(value)) = discriminator.value(name) {
            return matches!(&instruction.kind, InstructionKind::DecimalConstant(candidate) if decimal_constants_equal(candidate, value));
        }
    }
    if let Some(value) = discriminator
        .value("stringValue")
        .and_then(|value| value.strings().into_iter().next())
    {
        return matches!(&instruction.kind, InstructionKind::StringConstant(candidate) if candidate == &value);
    }
    if let Some(value) = discriminator
        .value("classValue")
        .and_then(|value| value.strings().into_iter().next())
        .and_then(|value| normalize_class_name(&value))
    {
        return matches!(&instruction.kind, InstructionKind::Type(candidate) if candidate == &value);
    }
    if discriminator
        .value("expandZeroConditions")
        .is_some_and(annotation_value_is_nonempty)
    {
        return matches!(&instruction.kind, InstructionKind::Jump);
    }
    false
}

fn decimal_constants_equal(left: &str, right: &str) -> bool {
    left.parse::<f64>()
        .ok()
        .zip(right.parse::<f64>().ok())
        .is_some_and(|(left, right)| left == right)
}

fn annotation_value_is_nonempty(value: &AnnotationValue) -> bool {
    match value {
        AnnotationValue::Array(values) => !values.is_empty(),
        _ => true,
    }
}

#[derive(Debug)]
struct ActiveSlice {
    id: Option<String>,
    range: Option<(usize, usize)>,
    requirements: Vec<ShapeRequirement>,
    resolution: SoftReferenceResolution,
    unresolved: Option<String>,
}

fn resolve_active_slice(
    method: &ParsedMethod,
    method_target: &Target,
    slices: &[&ParsedAnnotation],
    at: &ParsedAnnotation,
    artifact: &ParsedArtifact,
    mixin_class: &str,
    target_class: &str,
) -> ActiveSlice {
    let requested = at
        .value("slice")
        .and_then(|value| value.strings().into_iter().next())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "<default>".to_string());
    let matching = slices
        .iter()
        .filter(|slice| {
            slice
                .value("id")
                .and_then(|value| value.strings().into_iter().next())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "<default>".to_string())
                == requested
        })
        .copied()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        if requested == "<default>" {
            return ActiveSlice {
                id: None,
                range: method.instructions.len().checked_sub(1).map(|end| (0, end)),
                requirements: Vec::new(),
                resolution: SoftReferenceResolution::NotApplicable,
                unresolved: method
                    .instructions
                    .is_empty()
                    .then(|| "the target method contains no instructions".to_string()),
            };
        }
        return ActiveSlice {
            id: Some(requested.clone()),
            range: None,
            requirements: Vec::new(),
            resolution: SoftReferenceResolution::Unresolved,
            unresolved: Some(format!("requested slice '{requested}' is not declared")),
        };
    }
    if matching.len() > 1 {
        return ActiveSlice {
            id: Some(requested.clone()),
            range: None,
            requirements: Vec::new(),
            resolution: SoftReferenceResolution::Ambiguous,
            unresolved: Some(format!("slice '{requested}' is declared more than once")),
        };
    }
    let slice = matching[0];
    let mut requirements = Vec::new();
    let mut indices = Vec::new();
    let mut resolution = SoftReferenceResolution::NotApplicable;
    for boundary_name in ["from", "to"] {
        let boundary = slice
            .value(boundary_name)
            .map(AnnotationValue::annotations)
            .unwrap_or_default();
        let (instruction, ordinal, boundary_resolution) = if boundary.is_empty() {
            let instruction = if boundary_name == "from" {
                method.instructions.first()
            } else {
                method
                    .instructions
                    .iter()
                    .rev()
                    .find(|instruction| matches!(instruction.kind, InstructionKind::Return))
            };
            (instruction, None, SoftReferenceResolution::NotApplicable)
        } else {
            let boundary_at = boundary[0];
            let kind = boundary_at
                .value("value")
                .and_then(|value| value.strings().into_iter().next())
                .unwrap_or_default()
                .to_ascii_uppercase();
            if injection_point_support(&kind) != InjectionPointSupport::Supported {
                return ActiveSlice {
                    id: Some(requested.clone()),
                    range: None,
                    requirements,
                    resolution: SoftReferenceResolution::Unresolved,
                    unresolved: Some(format!(
                        "slice '{requested}' {boundary_name} uses unsupported @At({kind})"
                    )),
                };
            }
            let ordinal = boundary_at
                .value("ordinal")
                .and_then(AnnotationValue::integer)
                .filter(|value| *value >= 0)
                .and_then(|value| u32::try_from(value).ok());
            let (target_candidates, _, boundary_resolution) = resolve_at_reference(
                method,
                artifact,
                mixin_class,
                target_class,
                boundary_at,
                &kind,
                None,
            );
            let matches = match_instructions(method, boundary_at, &kind, &target_candidates, None);
            let selected = ordinal.map_or_else(
                || matches.clone(),
                |ordinal| {
                    matches
                        .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
                        .copied()
                        .into_iter()
                        .collect()
                },
            );
            let shifted = apply_shift(method, boundary_at, selected);
            let instruction = if boundary_name == "from" {
                shifted.first().copied()
            } else {
                shifted.last().copied()
            };
            (instruction, ordinal, boundary_resolution)
        };
        resolution = weaker_resolution(resolution, boundary_resolution);
        let Some(instruction) = instruction else {
            return ActiveSlice {
                id: Some(requested.clone()),
                range: None,
                requirements,
                resolution: SoftReferenceResolution::Unresolved,
                unresolved: Some(format!(
                    "slice '{requested}' {boundary_name} boundary did not resolve"
                )),
            };
        };
        let Some(index) = instruction_index(method, instruction.reference.stable_id) else {
            return ActiveSlice {
                id: Some(requested.clone()),
                range: None,
                requirements,
                resolution: SoftReferenceResolution::Unresolved,
                unresolved: Some(format!(
                    "slice '{requested}' {boundary_name} boundary has no stable instruction"
                )),
            };
        };
        indices.push(index);
        let mut target = method_target.clone();
        target.instruction = Some(instruction.reference.clone());
        requirements.push(ShapeRequirement {
            kind: RequirementKind::SliceBoundary,
            target,
            precision: Precision::Instruction,
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal,
            slice: Some(requested.clone()),
        });
    }
    if indices[0] > indices[1] {
        return ActiveSlice {
            id: Some(requested.clone()),
            range: None,
            requirements,
            resolution: SoftReferenceResolution::Unresolved,
            unresolved: Some(format!("slice '{requested}' starts after its end boundary")),
        };
    }
    ActiveSlice {
        id: Some(requested),
        range: Some((indices[0], indices[1])),
        requirements,
        resolution,
        unresolved: None,
    }
}

fn instruction_index(method: &ParsedMethod, stable_id: u32) -> Option<usize> {
    method
        .instructions
        .iter()
        .position(|instruction| instruction.reference.stable_id == stable_id)
}

fn apply_shift<'a>(
    method: &'a ParsedMethod,
    at: &ParsedAnnotation,
    matches: Vec<&'a ParsedInstruction>,
) -> Vec<&'a ParsedInstruction> {
    let shift = at.value("shift").and_then(|value| match value {
        AnnotationValue::Enum { value, .. } => Some(value.as_str()),
        _ => None,
    });
    let by = at
        .value("by")
        .and_then(AnnotationValue::integer)
        .and_then(|value| isize::try_from(value).ok())
        .unwrap_or(0);
    let delta = match shift {
        Some("BEFORE") => -1,
        Some("AFTER") => 1,
        Some("BY") => by,
        _ => 0,
    };
    if delta == 0 {
        return matches;
    }
    matches
        .into_iter()
        .filter_map(|instruction| {
            let index =
                isize::try_from(instruction_index(method, instruction.reference.stable_id)?)
                    .ok()?;
            usize::try_from(index.checked_add(delta)?)
                .ok()
                .and_then(|index| method.instructions.get(index))
        })
        .collect()
}

fn render_shift(at: &ParsedAnnotation) -> Option<String> {
    let shift = at.value("shift").and_then(|value| match value {
        AnnotationValue::Enum { value, .. } => Some(value.as_str()),
        _ => None,
    })?;
    if shift == "NONE" {
        return None;
    }
    if shift == "BY" {
        let by = at
            .value("by")
            .and_then(AnnotationValue::integer)
            .unwrap_or_default();
        Some(format!("BY({by})"))
    } else {
        Some(shift.to_string())
    }
}

fn resolve_target_methods<'a>(
    scanned: &'a ScannedArtifacts,
    target_class: &str,
    selector: &str,
) -> Vec<(String, &'a ParsedMethod)> {
    let selector = parse_selector(selector);
    let owner = selector.owner.as_deref().unwrap_or(target_class);
    scanned
        .universe
        .parsed_definitions(&scanned.artifacts, owner)
        .into_iter()
        .flat_map(|(artifact, class)| {
            class
                .methods
                .iter()
                .filter(|method| {
                    method.name == selector.name
                        && selector
                            .descriptor
                            .as_ref()
                            .is_none_or(|descriptor| descriptor == &method.descriptor)
                })
                .map(move |method| (artifact.id.clone(), method))
        })
        .collect()
}

#[derive(Debug)]
struct ResolvedMethods<'a> {
    candidates: Vec<String>,
    sources: Vec<String>,
    resolution: SoftReferenceResolution,
    methods: Vec<(String, &'a ParsedMethod)>,
}

fn resolve_method_reference<'a>(
    scanned: &'a ScannedArtifacts,
    artifact: &ParsedArtifact,
    mixin_class: &str,
    target_class: &str,
    original: &str,
) -> ResolvedMethods<'a> {
    let candidates = refmap_candidates(artifact, mixin_class, original, target_class);
    let sources = refmap_sources(artifact, mixin_class, original, target_class);
    let mut methods = Vec::new();
    let mut active_candidates = BTreeSet::new();
    let mut direct_keys = BTreeSet::new();
    for candidate in &candidates {
        let resolved = resolve_target_methods(scanned, target_class, candidate);
        if !resolved.is_empty() {
            active_candidates.insert(normalize_soft_reference(candidate, target_class));
        }
        for (artifact_id, method) in resolved {
            let key = format!("{artifact_id}|{}{}", method.name, method.descriptor);
            if candidate == original {
                direct_keys.insert(key);
            }
            methods.push((artifact_id, method));
        }
    }
    methods.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.name.cmp(&right.1.name))
            .then_with(|| left.1.descriptor.cmp(&right.1.descriptor))
    });
    methods.dedup_by(|left, right| {
        left.0 == right.0 && left.1.name == right.1.name && left.1.descriptor == right.1.descriptor
    });
    let distinct_methods = methods
        .iter()
        .map(|(artifact_id, method)| format!("{artifact_id}|{}{}", method.name, method.descriptor))
        .collect::<BTreeSet<_>>();
    let resolution = match distinct_methods.len() {
        0 => SoftReferenceResolution::Unresolved,
        1 if distinct_methods.iter().all(|key| direct_keys.contains(key)) => {
            SoftReferenceResolution::DirectExact
        }
        1 => SoftReferenceResolution::RefmapExact,
        _ => SoftReferenceResolution::Ambiguous,
    };
    ResolvedMethods {
        candidates,
        sources,
        resolution,
        methods,
    }
}

fn resolve_at_reference(
    method: &ParsedMethod,
    artifact: &ParsedArtifact,
    mixin_class: &str,
    target_class: &str,
    at: &ParsedAnnotation,
    kind: &str,
    range: Option<(usize, usize)>,
) -> (Vec<String>, Vec<String>, SoftReferenceResolution) {
    let originals = at
        .value("target")
        .into_iter()
        .chain(at.value("desc"))
        .flat_map(selector_values)
        .collect::<BTreeSet<_>>();
    let (candidates, sources) = at_selector_candidates(artifact, mixin_class, at, target_class);
    if originals.is_empty() {
        return (candidates, sources, SoftReferenceResolution::NotApplicable);
    }
    let mut active = BTreeSet::new();
    let mut direct = BTreeSet::new();
    for candidate in &candidates {
        if !match_instructions(method, at, kind, std::slice::from_ref(candidate), range).is_empty()
        {
            let normalized = normalize_soft_reference(candidate, target_class);
            if originals
                .iter()
                .any(|original| normalize_soft_reference(original, target_class) == normalized)
            {
                direct.insert(normalized.clone());
            }
            active.insert(normalized);
        }
    }
    let resolution = match active.len() {
        0 => SoftReferenceResolution::Unresolved,
        1 if active.iter().all(|candidate| direct.contains(candidate)) => {
            SoftReferenceResolution::DirectExact
        }
        1 => SoftReferenceResolution::RefmapExact,
        _ => SoftReferenceResolution::Ambiguous,
    };
    (candidates, sources, resolution)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionPointSupport {
    Supported,
    KnownUnsupported,
    Custom,
}

fn injection_point_support(kind: &str) -> InjectionPointSupport {
    match kind {
        "HEAD" | "TAIL" | "RETURN" | "INVOKE" | "INVOKE_STRING" | "FIELD" | "NEW" | "CONSTANT"
        | "JUMP" | "LOAD" | "STORE" => InjectionPointSupport::Supported,
        "INVOKE_ASSIGN" | "MIXINEXTRAS:EXPRESSION" => InjectionPointSupport::KnownUnsupported,
        _ => InjectionPointSupport::Custom,
    }
}

fn confidence_for_resolution(resolution: SoftReferenceResolution) -> Confidence {
    match resolution {
        SoftReferenceResolution::DirectExact
        | SoftReferenceResolution::RefmapExact
        | SoftReferenceResolution::NotApplicable => Confidence::Exact,
        SoftReferenceResolution::Ambiguous => Confidence::Medium,
        SoftReferenceResolution::Unresolved => Confidence::Low,
    }
}

fn weaker_resolution(
    left: SoftReferenceResolution,
    right: SoftReferenceResolution,
) -> SoftReferenceResolution {
    fn rank(resolution: SoftReferenceResolution) -> u8 {
        match resolution {
            SoftReferenceResolution::Unresolved => 0,
            SoftReferenceResolution::Ambiguous => 1,
            SoftReferenceResolution::RefmapExact => 2,
            SoftReferenceResolution::DirectExact | SoftReferenceResolution::NotApplicable => 3,
        }
    }
    if rank(left) <= rank(right) {
        left
    } else {
        right
    }
}

fn warn_for_soft_reference(
    warnings: &mut Vec<Warning>,
    artifact: &ParsedArtifact,
    scope: &str,
    reference_kind: &str,
    reference: &str,
    resolution: SoftReferenceResolution,
) {
    let (kind, message) = match resolution {
        SoftReferenceResolution::Ambiguous => (
            WarningKind::AmbiguousSoftReference,
            format!(
                "{reference_kind} '{reference}' resolves to multiple active class-universe candidates"
            ),
        ),
        SoftReferenceResolution::Unresolved => (
            WarningKind::UnresolvedSoftReference,
            format!(
                "{reference_kind} '{reference}' could not be resolved in the active class universe; no usable direct or refmap candidate was available"
            ),
        ),
        _ => return,
    };
    warnings.push(warning(artifact, scope, kind, &message));
}

fn query_id(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    handler: &ParsedMethod,
    target_class: &str,
    target_method: &ParsedMethod,
    selector_kind: &str,
) -> String {
    format!(
        "{}|{}|{}{}|{}|{}{}|{}",
        artifact.id,
        mixin.name,
        handler.name,
        handler.descriptor,
        target_class,
        target_method.name,
        target_method.descriptor,
        selector_kind
    )
}

fn injection_group_constraint(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    handler: &ParsedMethod,
    group_name: Option<&str>,
    minimum_successes: Option<u32>,
    maximum_successes: Option<u32>,
) -> Option<InjectionGroupConstraint> {
    let group_name = group_name?;
    Some(InjectionGroupConstraint {
        id: format!("{}|{}|{group_name}", artifact.id, mixin.name),
        member_id: format!("{}{}", handler.name, handler.descriptor),
        successful_members: 0,
        minimum_successes,
        maximum_successes,
    })
}

fn finalize_injection_groups(effects: &mut [Effect]) {
    let mut successes = BTreeMap::<String, BTreeSet<String>>::new();
    for query in effects.iter().flat_map(|effect| &effect.queries) {
        let Some(group) = &query.group else {
            continue;
        };
        let selected = u32::try_from(query.selected.len()).unwrap_or(u32::MAX);
        let successful = query
            .minimum_matches
            .is_none_or(|minimum| selected >= minimum)
            && query
                .maximum_matches
                .is_none_or(|maximum| selected <= maximum);
        if successful {
            successes
                .entry(group.id.clone())
                .or_default()
                .insert(group.member_id.clone());
        }
    }
    for query in effects.iter_mut().flat_map(|effect| &mut effect.queries) {
        if let Some(group) = &mut query.group {
            group.successful_members =
                u32::try_from(successes.get(&group.id).map_or(0, BTreeSet::len))
                    .unwrap_or(u32::MAX);
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn injector_evidence(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    handler: &ParsedMethod,
    injector: &ParsedAnnotation,
    mechanism: Mechanism,
    mutation: MutationKind,
    method_selector: &str,
    at_kind: Option<&str>,
    at_target: Option<&str>,
    resolution: SoftReferenceResolution,
    refmap_sources: &[String],
    target_candidates: &[String],
    precision: Precision,
    summary: &str,
) -> Evidence {
    let mut evidence = Evidence::new(
        &artifact.id,
        &mixin.name,
        format!(
            "{summary}; annotation values: {}",
            annotation_values(injector)
        ),
    );
    evidence.method = Some(format!("{}{}", handler.name, handler.descriptor));
    evidence.annotation = Some(injector.descriptor.clone());
    evidence.mechanism = Some(mechanism);
    evidence.injector_kind = Some(injector_simple_name(&injector.descriptor).to_string());
    evidence.composition_semantics = Some(mutation.default_composition());
    evidence.method_selector = Some(method_selector.to_string());
    evidence.at_kind = at_kind.map(str::to_string);
    evidence.at_target = at_target.map(str::to_string);
    evidence.resolution_kind = Some(resolution);
    evidence.refmap_sources = refmap_sources.to_vec();
    evidence.target_candidates = target_candidates.to_vec();
    evidence.analysis_precision = Some(precision);
    evidence
}

#[derive(Debug)]
struct Selector {
    owner: Option<String>,
    name: String,
    descriptor: Option<String>,
}

fn parse_selector(value: &str) -> Selector {
    let value = value.trim();
    let (owner, member) = if let Some(rest) = value.strip_prefix('L') {
        rest.split_once(';')
            .map_or((None, value), |(owner, member)| {
                (Some(owner.to_string()), member)
            })
    } else {
        (None, value)
    };
    let (name, descriptor) = if let Some(position) = member.find('(') {
        (&member[..position], Some(member[position..].to_string()))
    } else if let Some((name, descriptor)) = member.split_once(':') {
        (name, Some(descriptor.to_string()))
    } else {
        (member, None)
    };
    Selector {
        owner,
        name: name
            .trim_start_matches('.')
            .trim_start_matches(':')
            .to_string(),
        descriptor,
    }
}

fn selector_values(value: &AnnotationValue) -> Vec<String> {
    let mut selectors = value.strings();
    selectors.extend(value.annotations().into_iter().filter_map(desc_selector));
    selectors
}

fn at_selector_candidates(
    artifact: &ParsedArtifact,
    mixin_class: &str,
    at: &ParsedAnnotation,
    default_owner: &str,
) -> (Vec<String>, Vec<String>) {
    let originals = at
        .value("target")
        .into_iter()
        .chain(at.value("desc"))
        .flat_map(selector_values)
        .collect::<BTreeSet<_>>();
    let mut candidates = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for original in originals {
        candidates.extend(refmap_candidates(
            artifact,
            mixin_class,
            &original,
            default_owner,
        ));
        sources.extend(refmap_sources(
            artifact,
            mixin_class,
            &original,
            default_owner,
        ));
    }
    (
        candidates.into_iter().collect(),
        sources.into_iter().collect(),
    )
}

fn desc_selector(annotation: &ParsedAnnotation) -> Option<String> {
    if !annotation.descriptor.ends_with("/Desc;")
        && annotation.descriptor != "Lorg/spongepowered/asm/mixin/injection/Desc;"
    {
        return None;
    }
    let name = annotation
        .value("value")
        .or_else(|| annotation.value("name"))
        .and_then(|value| value.strings().into_iter().next())?;
    let owner = annotation
        .value("owner")
        .and_then(|value| value.strings().into_iter().next())
        .and_then(|value| normalize_class_name(&value));
    let parameters = annotation
        .value("args")
        .map(AnnotationValue::strings)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| type_descriptor(&value))
        .collect::<String>();
    let result = annotation
        .value("ret")
        .and_then(|value| value.strings().into_iter().next())
        .and_then(|value| type_descriptor(&value))
        .unwrap_or_else(|| "V".to_string());
    Some(owner.map_or_else(
        || format!("{name}({parameters}){result}"),
        |owner| format!("L{owner};{name}({parameters}){result}"),
    ))
}

fn type_descriptor(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with('[')
        || (value.starts_with('L') && value.ends_with(';'))
        || (value.len() == 1
            && matches!(
                value.as_bytes()[0],
                b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b'V'
            ))
    {
        return Some(value.replace('.', "/"));
    }
    normalize_class_name(value).map(|class| format!("L{class};"))
}

fn selector_matches_member(selector: &Selector, member: &MemberReference) -> bool {
    selector
        .owner
        .as_ref()
        .is_none_or(|owner| owner == &member.owner)
        && selector.name == member.name
        && selector
            .descriptor
            .as_ref()
            .is_none_or(|descriptor| descriptor == &member.descriptor)
}

fn refmap_candidates(
    artifact: &ParsedArtifact,
    mixin_class: &str,
    original: &str,
    default_owner: &str,
) -> Vec<String> {
    let normalized_mixin = mixin_class.replace('.', "/");
    let mut candidates = artifact
        .refmaps
        .iter()
        .filter(|entry| {
            entry.mixin_class == normalized_mixin
                && (entry.original == original
                    || normalize_soft_reference(&entry.original, default_owner)
                        == normalize_soft_reference(original, default_owner))
        })
        .map(|entry| entry.mapped.clone())
        .collect::<BTreeSet<_>>();
    candidates.insert(original.to_string());
    candidates.into_iter().collect()
}

fn refmap_sources(
    artifact: &ParsedArtifact,
    mixin_class: &str,
    original: &str,
    default_owner: &str,
) -> Vec<String> {
    let normalized_mixin = mixin_class.replace('.', "/");
    artifact
        .refmaps
        .iter()
        .filter(|entry| {
            entry.mixin_class == normalized_mixin
                && (entry.original == original
                    || normalize_soft_reference(&entry.original, default_owner)
                        == normalize_soft_reference(original, default_owner))
        })
        .map(|entry| {
            entry.context.as_ref().map_or_else(
                || entry.path.clone(),
                |context| format!("{}[{context}]", entry.path),
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_soft_reference(value: &str, default_owner: &str) -> String {
    let selector = parse_selector(value);
    format!(
        "L{};{}{}",
        selector.owner.as_deref().unwrap_or(default_owner),
        selector.name,
        selector.descriptor.unwrap_or_default()
    )
}

fn mixin_targets(annotation: &ParsedAnnotation) -> Vec<String> {
    annotation
        .value("value")
        .into_iter()
        .chain(annotation.value("targets"))
        .flat_map(AnnotationValue::strings)
        .filter_map(|value| normalize_class_name(&value))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_class_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let trimmed = trimmed.strip_prefix('L').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed);
    (!trimmed.is_empty()).then(|| trimmed.replace('.', "/"))
}

fn annotation<'a>(
    annotations: &'a [ParsedAnnotation],
    descriptor: &str,
) -> Option<&'a ParsedAnnotation> {
    annotations
        .iter()
        .find(|annotation| annotation.descriptor == descriptor)
}

fn positive_u32(value: Option<&AnnotationValue>) -> Option<u32> {
    value
        .and_then(AnnotationValue::integer)
        .filter(|value| *value >= 0)
        .and_then(|value| u32::try_from(value).ok())
}

fn enum_is(value: &AnnotationValue, expected: &str) -> bool {
    matches!(value, AnnotationValue::Enum { value, .. } if value == expected)
}

fn annotation_values(annotation: &ParsedAnnotation) -> String {
    if annotation.values.is_empty() {
        return "none".to_string();
    }
    annotation
        .values
        .iter()
        .map(|(name, value)| format!("{name}={}", value.render_lossy()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn derived_accessor_name(method: &str) -> String {
    ["get", "set", "is", "call", "invoke"]
        .into_iter()
        .find_map(|prefix| method.strip_prefix(prefix))
        .filter(|name| !name.is_empty())
        .map(|name| {
            let mut characters = name.chars();
            characters
                .next()
                .map(|first| first.to_ascii_lowercase().to_string() + characters.as_str())
                .unwrap_or_default()
        })
        .unwrap_or_else(|| method.to_string())
}

fn accessor_field_descriptor(method_descriptor: &str) -> String {
    method_descriptor
        .split_once(')')
        .map(|(parameters, result)| {
            if result == "V" {
                parameters
                    .strip_prefix('(')
                    .unwrap_or(parameters)
                    .to_string()
            } else {
                result.to_string()
            }
        })
        .unwrap_or_default()
}

fn evidence(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    method: Option<&ParsedMethod>,
    annotation: &str,
    detail: String,
) -> Evidence {
    let mut evidence = Evidence::new(&artifact.id, &mixin.name, detail);
    evidence.method = method.map(|method| format!("{}{}", method.name, method.descriptor));
    evidence.annotation = Some(annotation.to_string());
    evidence.mechanism = Some(Mechanism::Mixin);
    evidence.analysis_precision = Some(if method.is_some() {
        Precision::Method
    } else {
        Precision::Class
    });
    evidence
}

fn warning(artifact: &ParsedArtifact, scope: &str, kind: WarningKind, message: &str) -> Warning {
    Warning {
        artifact_id: Some(artifact.id.clone()),
        scope: scope.to_string(),
        kind,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::classfile::{
        ParsedAnnotation, ParsedClass, ParsedField, ParsedInstruction, ParsedMethod,
    };
    use crate::jar::{ClassUniverse, RefmapEntry};
    use crate::model::{
        AnalysisLimits, ArtifactKind, Confidence, Coverage, InstructionReference, MutationKind,
    };

    use super::*;

    #[test]
    fn direct_exact_reference_needs_no_refmap_warning_or_confidence_penalty() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        let effects = analyze(&mut scanned);
        let injector = effects
            .iter()
            .find(|effect| effect.precision == Precision::Instruction)
            .unwrap();
        assert_eq!(injector.confidence, Confidence::Exact);
        assert!(!scanned.warnings.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::UnresolvedSoftReference | WarningKind::AmbiguousSoftReference
        )));
    }

    #[test]
    fn custom_injection_point_degrades_to_method_precision() {
        let mut scanned = mixin_fixture(
            "example:custom",
            "Lorg/spongepowered/asm/mixin/injection/Inject;",
        );
        let effects = analyze(&mut scanned);
        let injector = effects
            .iter()
            .find(|effect| {
                effect
                    .mutations
                    .iter()
                    .any(|mutation| mutation.kind == MutationKind::InsertInstructions)
            })
            .unwrap();
        assert_eq!(injector.precision, Precision::Method);
        assert_eq!(injector.mutations[0].kind, MutationKind::InsertInstructions);
        assert!(
            scanned
                .warnings
                .iter()
                .any(|warning| warning.kind == WarningKind::CustomInjectionPoint)
        );
    }

    #[test]
    fn multiple_refmap_contexts_are_not_arbitrarily_selected() {
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: Vec::new(),
            refmaps: vec![
                RefmapEntry {
                    path: "mod.refmap.json".to_string(),
                    context: Some("named:intermediary".to_string()),
                    mixin_class: "example/Mixin".to_string(),
                    original: "tick()V".to_string(),
                    mapped: "method_1()V".to_string(),
                },
                RefmapEntry {
                    path: "mod.refmap.json".to_string(),
                    context: Some("named:official".to_string()),
                    mixin_class: "example/Mixin".to_string(),
                    original: "tick()V".to_string(),
                    mapped: "a()V".to_string(),
                },
            ],
        };
        let candidates = refmap_candidates(&artifact, "example/Mixin", "tick()V", "game/Target");
        assert_eq!(candidates, vec!["a()V", "method_1()V", "tick()V"]);
        let sources = refmap_sources(&artifact, "example/Mixin", "tick()V", "game/Target");
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().all(|source| source.contains('[')));
    }

    #[test]
    fn multiple_active_refmap_candidates_are_reported_as_ambiguous() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![
            call_instruction(0, "game/Owner", "a"),
            call_instruction(1, "game/Owner", "method_1"),
            return_instruction(2),
        ];
        fixture_at_mut(&mut scanned).values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );
        scanned.artifacts[1].refmaps.extend([
            RefmapEntry {
                path: "mod.refmap.json".to_string(),
                context: Some("named:official".to_string()),
                mixin_class: "example/Mixin".to_string(),
                original: "Lgame/Owner;run()V".to_string(),
                mapped: "Lgame/Owner;a()V".to_string(),
            },
            RefmapEntry {
                path: "mod.refmap.json".to_string(),
                context: Some("named:intermediary".to_string()),
                mixin_class: "example/Mixin".to_string(),
                original: "Lgame/Owner;run()V".to_string(),
                mapped: "Lgame/Owner;method_1()V".to_string(),
            },
        ]);

        let effects = analyze(&mut scanned);
        let effect = effects
            .iter()
            .find(|effect| effect.precision == Precision::Instruction)
            .unwrap();

        assert_eq!(effect.confidence, Confidence::Medium);
        assert!(
            scanned
                .warnings
                .iter()
                .any(|warning| warning.kind == WarningKind::AmbiguousSoftReference)
        );
    }

    #[test]
    fn at_target_uses_refmap_candidates_and_actual_instruction_validation() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        let mapped_member = MemberReference {
            owner: "game/Owner".to_string(),
            name: "a".to_string(),
            descriptor: "()V".to_string(),
            kind: MemberKind::Method,
            is_static: Some(false),
        };
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![ParsedInstruction {
            reference: InstructionReference {
                stable_id: 0,
                original_offset: Some(0),
                opcode: 182,
                member: Some(mapped_member.clone()),
                constant: None,
            },
            kind: InstructionKind::MethodCall(mapped_member),
        }];
        let injector = &mut scanned.artifacts[1].classes[0].methods[0].annotations[0];
        let AnnotationValue::Array(ats) = injector.values.get_mut("at").unwrap() else {
            panic!("fixture @At must be an array");
        };
        let AnnotationValue::Annotation(at) = &mut ats[0] else {
            panic!("fixture @At must be an annotation");
        };
        at.values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );
        scanned.artifacts[1].refmaps.push(RefmapEntry {
            path: "mod.refmap.json".to_string(),
            context: Some("named:runtime".to_string()),
            mixin_class: "example/Mixin".to_string(),
            original: "Lgame/Owner;run()V".to_string(),
            mapped: "Lgame/Owner;a()V".to_string(),
        });

        let effects = analyze(&mut scanned);
        let effect = effects
            .iter()
            .find(|effect| effect.precision == Precision::Instruction)
            .unwrap();

        assert_eq!(effect.confidence, Confidence::Exact);
        assert_eq!(
            effect.evidence[0].resolution_kind,
            Some(SoftReferenceResolution::RefmapExact)
        );
        assert_eq!(
            effect.mutations[0]
                .target
                .instruction
                .as_ref()
                .unwrap()
                .member
                .as_ref()
                .unwrap()
                .name,
            "a"
        );
        assert!(
            effect.evidence[0]
                .refmap_sources
                .iter()
                .any(|source| source.contains("mod.refmap.json"))
        );
    }

    #[test]
    fn desc_selectors_and_mixinextras_semantics_are_recovered() {
        let desc = ParsedAnnotation {
            descriptor: "Lorg/spongepowered/asm/mixin/injection/Desc;".to_string(),
            values: BTreeMap::from([
                (
                    "owner".to_string(),
                    AnnotationValue::Class("Lgame/Target;".to_string()),
                ),
                (
                    "value".to_string(),
                    AnnotationValue::String("tick".to_string()),
                ),
                (
                    "args".to_string(),
                    AnnotationValue::Array(vec![AnnotationValue::Class(
                        "Ljava/lang/String;".to_string(),
                    )]),
                ),
                ("ret".to_string(), AnnotationValue::Class("V".to_string())),
            ]),
        };
        assert_eq!(
            desc_selector(&desc).as_deref(),
            Some("Lgame/Target;tick(Ljava/lang/String;)V")
        );
        let (wrap, mechanism) =
            injector_kind("Lcom/llamalad7/mixinextras/injector/wrapoperation/WrapOperation;")
                .unwrap();
        assert_eq!(wrap, MutationKind::WrapOperation);
        assert_eq!(mechanism, Mechanism::MixinExtras);
        let (redirect, _) =
            injector_kind("Lorg/spongepowered/asm/mixin/injection/Redirect;").unwrap();
        assert_eq!(
            redirect.default_composition(),
            crate::model::CompositionSemantics::ExclusiveOwner
        );
        let (condition, _) =
            injector_kind("Lcom/llamalad7/mixinextras/injector/WrapWithCondition;").unwrap();
        assert_eq!(
            condition.default_composition(),
            crate::model::CompositionSemantics::AdjacentInsertion
        );
        let (value, _) =
            injector_kind("Lcom/llamalad7/mixinextras/injector/ModifyExpressionValue;").unwrap();
        assert_eq!(value, MutationKind::TransformExpressionValue);
    }

    #[test]
    fn wrap_method_resolves_as_a_composable_method_effect_without_at() {
        let mut scanned = mixin_fixture(
            "HEAD",
            "Lcom/llamalad7/mixinextras/injector/wrapmethod/WrapMethod;",
        );
        let handler = &mut scanned.artifacts[1].classes[0].methods[0];
        handler.annotations[0].values.remove("at");

        let effects = analyze(&mut scanned);
        let effect = effects
            .iter()
            .find(|effect| effect.mechanism == Mechanism::MixinExtras)
            .unwrap();

        assert_eq!(effect.precision, Precision::Method);
        assert_eq!(effect.requirements[0].kind, RequirementKind::MemberExists);
        assert_eq!(effect.mutations[0].kind, MutationKind::WrapOperation);
        assert_eq!(
            effect.mutations[0].composition,
            crate::model::CompositionSemantics::OperationWrapper
        );
        assert!(
            !scanned
                .warnings
                .iter()
                .any(|warning| warning.message.contains("no recoverable @At"))
        );
    }

    #[test]
    fn modify_constant_uses_constant_discriminator_without_at() {
        let mut scanned = mixin_fixture(
            "HEAD",
            "Lorg/spongepowered/asm/mixin/injection/ModifyConstant;",
        );
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![ParsedInstruction {
            reference: InstructionReference {
                stable_id: 0,
                original_offset: Some(0),
                opcode: 8,
                member: None,
                constant: Some("5".to_string()),
            },
            kind: InstructionKind::IntegerConstant(5),
        }];
        let handler = &mut scanned.artifacts[1].classes[0].methods[0];
        handler.descriptor = "(I)I".to_string();
        let injector = &mut handler.annotations[0];
        injector.values.remove("at");
        injector.values.insert(
            "constant".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::Annotation(Box::new(
                ParsedAnnotation {
                    descriptor: "Lorg/spongepowered/asm/mixin/injection/Constant;".to_string(),
                    values: BTreeMap::from([("intValue".to_string(), AnnotationValue::Integer(5))]),
                },
            ))]),
        );

        let effects = analyze(&mut scanned);
        let effect = effects
            .iter()
            .find(|effect| {
                effect
                    .mutations
                    .iter()
                    .any(|mutation| mutation.kind == MutationKind::ModifyConstant)
            })
            .unwrap();

        assert_eq!(effect.precision, Precision::Instruction);
        assert_eq!(
            effect.mutations[0]
                .target
                .instruction
                .as_ref()
                .unwrap()
                .constant
                .as_deref(),
            Some("5")
        );
    }

    #[test]
    fn invoke_string_is_known_and_matches_the_exact_ldc_before_the_call() {
        let mut scanned = mixin_fixture(
            "INVOKE_STRING",
            "Lorg/spongepowered/asm/mixin/injection/Inject;",
        );
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![
            string_instruction(0, "needle"),
            call_instruction(1, "game/Owner", "run"),
            return_instruction(2),
        ];
        let at = fixture_at_mut(&mut scanned);
        at.values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );
        at.values.insert(
            "args".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String("ldc=needle".to_string())]),
        );

        let effects = analyze(&mut scanned);
        let effect = effects
            .iter()
            .find(|effect| effect.precision == Precision::Instruction)
            .unwrap();

        assert!(effect.mutations.iter().any(|mutation| {
            mutation
                .target
                .instruction
                .as_ref()
                .is_some_and(|instruction| instruction.stable_id == 1)
        }));
        assert!(
            !scanned
                .warnings
                .iter()
                .any(|warning| warning.kind == WarningKind::CustomInjectionPoint)
        );
    }

    #[test]
    fn mixinextras_expression_is_known_unsupported_and_preserves_mutation_semantics() {
        let mut scanned = mixin_fixture(
            "MIXINEXTRAS:EXPRESSION",
            "Lcom/llamalad7/mixinextras/injector/ModifyExpressionValue;",
        );

        let effects = analyze(&mut scanned);
        let effect = effects
            .iter()
            .find(|effect| {
                effect
                    .mutations
                    .iter()
                    .any(|mutation| mutation.kind == MutationKind::TransformExpressionValue)
            })
            .unwrap();

        assert_eq!(effect.precision, Precision::Method);
        assert_eq!(
            effect.mutations[0].kind,
            MutationKind::TransformExpressionValue
        );
        assert!(
            scanned
                .warnings
                .iter()
                .any(|warning| warning.kind == WarningKind::KnownUnsupportedInjectionPoint)
        );
        assert!(
            !scanned
                .warnings
                .iter()
                .any(|warning| warning.kind == WarningKind::CustomInjectionPoint)
        );
    }

    #[test]
    fn invoke_assign_is_known_unsupported_instead_of_a_fake_exact_invoke() {
        assert_eq!(
            injection_point_support("INVOKE_ASSIGN"),
            InjectionPointSupport::KnownUnsupported
        );
    }

    #[test]
    fn new_only_matches_the_new_opcode() {
        let method = ParsedMethod {
            name: "make".to_string(),
            descriptor: "()V".to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: Vec::new(),
            max_locals: Some(1),
            instructions: vec![
                type_instruction(0, 187, "game/Thing"),
                type_instruction(1, 192, "game/Thing"),
                type_instruction(2, 193, "game/Thing"),
            ],
        };
        let at = at_annotation("NEW", Some("Lgame/Thing;<init>()V"), Vec::new(), None, None);

        let matches = match_instructions(
            &method,
            &at,
            "NEW",
            &["Lgame/Thing;<init>()V".to_string()],
            None,
        );

        assert_eq!(
            matches
                .iter()
                .map(|instruction| instruction.reference.stable_id)
                .collect::<Vec<_>>(),
            vec![0]
        );
    }

    #[test]
    fn constant_at_arguments_use_exact_key_value_matching() {
        let method = ParsedMethod {
            name: "constant".to_string(),
            descriptor: "()V".to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: Vec::new(),
            max_locals: Some(1),
            instructions: vec![integer_instruction(0, 0), integer_instruction(1, 10)],
        };
        let at = at_annotation("CONSTANT", None, vec!["intValue=10"], None, None);

        let matches = match_instructions(&method, &at, "CONSTANT", &[], None);

        assert_eq!(
            matches
                .iter()
                .map(|instruction| instruction.reference.stable_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn named_slice_limits_matching_and_applies_boundary_ordinal_first() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![
            call_instruction(0, "game/Owner", "run"),
            call_instruction(1, "game/Owner", "run"),
            call_instruction(2, "game/Owner", "run"),
            return_instruction(3),
        ];
        let at = fixture_at_mut(&mut scanned);
        at.values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );
        at.values.insert(
            "slice".to_string(),
            AnnotationValue::String("last".to_string()),
        );
        at.values
            .insert("ordinal".to_string(), AnnotationValue::Integer(0));
        let injector = &mut scanned.artifacts[1].classes[0].methods[0].annotations[0];
        injector.values.insert(
            "slice".to_string(),
            AnnotationValue::Array(vec![
                AnnotationValue::Annotation(Box::new(slice_annotation("first", 0, 0))),
                AnnotationValue::Annotation(Box::new(slice_annotation("last", 2, 2))),
            ]),
        );

        let effects = analyze(&mut scanned);
        let effect = effects
            .iter()
            .find(|effect| effect.precision == Precision::Instruction)
            .unwrap();
        let selected = effect
            .mutations
            .iter()
            .filter_map(|mutation| mutation.target.instruction.as_ref())
            .map(|instruction| instruction.stable_id)
            .collect::<BTreeSet<_>>();

        assert_eq!(selected, BTreeSet::from([2]));
        assert_eq!(effect.queries[0].slice.as_deref(), Some("last"));
        assert_eq!(effect.queries[0].slice_start, Some(2));
        assert_eq!(effect.queries[0].slice_end, Some(2));
    }

    #[test]
    fn unresolved_slice_never_falls_back_to_instruction_precision() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        fixture_at_mut(&mut scanned).values.insert(
            "slice".to_string(),
            AnnotationValue::String("missing".to_string()),
        );

        let effects = analyze(&mut scanned);
        let injector = effects
            .iter()
            .find(|effect| {
                effect
                    .mutations
                    .iter()
                    .any(|mutation| mutation.kind == MutationKind::InsertInstructions)
            })
            .unwrap();

        assert_eq!(injector.precision, Precision::Method);
        assert!(
            injector
                .mutations
                .iter()
                .all(|mutation| mutation.precision == Precision::Method)
        );
    }

    #[test]
    fn unrelated_refmap_does_not_change_direct_reference_resolution() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[1].refmaps.push(RefmapEntry {
            path: "unrelated.refmap.json".to_string(),
            context: None,
            mixin_class: "other/Mixin".to_string(),
            original: "other()V".to_string(),
            mapped: "a()V".to_string(),
        });

        let effects = analyze(&mut scanned);
        let injector = effects
            .iter()
            .find(|effect| effect.precision == Precision::Instruction)
            .unwrap();

        assert_eq!(injector.confidence, Confidence::Exact);
        assert_eq!(
            injector.evidence[0].resolution_kind,
            Some(SoftReferenceResolution::DirectExact)
        );
    }

    #[test]
    fn genuinely_unresolved_at_reference_emits_a_specific_warning() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        fixture_at_mut(&mut scanned).values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Missing;run()V".to_string()),
        );

        let _ = analyze(&mut scanned);

        assert!(
            scanned
                .warnings
                .iter()
                .any(|warning| warning.kind == WarningKind::UnresolvedSoftReference)
        );
    }

    #[test]
    fn mixin_unique_and_synthetic_renames_do_not_create_false_member_collisions() {
        let unique = ParsedAnnotation {
            descriptor: UNIQUE.to_string(),
            values: BTreeMap::new(),
        };
        let method = |name: &str, is_public: bool, is_synthetic: bool, unique: bool| ParsedMethod {
            name: name.to_string(),
            descriptor: "()V".to_string(),
            is_static: false,
            is_public,
            is_synthetic,
            annotations: unique.then(unique_annotation).into_iter().collect(),
            max_locals: Some(1),
            instructions: Vec::new(),
        };
        let mixin = ParsedClass {
            minor: 0,
            major: 61,
            future_version_best_effort: false,
            name: "example/Mixin".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            is_interface: false,
            annotations: Vec::new(),
            fields: vec![
                ParsedField {
                    name: "privateUnique".to_string(),
                    descriptor: "I".to_string(),
                    is_static: false,
                    is_private_or_protected: true,
                    annotations: vec![unique.clone()],
                },
                ParsedField {
                    name: "publicUnique".to_string(),
                    descriptor: "I".to_string(),
                    is_static: false,
                    is_private_or_protected: false,
                    annotations: vec![unique],
                },
            ],
            methods: vec![
                method("lambda$run$0", false, true, false),
                method("privateUnique", false, false, true),
                method("publicUnique", true, false, true),
                method("regular", true, false, false),
            ],
        };
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: vec![mixin.clone()],
            refmaps: Vec::new(),
        };
        let mut effects = Vec::new();

        analyze_mixin_structure(
            &artifact,
            &mixin,
            &["game/Target".to_string()],
            1000,
            &mut effects,
        );

        let names = effects
            .iter()
            .filter_map(|effect| {
                effect
                    .target
                    .member
                    .as_ref()
                    .map(|member| member.name.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["publicUnique", "publicUnique", "regular"]);
    }

    fn unique_annotation() -> ParsedAnnotation {
        ParsedAnnotation {
            descriptor: UNIQUE.to_string(),
            values: BTreeMap::new(),
        }
    }

    fn fixture_at_mut(scanned: &mut ScannedArtifacts) -> &mut ParsedAnnotation {
        let injector = &mut scanned.artifacts[1].classes[0].methods[0].annotations[0];
        let AnnotationValue::Array(ats) = injector.values.get_mut("at").unwrap() else {
            panic!("fixture @At must be an array");
        };
        let AnnotationValue::Annotation(at) = &mut ats[0] else {
            panic!("fixture @At must be an annotation");
        };
        at
    }

    fn at_annotation(
        kind: &str,
        target: Option<&str>,
        args: Vec<&str>,
        ordinal: Option<i64>,
        slice: Option<&str>,
    ) -> ParsedAnnotation {
        let mut values = BTreeMap::from([(
            "value".to_string(),
            AnnotationValue::String(kind.to_string()),
        )]);
        if let Some(target) = target {
            values.insert(
                "target".to_string(),
                AnnotationValue::String(target.to_string()),
            );
        }
        if !args.is_empty() {
            values.insert(
                "args".to_string(),
                AnnotationValue::Array(
                    args.into_iter()
                        .map(|value| AnnotationValue::String(value.to_string()))
                        .collect(),
                ),
            );
        }
        if let Some(ordinal) = ordinal {
            values.insert("ordinal".to_string(), AnnotationValue::Integer(ordinal));
        }
        if let Some(slice) = slice {
            values.insert(
                "slice".to_string(),
                AnnotationValue::String(slice.to_string()),
            );
        }
        ParsedAnnotation {
            descriptor: "Lorg/spongepowered/asm/mixin/injection/At;".to_string(),
            values,
        }
    }

    fn slice_annotation(id: &str, from_ordinal: i64, to_ordinal: i64) -> ParsedAnnotation {
        ParsedAnnotation {
            descriptor: "Lorg/spongepowered/asm/mixin/injection/Slice;".to_string(),
            values: BTreeMap::from([
                ("id".to_string(), AnnotationValue::String(id.to_string())),
                (
                    "from".to_string(),
                    AnnotationValue::Annotation(Box::new(at_annotation(
                        "INVOKE",
                        Some("Lgame/Owner;run()V"),
                        Vec::new(),
                        Some(from_ordinal),
                        None,
                    ))),
                ),
                (
                    "to".to_string(),
                    AnnotationValue::Annotation(Box::new(at_annotation(
                        "INVOKE",
                        Some("Lgame/Owner;run()V"),
                        Vec::new(),
                        Some(to_ordinal),
                        None,
                    ))),
                ),
            ]),
        }
    }

    fn call_instruction(stable_id: u32, owner: &str, name: &str) -> ParsedInstruction {
        let member = MemberReference {
            owner: owner.to_string(),
            name: name.to_string(),
            descriptor: "()V".to_string(),
            kind: MemberKind::Method,
            is_static: Some(false),
        };
        ParsedInstruction {
            reference: InstructionReference {
                stable_id,
                original_offset: Some(stable_id),
                opcode: 182,
                member: Some(member.clone()),
                constant: None,
            },
            kind: InstructionKind::MethodCall(member),
        }
    }

    fn string_instruction(stable_id: u32, value: &str) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                stable_id,
                original_offset: Some(stable_id),
                opcode: 18,
                member: None,
                constant: Some(value.to_string()),
            },
            kind: InstructionKind::StringConstant(value.to_string()),
        }
    }

    fn integer_instruction(stable_id: u32, value: i64) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                stable_id,
                original_offset: Some(stable_id),
                opcode: if value == 0 { 3 } else { 16 },
                member: None,
                constant: Some(value.to_string()),
            },
            kind: InstructionKind::IntegerConstant(value),
        }
    }

    fn type_instruction(stable_id: u32, opcode: u8, class: &str) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                stable_id,
                original_offset: Some(stable_id),
                opcode,
                member: None,
                constant: Some(class.to_string()),
            },
            kind: InstructionKind::Type(class.to_string()),
        }
    }

    fn return_instruction(stable_id: u32) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                stable_id,
                original_offset: Some(stable_id),
                opcode: 177,
                member: None,
                constant: None,
            },
            kind: InstructionKind::Return,
        }
    }

    fn mixin_fixture(at_kind: &str, injector_descriptor: &str) -> ScannedArtifacts {
        let target = ParsedClass {
            minor: 0,
            major: 61,
            future_version_best_effort: false,
            name: "game/Target".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            is_interface: false,
            annotations: Vec::new(),
            fields: Vec::new(),
            methods: vec![ParsedMethod {
                name: "tick".to_string(),
                descriptor: "()V".to_string(),
                is_static: false,
                is_public: true,
                is_synthetic: false,
                annotations: Vec::new(),
                max_locals: Some(1),
                instructions: vec![return_instruction(0)],
            }],
        };
        let at = ParsedAnnotation {
            descriptor: "Lorg/spongepowered/asm/mixin/injection/At;".to_string(),
            values: BTreeMap::from([(
                "value".to_string(),
                AnnotationValue::String(at_kind.to_string()),
            )]),
        };
        let injector = ParsedAnnotation {
            descriptor: injector_descriptor.to_string(),
            values: BTreeMap::from([
                (
                    "method".to_string(),
                    AnnotationValue::Array(vec![AnnotationValue::String("tick()V".to_string())]),
                ),
                (
                    "at".to_string(),
                    AnnotationValue::Array(vec![AnnotationValue::Annotation(Box::new(at))]),
                ),
            ]),
        };
        let mixin_annotation = ParsedAnnotation {
            descriptor: MIXIN.to_string(),
            values: BTreeMap::from([(
                "value".to_string(),
                AnnotationValue::Array(vec![AnnotationValue::Class("Lgame/Target;".to_string())]),
            )]),
        };
        let mixin = ParsedClass {
            minor: 0,
            major: 61,
            future_version_best_effort: false,
            name: "example/Mixin".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            is_interface: false,
            annotations: vec![mixin_annotation],
            fields: Vec::new(),
            methods: vec![ParsedMethod {
                name: "handler".to_string(),
                descriptor: "()V".to_string(),
                is_static: false,
                is_public: false,
                is_synthetic: false,
                annotations: vec![injector],
                max_locals: Some(1),
                instructions: Vec::new(),
            }],
        };
        ScannedArtifacts {
            artifact_reports: Vec::new(),
            artifacts: vec![
                ParsedArtifact {
                    id: "minecraft".to_string(),
                    display_name: "minecraft".to_string(),
                    kind: ArtifactKind::Minecraft,
                    classes: vec![target],
                    refmaps: Vec::new(),
                },
                ParsedArtifact {
                    id: "mod".to_string(),
                    display_name: "mod".to_string(),
                    kind: ArtifactKind::Mod,
                    classes: vec![mixin],
                    refmaps: Vec::new(),
                },
            ],
            universe: ClassUniverse::default(),
            limits: AnalysisLimits::default(),
            coverage: Coverage::default(),
            warnings: Vec::new(),
        }
    }
}
