use std::collections::BTreeSet;

use crate::classfile::{
    AnnotationValue, InstructionKind, ParsedAnnotation, ParsedClass, ParsedInstruction,
    ParsedMethod,
};
use crate::jar::{ParsedArtifact, ScannedArtifacts};
use crate::model::{
    Activation, Confidence, Effect, Evidence, Mechanism, MemberKind, MemberReference, Mutation,
    MutationKind, Precision, RequirementKind, ShapeRequirement, Target, Warning,
};

const MIXIN: &str = "Lorg/spongepowered/asm/mixin/Mixin;";
const SHADOW: &str = "Lorg/spongepowered/asm/mixin/Shadow;";
const OVERWRITE: &str = "Lorg/spongepowered/asm/mixin/Overwrite;";
const UNIQUE: &str = "Lorg/spongepowered/asm/mixin/Unique;";
const ACCESSOR: &str = "Lorg/spongepowered/asm/mixin/gen/Accessor;";
const INVOKER: &str = "Lorg/spongepowered/asm/mixin/gen/Invoker;";

pub(crate) fn analyze(scanned: &mut ScannedArtifacts) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut warnings = Vec::new();
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
        let mut warned_missing_refmap = false;
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
                    "@Mixin contains no recoverable target class",
                ));
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
                    if artifact.refmaps.is_empty() && !warned_missing_refmap {
                        warnings.push(warning(
                            artifact,
                            &mixin.name,
                            "no refmap was found; soft Mixin references retain lower-confidence original names",
                        ));
                        warned_missing_refmap = true;
                    }
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
        }
    }
    scanned.warnings.extend(warnings);
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
                    minimum_matches: Some(1),
                    maximum_matches: None,
                    ordinal: None,
                    slice: None,
                }],
                mutations: vec![Mutation {
                    kind: MutationKind::ChangeInterfaces,
                    target: Target::class(target_class),
                    exclusive: false,
                }],
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
    let Some((mutation_kind, exclusive, mechanism)) = injector_kind(&injector.descriptor) else {
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
    let group = annotation(
        &handler.annotations,
        "Lorg/spongepowered/asm/mixin/injection/Group;",
    );
    let minimum = group
        .and_then(|group| positive_u32(group.value("min")))
        .or(require)
        .or(expect);
    let maximum = group
        .and_then(|group| positive_u32(group.value("max")))
        .or(allow);
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
    let slice_ids = slices
        .iter()
        .map(|slice| {
            slice
                .value("id")
                .and_then(|value| value.strings().into_iter().next())
                .unwrap_or_else(|| "<default>".to_string())
        })
        .collect::<Vec<_>>();

    for target_class in mixin_targets {
        for raw_selector in &raw_selectors {
            let selector_candidates =
                refmap_candidates(artifact, &mixin.name, raw_selector, target_class);
            let refmap_sources = refmap_sources(artifact, &mixin.name, raw_selector, target_class);
            let mut resolved_methods = Vec::new();
            for candidate in &selector_candidates {
                resolved_methods.extend(resolve_target_methods(scanned, target_class, candidate));
            }
            resolved_methods.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| left.1.name.cmp(&right.1.name))
                    .then_with(|| left.1.descriptor.cmp(&right.1.descriptor))
            });
            resolved_methods.dedup_by(|left, right| {
                left.0 == right.0
                    && left.1.name == right.1.name
                    && left.1.descriptor == right.1.descriptor
            });
            if resolved_methods.is_empty() {
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
            let method_mapping_ambiguous = resolved_methods.len() > 1;
            for (_, target_method) in resolved_methods {
                let method_target = Target::method(
                    target_class,
                    target_method.name.clone(),
                    target_method.descriptor.clone(),
                );
                let (slice_requirements, unresolved_slices, slice_mapping_ambiguous) =
                    resolve_slice_requirements(
                        target_method,
                        &method_target,
                        &slices,
                        artifact,
                        &mixin.name,
                        target_class,
                    );
                for unresolved in unresolved_slices {
                    warnings.push(warning(
                        artifact,
                        &mixin.name,
                        &format!(
                            "slice boundary {unresolved} did not resolve in {}{}",
                            target_method.name, target_method.descriptor
                        ),
                    ));
                }
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
                        for instruction in matches {
                            let mut target = method_target.clone();
                            target.instruction = Some(instruction.reference.clone());
                            let mut requirements = vec![
                                ShapeRequirement {
                                    kind: RequirementKind::InstructionExists,
                                    target: target.clone(),
                                    minimum_matches: Some(1),
                                    maximum_matches: None,
                                    ordinal: None,
                                    slice: slice_ids.first().cloned(),
                                },
                                ShapeRequirement {
                                    kind: RequirementKind::Cardinality,
                                    target: target.clone(),
                                    minimum_matches: minimum,
                                    maximum_matches: maximum,
                                    ordinal: None,
                                    slice: slice_ids.first().cloned(),
                                },
                            ];
                            requirements.extend(slice_requirements.clone());
                            effects.push(Effect {
                                artifact_id: artifact.id.clone(),
                                mechanism,
                                target: target.clone(),
                                requirements,
                                mutations: vec![Mutation {
                                    kind: MutationKind::ModifyConstant,
                                    target,
                                    exclusive: false,
                                }],
                                evidence: vec![evidence(
                                    artifact,
                                    mixin,
                                    Some(handler),
                                    &injector.descriptor,
                                    format!(
                                        "ModifyConstant resolved to instruction {} at offset {:?}; refmap candidates: {}; refmap sources: {}; annotation values: {}",
                                        instruction.reference.stable_id,
                                        instruction.reference.original_offset,
                                        selector_candidates.join(", "),
                                        if refmap_sources.is_empty() {
                                            "none".to_string()
                                        } else {
                                            refmap_sources.join(", ")
                                        },
                                        annotation_values(injector),
                                    ),
                                )],
                                precision: Precision::Instruction,
                                confidence: if method_mapping_ambiguous
                                    || slice_mapping_ambiguous
                                {
                                    Confidence::Medium
                                } else if artifact.refmaps.is_empty() {
                                    Confidence::High
                                } else {
                                    Confidence::Exact
                                },
                                activation: Activation::Candidate,
                                priority: Some(priority),
                            });
                        }
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
                                minimum_matches: Some(1),
                                maximum_matches: None,
                                ordinal: None,
                                slice: None,
                            }],
                            mutations: vec![Mutation {
                                kind: MutationKind::WrapOperation,
                                target: method_target.clone(),
                                exclusive: false,
                            }],
                            evidence: vec![evidence(
                                artifact,
                                mixin,
                                Some(handler),
                                &injector.descriptor,
                                format!(
                                    "WrapMethod resolved to {}{}; refmap candidates: {}; refmap sources: {}; annotation values: {}",
                                    target_method.name,
                                    target_method.descriptor,
                                    selector_candidates.join(", "),
                                    if refmap_sources.is_empty() {
                                        "none".to_string()
                                    } else {
                                        refmap_sources.join(", ")
                                    },
                                    annotation_values(injector),
                                ),
                            )],
                            precision: Precision::Method,
                            confidence: if method_mapping_ambiguous {
                                Confidence::Medium
                            } else if artifact.refmaps.is_empty() {
                                Confidence::High
                            } else {
                                Confidence::Exact
                            },
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
                for at in &ats {
                    let at_kind = at
                        .value("value")
                        .and_then(|value| value.strings().into_iter().next())
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    let ordinal = at
                        .value("ordinal")
                        .and_then(AnnotationValue::integer)
                        .filter(|value| *value >= 0)
                        .and_then(|value| u32::try_from(value).ok());
                    let (at_candidates, at_refmap_sources) =
                        at_selector_candidates(artifact, &mixin.name, at, target_class);
                    let at_mapping_ambiguous =
                        matching_at_candidate_count(target_method, at, &at_kind, &at_candidates)
                            > 1;
                    let matches = match_instructions(target_method, at, &at_kind, &at_candidates);
                    let known_point = matches!(
                        at_kind.as_str(),
                        "HEAD"
                            | "TAIL"
                            | "RETURN"
                            | "INVOKE"
                            | "INVOKE_ASSIGN"
                            | "FIELD"
                            | "NEW"
                            | "CONSTANT"
                            | "JUMP"
                            | "LOAD"
                            | "STORE"
                    );
                    if !known_point {
                        effects.push(degraded_injector_effect(
                            artifact,
                            mixin,
                            handler,
                            injector,
                            method_target.clone(),
                            MutationKind::UnknownMethod,
                            mechanism,
                            priority,
                            &format!("custom InjectionPoint '{at_kind}' is not understood"),
                        ));
                        warnings.push(warning(
                            artifact,
                            &mixin.name,
                            &format!(
                                "custom InjectionPoint '{at_kind}' degraded {}{} to method precision",
                                target_method.name, target_method.descriptor
                            ),
                        ));
                        continue;
                    }
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
                    if selected.is_empty() {
                        effects.push(degraded_injector_effect(
                            artifact,
                            mixin,
                            handler,
                            injector,
                            method_target.clone(),
                            mutation_kind,
                            mechanism,
                            priority,
                            &format!("@At({at_kind}) has no match in the original target method"),
                        ));
                        continue;
                    }
                    for instruction in selected {
                        let mut target = method_target.clone();
                        target.instruction = Some(instruction.reference.clone());
                        let mut requirements = vec![
                            ShapeRequirement {
                                kind: RequirementKind::InstructionExists,
                                target: target.clone(),
                                minimum_matches: Some(1),
                                maximum_matches: None,
                                ordinal,
                                slice: slice_ids.first().cloned(),
                            },
                            ShapeRequirement {
                                kind: RequirementKind::Cardinality,
                                target: target.clone(),
                                minimum_matches: minimum,
                                maximum_matches: maximum,
                                ordinal,
                                slice: slice_ids.first().cloned(),
                            },
                        ];
                        requirements.extend(slice_requirements.clone());
                        if locals || mutation_kind == MutationKind::ModifyLocal {
                            requirements.push(ShapeRequirement {
                                kind: RequirementKind::LocalLayout,
                                target: method_target.clone(),
                                minimum_matches: None,
                                maximum_matches: None,
                                ordinal,
                                slice: None,
                            });
                        }
                        if matches!(at_kind.as_str(), "RETURN" | "TAIL") {
                            requirements.push(ShapeRequirement {
                                kind: RequirementKind::ControlFlow,
                                target: method_target.clone(),
                                minimum_matches: Some(1),
                                maximum_matches: None,
                                ordinal,
                                slice: None,
                            });
                        }
                        let mut mutations = vec![Mutation {
                            kind: mutation_kind,
                            target: target.clone(),
                            exclusive,
                        }];
                        if cancellable {
                            mutations.push(Mutation {
                                kind: MutationKind::ChangeControlFlow,
                                target: method_target.clone(),
                                exclusive: false,
                            });
                        }
                        effects.push(Effect {
                            artifact_id: artifact.id.clone(),
                            mechanism,
                            target: target.clone(),
                            requirements,
                            mutations,
                            evidence: vec![evidence(
                                artifact,
                                mixin,
                                Some(handler),
                                &injector.descriptor,
                                format!(
                                    "@At({at_kind}) resolved to instruction {} at offset {:?}; refmap candidates: method=[{}], at=[{}]; refmap sources: {}; annotation values: {}",
                                    instruction.reference.stable_id,
                                    instruction.reference.original_offset,
                                    selector_candidates.join(", "),
                                    at_candidates.join(", "),
                                    if refmap_sources.is_empty() && at_refmap_sources.is_empty() {
                                        "none".to_string()
                                    } else {
                                        refmap_sources
                                            .iter()
                                            .chain(&at_refmap_sources)
                                            .cloned()
                                            .collect::<BTreeSet<_>>()
                                            .into_iter()
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    },
                                    annotation_values(injector),
                                ),
                            )],
                            precision: Precision::Instruction,
                            confidence: if method_mapping_ambiguous
                                || slice_mapping_ambiguous
                                || at_mapping_ambiguous
                            {
                                Confidence::Medium
                            } else if artifact.refmaps.is_empty() {
                                Confidence::High
                            } else {
                                Confidence::Exact
                            },
                            activation: Activation::Candidate,
                            priority: Some(priority),
                        });
                    }
                }
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
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
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
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
        mutations: vec![Mutation {
            kind,
            target,
            exclusive: kind == MutationKind::ReplaceMethodBody,
        }],
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
            minimum_matches: None,
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
        mutations: vec![Mutation {
            kind: match precision {
                Precision::Method => MutationKind::UnknownMethod,
                _ => MutationKind::UnknownClass,
            },
            target,
            exclusive: false,
        }],
        evidence: vec![evidence(
            artifact,
            mixin,
            Some(handler),
            &injector.descriptor,
            format!("{reason}; declared mutation {mutation:?}"),
        )],
        precision,
        confidence: Confidence::Low,
        activation: Activation::Candidate,
        priority: Some(priority),
    }
}

fn injector_kind(descriptor: &str) -> Option<(MutationKind, bool, Mechanism)> {
    let simple = injector_simple_name(descriptor);
    let mixin_extras = descriptor.contains("mixinextras");
    let mechanism = if mixin_extras {
        Mechanism::MixinExtras
    } else {
        Mechanism::Mixin
    };
    Some(match simple {
        "Inject" => (MutationKind::InsertInstructions, false, mechanism),
        "Redirect" => (MutationKind::RedirectOperation, true, mechanism),
        "ModifyArg" | "ModifyArgs" => (MutationKind::ModifyArgument, false, mechanism),
        "ModifyVariable" => (MutationKind::ModifyLocal, false, mechanism),
        "ModifyConstant" => (MutationKind::ModifyConstant, false, mechanism),
        "WrapOperation" | "WrapMethod" => (MutationKind::WrapOperation, false, mechanism),
        "ModifyExpressionValue" | "ModifyReturnValue" => {
            (MutationKind::ReplaceInstruction, false, mechanism)
        }
        "WrapWithCondition" => (MutationKind::ChangeControlFlow, false, mechanism),
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
) -> Vec<&'a ParsedInstruction> {
    let targets = target_candidates
        .iter()
        .map(|value| parse_selector(value))
        .collect::<Vec<_>>();
    let mut matches = match kind {
        "HEAD" => method.instructions.first().into_iter().collect(),
        "TAIL" => method
            .instructions
            .iter()
            .rev()
            .find(|instruction| matches!(instruction.kind, InstructionKind::Return))
            .into_iter()
            .collect(),
        "RETURN" => method
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Return))
            .collect(),
        "INVOKE" | "INVOKE_ASSIGN" => method
            .instructions
            .iter()
            .filter(|instruction| {
                matches!(&instruction.kind, InstructionKind::MethodCall(member) if targets.is_empty() || targets.iter().any(|target| selector_matches_member(target, member)))
            })
            .collect(),
        "FIELD" => method
            .instructions
            .iter()
            .filter(|instruction| {
                matches!(&instruction.kind, InstructionKind::FieldRead(member) | InstructionKind::FieldWrite(member) if targets.is_empty() || targets.iter().any(|target| selector_matches_member(target, member)))
            })
            .collect(),
        "NEW" => method
            .instructions
            .iter()
            .filter(|instruction| {
                matches!(&instruction.kind, InstructionKind::Type(class) if targets.is_empty() || targets.iter().any(|target| target.owner.as_deref().is_none_or(|owner| owner == class)))
            })
            .collect(),
        "CONSTANT" => {
            let constant = at
                .value("args")
                .map(AnnotationValue::strings)
                .unwrap_or_default();
            method
                .instructions
                .iter()
                .filter(|instruction| match &instruction.kind {
                    InstructionKind::StringConstant(value) => {
                        constant.is_empty() || constant.iter().any(|arg| arg.ends_with(value))
                    }
                    InstructionKind::IntegerConstant(value) => {
                        constant.is_empty()
                            || constant
                                .iter()
                                .any(|arg| arg.ends_with(&value.to_string()))
                    }
                    _ => false,
                })
                .collect()
        }
        "JUMP" => method
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Jump))
            .collect(),
        "LOAD" => method
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, InstructionKind::Load(_)))
            .collect(),
        "STORE" => method
            .instructions
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
    apply_shift(method, at, matches)
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

fn resolve_slice_requirements(
    method: &ParsedMethod,
    method_target: &Target,
    slices: &[&ParsedAnnotation],
    artifact: &ParsedArtifact,
    mixin_class: &str,
    target_class: &str,
) -> (Vec<ShapeRequirement>, Vec<String>, bool) {
    let mut requirements = Vec::new();
    let mut unresolved = Vec::new();
    let mut mapping_ambiguous = false;
    for slice in slices {
        let id = slice
            .value("id")
            .and_then(|value| value.strings().into_iter().next())
            .unwrap_or_else(|| "<default>".to_string());
        for boundary_name in ["from", "to"] {
            let boundary = slice
                .value(boundary_name)
                .map(AnnotationValue::annotations)
                .unwrap_or_default();
            if boundary.is_empty() {
                let default_boundary = if boundary_name == "from" {
                    method.instructions.first()
                } else {
                    method
                        .instructions
                        .iter()
                        .rev()
                        .find(|instruction| matches!(instruction.kind, InstructionKind::Return))
                };
                let mut target = method_target.clone();
                target.instruction =
                    default_boundary.map(|instruction| instruction.reference.clone());
                requirements.push(ShapeRequirement {
                    kind: RequirementKind::SliceBoundary,
                    target,
                    minimum_matches: Some(1),
                    maximum_matches: None,
                    ordinal: None,
                    slice: Some(id.clone()),
                });
                if default_boundary.is_none() {
                    unresolved.push(format!("{id}.{boundary_name}=default"));
                }
                continue;
            }
            for at in boundary {
                let kind = at
                    .value("value")
                    .and_then(|value| value.strings().into_iter().next())
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                let ordinal = at
                    .value("ordinal")
                    .and_then(AnnotationValue::integer)
                    .filter(|value| *value >= 0)
                    .and_then(|value| u32::try_from(value).ok());
                let (target_candidates, _) =
                    at_selector_candidates(artifact, mixin_class, at, target_class);
                mapping_ambiguous |=
                    matching_at_candidate_count(method, at, &kind, &target_candidates) > 1;
                let matches = match_instructions(method, at, &kind, &target_candidates);
                if matches.is_empty() {
                    unresolved.push(format!("{id}.{boundary_name}=@At({kind})"));
                    requirements.push(ShapeRequirement {
                        kind: RequirementKind::SliceBoundary,
                        target: method_target.clone(),
                        minimum_matches: Some(1),
                        maximum_matches: None,
                        ordinal,
                        slice: Some(id.clone()),
                    });
                    continue;
                }
                for instruction in matches {
                    let mut target = method_target.clone();
                    target.instruction = Some(instruction.reference.clone());
                    requirements.push(ShapeRequirement {
                        kind: RequirementKind::SliceBoundary,
                        target,
                        minimum_matches: Some(1),
                        maximum_matches: None,
                        ordinal,
                        slice: Some(id.clone()),
                    });
                }
            }
        }
    }
    (requirements, unresolved, mapping_ambiguous)
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
            let index = isize::try_from(instruction.reference.stable_id).ok()?;
            usize::try_from(index.checked_add(delta)?)
                .ok()
                .and_then(|index| method.instructions.get(index))
        })
        .collect()
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

fn matching_at_candidate_count(
    method: &ParsedMethod,
    at: &ParsedAnnotation,
    kind: &str,
    candidates: &[String],
) -> usize {
    candidates
        .iter()
        .filter(|candidate| {
            !match_instructions(method, at, kind, std::slice::from_ref(candidate)).is_empty()
        })
        .count()
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
    Evidence {
        artifact_id: artifact.id.clone(),
        class: mixin.name.clone(),
        method: method.map(|method| format!("{}{}", method.name, method.descriptor)),
        annotation: Some(annotation.to_string()),
        instruction: None,
        detail,
    }
}

fn warning(artifact: &ParsedArtifact, scope: &str, message: &str) -> Warning {
    Warning {
        artifact_id: Some(artifact.id.clone()),
        scope: scope.to_string(),
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
    fn missing_refmap_keeps_original_selector_with_lower_confidence() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        let effects = analyze(&mut scanned);
        let injector = effects
            .iter()
            .find(|effect| effect.precision == Precision::Instruction)
            .unwrap();
        assert_eq!(injector.confidence, Confidence::High);
        assert!(
            scanned
                .warnings
                .iter()
                .any(|warning| { warning.message.contains("no refmap was found") })
        );
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
                    .any(|mutation| mutation.kind == MutationKind::UnknownMethod)
            })
            .unwrap();
        assert_eq!(injector.precision, Precision::Method);
        assert_eq!(injector.mutations[0].kind, MutationKind::UnknownMethod);
        assert!(
            scanned
                .warnings
                .iter()
                .any(|warning| { warning.message.contains("custom InjectionPoint") })
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
            effect
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
        assert!(effect.evidence[0].detail.contains("mod.refmap.json"));
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
        let (wrap, exclusive, mechanism) =
            injector_kind("Lcom/llamalad7/mixinextras/injector/wrapoperation/WrapOperation;")
                .unwrap();
        assert_eq!(wrap, MutationKind::WrapOperation);
        assert!(!exclusive);
        assert_eq!(mechanism, Mechanism::MixinExtras);
        let (_, redirect_exclusive, _) =
            injector_kind("Lorg/spongepowered/asm/mixin/injection/Redirect;").unwrap();
        assert!(redirect_exclusive);
        let (_, condition_exclusive, _) =
            injector_kind("Lcom/llamalad7/mixinextras/injector/WrapWithCondition;").unwrap();
        assert!(!condition_exclusive);
        let (_, exclusive, _) =
            injector_kind("Lcom/llamalad7/mixinextras/injector/ModifyExpressionValue;").unwrap();
        assert!(!exclusive);
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
        assert!(!effect.mutations[0].exclusive);
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
            effect
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
                instructions: vec![ParsedInstruction {
                    reference: InstructionReference {
                        stable_id: 0,
                        original_offset: Some(0),
                        opcode: 177,
                        member: None,
                        constant: None,
                    },
                    kind: InstructionKind::Return,
                }],
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
