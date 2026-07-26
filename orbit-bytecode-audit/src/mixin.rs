use std::collections::{BTreeMap, BTreeSet};

use crate::classfile::{
    AnnotationValue, InstructionKind, ParsedAnnotation, ParsedClass, ParsedInstruction,
    ParsedMethod,
};
use crate::jar::{ParsedArtifact, ScannedArtifacts};
use crate::mixin_config::MixinRegistry;
use crate::model::{
    Activation, BehavioralInteraction, BehavioralInteractionKind, Confidence, CoverageGap,
    CoverageGapKind, Effect, Evidence, FramePosition, InactiveCandidate, InactiveCandidateKind,
    InjectionGroupConstraint, InjectionQuery, LocalSelector, Mechanism, MemberKind,
    MemberReference, MethodContributionKind, Mutation, MutationKind, Precision, RequirementKind,
    ShapeRequirement, SoftReferenceResolution, Target, Warning, WarningKind,
};

const MIXIN: &str = "Lorg/spongepowered/asm/mixin/Mixin;";
const SHADOW: &str = "Lorg/spongepowered/asm/mixin/Shadow;";
const OVERWRITE: &str = "Lorg/spongepowered/asm/mixin/Overwrite;";
const UNIQUE: &str = "Lorg/spongepowered/asm/mixin/Unique;";
const ACCESSOR: &str = "Lorg/spongepowered/asm/mixin/gen/Accessor;";
const INVOKER: &str = "Lorg/spongepowered/asm/mixin/gen/Invoker;";
const PSEUDO: &str = "Lorg/spongepowered/asm/mixin/Pseudo;";
const INTRINSIC: &str = "Lorg/spongepowered/asm/mixin/Intrinsic;";

#[derive(Debug, Default)]
pub(crate) struct MixinAnalysis {
    pub effects: Vec<Effect>,
    pub unary_risks: Vec<crate::model::UnaryCompatibilityRisk>,
    pub risks: Vec<crate::model::Risk>,
    pub interactions: Vec<BehavioralInteraction>,
    pub coverage_gaps: Vec<CoverageGap>,
    pub inactive_candidates: Vec<InactiveCandidate>,
}

#[derive(Debug, Default)]
struct MixinFindings {
    coverage_gaps: Vec<CoverageGap>,
    inactive_candidates: Vec<InactiveCandidate>,
    unsupported_selectors: usize,
    unsupported_injection_points: usize,
    valid_multi_target_selectors: usize,
    optional_unresolved_references: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MethodContributionKey {
    artifact_id: String,
    mixin_class: String,
    target_class: String,
    method_name: String,
    descriptor: String,
}

#[derive(Debug, Clone)]
struct CandidateMethod {
    method: ParsedMethod,
    source_artifact: String,
    source_mixin: Option<String>,
    priority: i32,
    config_priority: i32,
    contribution: Option<MethodContributionKey>,
}

#[derive(Debug, Clone, Default)]
struct CandidateClassState {
    base_artifact: Option<String>,
    fields: BTreeMap<(String, String), crate::classfile::ParsedField>,
    base_methods: BTreeMap<(String, String), ParsedMethod>,
    declared_methods: BTreeSet<(String, String)>,
    methods: BTreeMap<(String, String), CandidateMethod>,
    interfaces: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct CandidateUniverse {
    classes: BTreeMap<String, CandidateClassState>,
    contribution_kinds: BTreeMap<MethodContributionKey, MethodContributionKind>,
    interactions: Vec<BehavioralInteraction>,
    invalid_contributions: Vec<InvalidMethodContribution>,
    ambiguous_methods: BTreeMap<(String, String, String), Vec<CandidateMethod>>,
}

#[derive(Debug, Clone)]
struct InvalidMethodContribution {
    key: MethodContributionKey,
    kind: MethodContributionKind,
}

#[derive(Debug, Clone)]
struct PendingMethodContribution {
    key: MethodContributionKey,
    method: ParsedMethod,
    priority: i32,
    config_priority: i32,
    sequence: usize,
    overwrite: bool,
    unique: bool,
    synthetic: bool,
    intrinsic: bool,
    require_overwrite_annotation: bool,
}

impl CandidateUniverse {
    fn build(scanned: &ScannedArtifacts, registry: &MixinRegistry) -> Self {
        let mut universe = Self::default();
        let mut pending = Vec::new();
        let mut sequence = 0_usize;
        for registered in registry.mixins.iter().filter(|registered| {
            matches!(
                registered.activation,
                crate::model::MixinActivation::RegisteredForCurrentSide
                    | crate::model::MixinActivation::PluginAccepted
            )
        }) {
            let Some(artifact) = scanned
                .artifacts
                .iter()
                .find(|artifact| artifact.id == registered.artifact_id)
            else {
                continue;
            };
            let Some(mixin) = artifact
                .classes
                .iter()
                .find(|class| class.name == registered.mixin_class)
            else {
                continue;
            };
            let Some(mixin_annotation) = annotation(&mixin.annotations, MIXIN) else {
                continue;
            };
            let priority = mixin_annotation
                .value("priority")
                .and_then(AnnotationValue::integer)
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(registered.class_priority);
            for target_class in mixin_targets(mixin_annotation) {
                universe.ensure_base_class(scanned, &target_class);
                if let Some(state) = universe.classes.get_mut(&target_class) {
                    state.interfaces.extend(mixin.interfaces.iter().cloned());
                    for field in &mixin.fields {
                        if annotation(&field.annotations, SHADOW).is_some() {
                            continue;
                        }
                        let key = (field.name.clone(), field.descriptor.clone());
                        let unique = annotation(&field.annotations, UNIQUE).is_some()
                            || annotation(&mixin.annotations, UNIQUE).is_some();
                        if !unique || !state.fields.contains_key(&key) {
                            state.fields.entry(key).or_insert_with(|| field.clone());
                        }
                    }
                }
                for method in &mixin.methods {
                    let key = MethodContributionKey {
                        artifact_id: artifact.id.clone(),
                        mixin_class: mixin.name.clone(),
                        target_class: target_class.clone(),
                        method_name: method.name.clone(),
                        descriptor: method.descriptor.clone(),
                    };
                    if method.name == "<init>" || method.name == "<clinit>" {
                        universe
                            .contribution_kinds
                            .insert(key, MethodContributionKind::HelperMethod);
                        continue;
                    }
                    if method
                        .annotations
                        .iter()
                        .any(|annotation| injector_kind(&annotation.descriptor).is_some())
                    {
                        universe
                            .contribution_kinds
                            .insert(key, MethodContributionKind::InjectorHandler);
                        continue;
                    }
                    if annotation(&method.annotations, SHADOW).is_some() {
                        continue;
                    }
                    if annotation(&method.annotations, ACCESSOR).is_some() {
                        universe
                            .contribution_kinds
                            .insert(key.clone(), MethodContributionKind::Accessor);
                        // Accessor bodies are generated in the ACCESSOR pass,
                        // after every Mixin's INJECT_PREPARE pass. They must
                        // not enter the method universe used to resolve
                        // wildcard injector selectors.
                        continue;
                    }
                    if annotation(&method.annotations, INVOKER).is_some() {
                        universe
                            .contribution_kinds
                            .insert(key.clone(), MethodContributionKind::Invoker);
                        continue;
                    }
                    pending.push(PendingMethodContribution {
                        key,
                        method: method.clone(),
                        priority,
                        config_priority: registered.config_priority,
                        sequence,
                        overwrite: annotation(&method.annotations, OVERWRITE).is_some(),
                        unique: annotation(&method.annotations, UNIQUE).is_some()
                            || annotation(&mixin.annotations, UNIQUE).is_some()
                            // MixinPreProcessorStandard routes every synthetic
                            // method through attachUniqueMethod, even without
                            // an explicit @Unique annotation. Private
                            // compiler-generated helpers (notably lambdas)
                            // are therefore conformed/renamed instead of
                            // overwriting a same-signature target method.
                            || method.is_synthetic,
                        synthetic: method.is_synthetic,
                        intrinsic: annotation(&method.annotations, INTRINSIC).is_some(),
                        require_overwrite_annotation: registry
                            .configs
                            .iter()
                            .find(|config| {
                                config.artifact_id == registered.artifact_id
                                    && config.config_path == registered.config_path
                            })
                            .and_then(|config| config.parsed.as_ref())
                            .is_some_and(|config| config.overwrite_require_annotations),
                    });
                    sequence += 1;
                }
            }
        }
        pending.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.config_priority.cmp(&right.config_priority))
                .then_with(|| left.sequence.cmp(&right.sequence))
        });
        for contribution in pending {
            universe.apply_method_contribution(contribution);
        }
        universe
    }

    fn ensure_base_class(&mut self, scanned: &ScannedArtifacts, class_name: &str) {
        if self.classes.contains_key(class_name) {
            return;
        }
        let definitions = scanned
            .universe
            .parsed_definitions(&scanned.artifacts, class_name)
            .into_iter()
            .collect::<Vec<_>>();
        let [(artifact, _class)] = definitions.as_slice() else {
            self.classes
                .insert(class_name.to_string(), CandidateClassState::default());
            return;
        };
        let mut hierarchy = Vec::new();
        collect_class_hierarchy(scanned, class_name, &mut BTreeSet::new(), &mut hierarchy);
        let mut fields = BTreeMap::new();
        let mut base_methods = BTreeMap::new();
        let mut methods = BTreeMap::new();
        let mut interfaces = BTreeSet::new();
        for (owner_artifact, owner_class) in hierarchy {
            interfaces.extend(owner_class.interfaces.iter().cloned());
            for field in &owner_class.fields {
                fields.insert(
                    (field.name.clone(), field.descriptor.clone()),
                    field.clone(),
                );
            }
            for method in &owner_class.methods {
                let key = (method.name.clone(), method.descriptor.clone());
                base_methods.insert(key.clone(), method.clone());
                methods.insert(
                    key,
                    CandidateMethod {
                        method: method.clone(),
                        source_artifact: owner_artifact.id.clone(),
                        source_mixin: None,
                        priority: i32::MIN,
                        config_priority: i32::MIN,
                        contribution: None,
                    },
                );
            }
        }
        let state = CandidateClassState {
            base_artifact: Some(artifact.id.clone()),
            fields,
            base_methods,
            declared_methods: _class
                .methods
                .iter()
                .map(|method| (method.name.clone(), method.descriptor.clone()))
                .collect(),
            methods,
            interfaces,
        };
        self.classes.insert(class_name.to_string(), state);
    }

    fn apply_method_contribution(&mut self, contribution: PendingMethodContribution) {
        self.classes
            .entry(contribution.key.target_class.clone())
            .or_default();
        let method_key = (
            contribution.key.method_name.clone(),
            contribution.key.descriptor.clone(),
        );
        let existing = self
            .classes
            .get(&contribution.key.target_class)
            .and_then(|state| state.methods.get(&method_key))
            .cloned();
        let target_declares_method = self
            .classes
            .get(&contribution.key.target_class)
            .is_some_and(|state| state.declared_methods.contains(&method_key));
        let equal_priority_mixin = existing.as_ref().is_some_and(|existing| {
            existing.source_mixin.is_some()
                && existing.priority == contribution.priority
                && existing.config_priority == contribution.config_priority
        });
        if equal_priority_mixin {
            let alternatives = self
                .ambiguous_methods
                .entry((
                    contribution.key.target_class.clone(),
                    contribution.key.method_name.clone(),
                    contribution.key.descriptor.clone(),
                ))
                .or_default();
            if let Some(existing) = existing.clone() {
                alternatives.push(existing);
            }
            alternatives.push(CandidateMethod {
                method: contribution.method.clone(),
                source_artifact: contribution.key.artifact_id.clone(),
                source_mixin: Some(contribution.key.mixin_class.clone()),
                priority: contribution.priority,
                config_priority: contribution.config_priority,
                contribution: Some(contribution.key.clone()),
            });
        }
        let kind = if contribution.unique && target_declares_method {
            if contribution.synthetic || !contribution.method.is_public {
                MethodContributionKind::UniqueRenamedMethod
            } else {
                MethodContributionKind::UniqueDiscardableMethod
            }
        } else if contribution.overwrite && !target_declares_method {
            MethodContributionKind::InvalidOverwriteTarget
        } else if !contribution.overwrite
            && !contribution.intrinsic
            && contribution.require_overwrite_annotation
            && target_declares_method
        {
            MethodContributionKind::MissingRequiredOverwriteAnnotation
        } else if existing.as_ref().is_some_and(|existing| {
            existing.source_mixin.is_some() && existing.priority >= contribution.priority
        }) {
            MethodContributionKind::SkippedByPriority
        } else if contribution.overwrite {
            MethodContributionKind::OverwriteExistingMethod
        } else if target_declares_method {
            MethodContributionKind::ReplaceExistingMethod
        } else {
            MethodContributionKind::AddNewMethod
        };

        if !matches!(
            kind,
            MethodContributionKind::UniqueRenamedMethod
                | MethodContributionKind::UniqueDiscardableMethod
        ) && let Some(existing) = existing.as_ref().filter(|existing| {
            existing
                .source_mixin
                .as_ref()
                .is_some_and(|_| existing.source_artifact != contribution.key.artifact_id)
        }) {
            self.interactions.push(method_interaction(
                existing,
                &contribution,
                &method_key,
                kind,
            ));
        }
        self.contribution_kinds
            .insert(contribution.key.clone(), kind);
        if matches!(
            kind,
            MethodContributionKind::UniqueRenamedMethod
                | MethodContributionKind::UniqueDiscardableMethod
                | MethodContributionKind::SkippedByPriority
                | MethodContributionKind::InvalidOverwriteTarget
                | MethodContributionKind::MissingRequiredOverwriteAnnotation
        ) {
            if matches!(
                kind,
                MethodContributionKind::InvalidOverwriteTarget
                    | MethodContributionKind::MissingRequiredOverwriteAnnotation
            ) {
                self.invalid_contributions.push(InvalidMethodContribution {
                    key: contribution.key,
                    kind,
                });
            }
            return;
        }
        let target = self
            .classes
            .get_mut(&contribution.key.target_class)
            .expect("candidate class was inserted above");
        target.declared_methods.insert(method_key.clone());
        target.methods.insert(
            method_key,
            CandidateMethod {
                method: contribution.method,
                source_artifact: contribution.key.artifact_id.clone(),
                source_mixin: Some(contribution.key.mixin_class.clone()),
                priority: contribution.priority,
                config_priority: contribution.config_priority,
                contribution: Some(contribution.key),
            },
        );
    }

    fn methods<'a>(
        &'a self,
        scanned: &'a ScannedArtifacts,
        class_name: &str,
    ) -> Vec<(String, &'a ParsedMethod)> {
        if let Some(state) = self.classes.get(class_name) {
            return state
                .methods
                .values()
                .map(|method| (method.source_artifact.clone(), &method.method))
                .collect();
        }
        scanned
            .universe
            .parsed_definitions(&scanned.artifacts, class_name)
            .into_iter()
            .flat_map(|(artifact, class)| {
                class
                    .methods
                    .iter()
                    .map(move |method| (artifact.id.clone(), method))
            })
            .collect()
    }

    fn contribution_kind(
        &self,
        artifact: &ParsedArtifact,
        mixin: &ParsedClass,
        target_class: &str,
        method: &ParsedMethod,
    ) -> Option<MethodContributionKind> {
        self.contribution_kinds
            .get(&MethodContributionKey {
                artifact_id: artifact.id.clone(),
                mixin_class: mixin.name.clone(),
                target_class: target_class.to_string(),
                method_name: method.name.clone(),
                descriptor: method.descriptor.clone(),
            })
            .copied()
    }

    fn method(&self, target: &Target) -> Option<&CandidateMethod> {
        let member = target.member.as_ref()?;
        self.classes
            .get(&target.class)?
            .methods
            .get(&(member.name.clone(), member.descriptor.clone()))
    }

    fn method_variants(&self, target: &Target) -> Vec<&CandidateMethod> {
        let Some(member) = target.member.as_ref() else {
            return Vec::new();
        };
        let key = (
            target.class.clone(),
            member.name.clone(),
            member.descriptor.clone(),
        );
        if let Some(alternatives) = self.ambiguous_methods.get(&key) {
            let mut variants = alternatives.iter().collect::<Vec<_>>();
            variants.sort_by(|left, right| {
                left.source_artifact
                    .cmp(&right.source_artifact)
                    .then_with(|| left.source_mixin.cmp(&right.source_mixin))
            });
            variants.dedup_by(|left, right| {
                left.source_artifact == right.source_artifact
                    && left.source_mixin == right.source_mixin
            });
            variants
        } else {
            self.method(target).into_iter().collect()
        }
    }

    fn base_method(&self, target: &Target) -> Option<&ParsedMethod> {
        let member = target.member.as_ref()?;
        self.classes
            .get(&target.class)?
            .base_methods
            .get(&(member.name.clone(), member.descriptor.clone()))
    }

    fn is_winning_contribution(
        &self,
        artifact: &ParsedArtifact,
        mixin: &ParsedClass,
        target_class: &str,
        method: &ParsedMethod,
    ) -> bool {
        self.classes
            .get(target_class)
            .and_then(|state| {
                state
                    .methods
                    .get(&(method.name.clone(), method.descriptor.clone()))
            })
            .and_then(|candidate| candidate.contribution.as_ref())
            .is_some_and(|key| {
                key.artifact_id == artifact.id
                    && key.mixin_class == mixin.name
                    && key.target_class == target_class
            })
    }
}

fn collect_class_hierarchy<'a>(
    scanned: &'a ScannedArtifacts,
    class_name: &str,
    visited: &mut BTreeSet<String>,
    output: &mut Vec<(&'a ParsedArtifact, &'a ParsedClass)>,
) {
    if !visited.insert(class_name.to_string()) {
        return;
    }
    let definitions = scanned
        .universe
        .parsed_definitions(&scanned.artifacts, class_name);
    let [(artifact, class)] = definitions.as_slice() else {
        return;
    };
    if let Some(super_name) = &class.super_name {
        collect_class_hierarchy(scanned, super_name, visited, output);
    }
    for interface in &class.interfaces {
        collect_class_hierarchy(scanned, interface, visited, output);
    }
    output.push((*artifact, *class));
}

fn method_interaction(
    existing: &CandidateMethod,
    incoming: &PendingMethodContribution,
    method_key: &(String, String),
    result: MethodContributionKind,
) -> BehavioralInteraction {
    let target = Target::method(&incoming.key.target_class, &method_key.0, &method_key.1);
    let mut left = Evidence::new(
        &existing.source_artifact,
        existing.source_mixin.as_deref().unwrap_or("<base>"),
        "earlier Mixin method contribution",
    );
    left.method = Some(format!("{}{}", method_key.0, method_key.1));
    let mut right = Evidence::new(
        &incoming.key.artifact_id,
        &incoming.key.mixin_class,
        format!("later contribution resolved as {result:?}"),
    );
    right.method = Some(format!("{}{}", method_key.0, method_key.1));
    BehavioralInteraction {
        left_artifact: existing.source_artifact.clone(),
        right_artifact: incoming.key.artifact_id.clone(),
        target,
        kind: BehavioralInteractionKind::OrderedMethodContributions,
        reason: format!(
            "Mixin priority/order selects one of multiple method contributions ({result:?})"
        ),
        evidence: vec![left, right],
        confidence: Confidence::Exact,
        activation: Activation::Definite,
        order: crate::model::OrderAnalysis::LeftMustRunFirst,
    }
}

fn candidate_query_findings(
    effects: &[Effect],
    candidates: &CandidateUniverse,
) -> (Vec<crate::model::Risk>, Vec<BehavioralInteraction>) {
    let mut risks = Vec::new();
    let mut interactions = Vec::new();
    for effect in effects {
        for query in &effect.queries {
            let Some(method) = candidates.method(&query.method) else {
                continue;
            };
            let Some(contribution) = method.contribution.as_ref() else {
                continue;
            };
            let is_replacement =
                candidates
                    .contribution_kinds
                    .get(contribution)
                    .is_some_and(|kind| {
                        matches!(
                            kind,
                            MethodContributionKind::ReplaceExistingMethod
                                | MethodContributionKind::OverwriteExistingMethod
                        )
                    });
            if !is_replacement || method.source_artifact == effect.artifact_id {
                continue;
            }
            let selected = u32::try_from(query.selected.len()).unwrap_or(u32::MAX);
            let minimum = query.minimum_matches;
            let variant_counts = candidates
                .method_variants(&query.method)
                .into_iter()
                .filter(|variant| variant.source_mixin.is_some())
                .filter_map(|variant| {
                    query_matches_candidate(query, &variant.method)
                        .map(|matches| (variant, matches))
                })
                .collect::<Vec<_>>();
            let mixed_outcome = minimum.is_some_and(|minimum| {
                variant_counts.iter().any(|(_, matches)| *matches < minimum)
                    && variant_counts
                        .iter()
                        .any(|(_, matches)| *matches >= minimum)
            });
            let hard_failure = mixed_outcome
                || minimum.is_some_and(|minimum| {
                    if variant_counts.len() > 1 {
                        variant_counts.iter().all(|(_, matches)| *matches < minimum)
                    } else {
                        selected < minimum
                    }
                });
            let failing_method = variant_counts
                .iter()
                .find(|(_, matches)| minimum.is_some_and(|minimum| *matches < minimum))
                .map_or(method, |(variant, _)| *variant);
            let mut source_evidence = Evidence::new(
                &failing_method.source_artifact,
                failing_method
                    .source_mixin
                    .as_deref()
                    .unwrap_or("<unknown mixin>"),
                "this Mixin contribution supplies the candidate replacement body",
            );
            source_evidence.method = query
                .method
                .member
                .as_ref()
                .map(|member| format!("{}{}", member.name, member.descriptor));
            if hard_failure {
                let mut evidence = effect.evidence.clone();
                evidence.push(source_evidence);
                let confidence = effect.confidence;
                let activation = if mixed_outcome {
                    Activation::Conditional
                } else {
                    effect.activation
                };
                let severity = crate::model::Severity::High;
                risks.push(crate::model::Risk {
                    left_artifact: failing_method.source_artifact.clone(),
                    right_artifact: effect.artifact_id.clone(),
                    target: query.method.clone(),
                    rule: "candidate_query_minimum_unsatisfied".to_string(),
                    reason: if mixed_outcome {
                        format!(
                            "Some finite candidate Mixin orders satisfy require {}, while another recovered replacement body does not.",
                            minimum.unwrap_or_default()
                        )
                    } else {
                        format!(
                            "The final candidate method body matches {} join point(s), below the injector require value {}.",
                            selected,
                            minimum.unwrap_or_default()
                        )
                    },
                    left_mutations: vec![MutationKind::ReplaceMethodBody],
                    right_mutations: effect
                        .mutations
                        .iter()
                        .map(|mutation| mutation.kind)
                        .collect(),
                    evidence,
                    order: if mixed_outcome {
                        crate::model::OrderAnalysis::Unknown
                    } else {
                        crate::model::OrderAnalysis::CardinalityInvalidated
                    },
                    severity,
                    confidence,
                    precision: effect.precision,
                    risk_index: effective_risk_index(severity, confidence, activation),
                    activation,
                });
            } else if selected == 0
                && query.minimum_matches.is_none_or(|minimum| minimum == 0)
                && candidates
                    .base_method(&query.method)
                    .is_some_and(|base| instruction_body_changed(base, &method.method))
            {
                let mut evidence = effect.evidence.clone();
                evidence.push(source_evidence);
                interactions.push(BehavioralInteraction {
                    left_artifact: method.source_artifact.clone(),
                    right_artifact: effect.artifact_id.clone(),
                    target: query.method.clone(),
                    kind: BehavioralInteractionKind::OptionalInjectionAffected,
                    reason: "The optional injector has no join point in the final replacement body; Mixin application remains valid."
                        .to_string(),
                    evidence,
                    confidence: effect.confidence,
                    activation: effect.activation,
                    order: crate::model::OrderAnalysis::LeftMustRunFirst,
                });
            }
        }
    }
    (risks, interactions)
}

fn query_matches_candidate(query: &InjectionQuery, method: &ParsedMethod) -> Option<u32> {
    if query.slice.is_some() || query.shift.is_some() {
        return None;
    }
    if query
        .local_selector
        .as_ref()
        .is_some_and(|selector| selector.args_only && selector.slot.is_some())
        && query.selector_kind == "HEAD"
    {
        return Some(1);
    }
    if query.selector_kind == "CONSTANT" {
        return None;
    }
    let mut matched = BTreeSet::new();
    for kind in query
        .selector_kind
        .split('+')
        .filter(|kind| !kind.is_empty())
    {
        let mut values = BTreeMap::from([(
            "value".to_string(),
            AnnotationValue::String(kind.to_string()),
        )]);
        if let Some(ordinal) = query.ordinal {
            values.insert(
                "ordinal".to_string(),
                AnnotationValue::Integer(i64::from(ordinal)),
            );
        }
        let at = ParsedAnnotation {
            descriptor: "Lorg/spongepowered/asm/mixin/injection/At;".to_string(),
            values,
        };
        let matches = match_instructions(method, &at, kind, &query.target_selectors, None);
        let matches = query.ordinal.map_or(matches.clone(), |ordinal| {
            matches
                .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
                .copied()
                .into_iter()
                .collect()
        });
        matched.extend(
            matches
                .into_iter()
                .map(|instruction| instruction.reference.stable_id),
        );
    }
    u32::try_from(matched.len()).ok()
}

fn instruction_body_changed(left: &ParsedMethod, right: &ParsedMethod) -> bool {
    left.instructions.len() != right.instructions.len()
        || left
            .instructions
            .iter()
            .zip(&right.instructions)
            .any(|(left, right)| {
                left.reference.opcode != right.reference.opcode
                    || left.reference.member != right.reference.member
                    || left.reference.constant != right.reference.constant
            })
}

fn effective_risk_index(
    severity: crate::model::Severity,
    confidence: Confidence,
    activation: Activation,
) -> u8 {
    let activation_factor = match activation {
        Activation::Definite => 100_u32,
        Activation::Conditional => 80,
        Activation::Candidate => 55,
        Activation::Unknown => 35,
    };
    let product = u32::from(severity.score()) * u32::from(confidence.score()) * activation_factor;
    u8::try_from(((product + 5_000) / 10_000).min(100)).unwrap_or(100)
}

#[cfg(test)]
pub(crate) fn analyze(scanned: &mut ScannedArtifacts) -> Vec<Effect> {
    analyze_detailed(scanned).effects
}

#[cfg(test)]
fn analyze_detailed(scanned: &mut ScannedArtifacts) -> MixinAnalysis {
    crate::jar::rebuild_universe(scanned);
    let registry = MixinRegistry::all_annotated(scanned);
    analyze_with_progress(scanned, &registry, None)
}

pub(crate) fn analyze_with_progress(
    scanned: &mut ScannedArtifacts,
    registry: &MixinRegistry,
    progress: Option<&crate::progress::AuditProgressReporter>,
) -> MixinAnalysis {
    use crate::progress::{AuditProgressEvent, AuditProgressStage, emit};

    let total = registry
        .mixins
        .iter()
        .filter(|mixin| {
            matches!(
                mixin.activation,
                crate::model::MixinActivation::RegisteredForCurrentSide
                    | crate::model::MixinActivation::PluginAccepted
                    | crate::model::MixinActivation::PluginControlled
            ) && scanned.artifacts.iter().any(|artifact| {
                artifact.id == mixin.artifact_id
                    && artifact.classes.iter().any(|class| {
                        class.name == mixin.mixin_class
                            && annotation(&class.annotations, MIXIN).is_some()
                    })
            })
        })
        .map(|mixin| (&mixin.artifact_id, &mixin.mixin_class))
        .collect::<BTreeSet<_>>()
        .len();
    emit(
        progress,
        AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::AnalyzeMixins,
            total: Some(total),
        },
    );
    let mut effects = Vec::new();
    let mut findings = MixinFindings::default();
    let candidates = CandidateUniverse::build(scanned, registry);
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
            let Some(registered) = registry.active_mixin(&artifact.id, &mixin.name) else {
                continue;
            };
            let plugin_controlled =
                registered.activation == crate::model::MixinActivation::PluginControlled;
            let warnings_before_mixin = warnings.len();
            scanned.coverage.mixins_discovered += 1;
            let target_classes = mixin_targets(mixin_annotation);
            if target_classes.is_empty() {
                if !plugin_controlled {
                    warnings.push(warning(
                        artifact,
                        &mixin.name,
                        WarningKind::Other,
                        "@Mixin contains no recoverable target class",
                    ));
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
                continue;
            }
            let priority = mixin_annotation
                .value("priority")
                .and_then(AnnotationValue::integer)
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(registered.class_priority);
            let pseudo = annotation(&mixin.annotations, PSEUDO).is_some();
            let target_classes = target_classes
                .into_iter()
                .filter(|target| {
                    match scanned.universe.definitions(target) {
                        [] => {
                            findings.inactive_candidates.push(InactiveCandidate {
                                artifact_id: artifact.id.clone(),
                                class: Some(mixin.name.clone()),
                                config_path: Some(registered.config_path.clone()),
                                kind: if pseudo {
                                    InactiveCandidateKind::PseudoTargetMissing
                                } else {
                                    InactiveCandidateKind::MissingTarget
                                },
                                reason: format!(
                                    "Mixin target {target} is absent from the aligned active class universe"
                                ),
                            });
                            false
                        }
                        [_] => true,
                        definitions => {
                            findings.coverage_gaps.push(CoverageGap {
                                artifact_id: Some(artifact.id.clone()),
                                scope: format!("{} -> {target}", mixin.name),
                                kind: CoverageGapKind::AmbiguousClassDefinition,
                                detail: format!(
                                    "the aligned active class universe contains {} definitions of the Mixin target; no definite finding was generated",
                                    definitions.len()
                                ),
                                count: definitions.len(),
                            });
                            false
                        }
                    }
                })
                .collect::<Vec<_>>();
            if target_classes.is_empty() {
                completed += 1;
                continue;
            }
            let first_effect = effects.len();
            analyze_mixin_structure(
                artifact,
                mixin,
                &target_classes,
                priority,
                &candidates,
                &mut effects,
            );
            for effect in &mut effects[first_effect..] {
                effect.config_priority = Some(registered.config_priority);
                effect.mixin_priority = Some(priority);
            }
            for method in &mixin.methods {
                for injector in method
                    .annotations
                    .iter()
                    .filter(|annotation| injector_kind(&annotation.descriptor).is_some())
                {
                    let first_injector_effect = effects.len();
                    analyze_injector(
                        scanned,
                        artifact,
                        mixin,
                        method,
                        injector,
                        &target_classes,
                        priority,
                        registered.default_require,
                        registered.refmap.as_deref(),
                        &candidates,
                        &mut effects,
                        &mut warnings,
                        &mut findings,
                    );
                    let injector_order = injector
                        .value("order")
                        .and_then(AnnotationValue::integer)
                        .and_then(|value| i32::try_from(value).ok())
                        .unwrap_or_else(|| default_injector_order(&injector.descriptor));
                    for effect in &mut effects[first_injector_effect..] {
                        effect.config_priority = Some(registered.config_priority);
                        effect.mixin_priority = Some(priority);
                        effect.injector_order = Some(injector_order);
                    }
                }
            }
            for effect in &mut effects[first_effect..] {
                effect.activation = if plugin_controlled {
                    Activation::Conditional
                } else {
                    Activation::Definite
                };
            }
            if plugin_controlled {
                // Static shape recovery remains useful for conditional
                // interactions, but an unresolved selector is not a runtime
                // warning until the plugin is known to apply the Mixin.
                warnings.truncate(warnings_before_mixin);
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
    let (candidate_risks, candidate_interactions) = candidate_query_findings(&effects, &candidates);
    let unary_risks = candidate_merge_risks(&candidates);
    scanned.coverage.unsupported_selector_syntax += findings.unsupported_selectors;
    scanned.coverage.unsupported_injection_points += findings.unsupported_injection_points;
    scanned.coverage.valid_multi_target_selectors += findings.valid_multi_target_selectors;
    scanned.coverage.optional_unresolved_references += findings.optional_unresolved_references;
    scanned.coverage.unresolved_required_references += warnings
        .iter()
        .filter(|warning| warning.kind == WarningKind::UnresolvedSoftReference)
        .count();
    scanned.warnings.extend(warnings);
    emit(
        progress,
        AuditProgressEvent::StageFinished {
            stage: AuditProgressStage::AnalyzeMixins,
            completed,
        },
    );
    MixinAnalysis {
        effects,
        unary_risks,
        risks: candidate_risks,
        interactions: candidates
            .interactions
            .into_iter()
            .chain(candidate_interactions)
            .collect(),
        coverage_gaps: findings.coverage_gaps,
        inactive_candidates: findings.inactive_candidates,
    }
}

fn candidate_merge_risks(
    candidates: &CandidateUniverse,
) -> Vec<crate::model::UnaryCompatibilityRisk> {
    candidates
        .invalid_contributions
        .iter()
        .map(|invalid| {
            let target = Target::method(
                &invalid.key.target_class,
                &invalid.key.method_name,
                &invalid.key.descriptor,
            );
            let (rule, reason, annotation) = match invalid.kind {
                MethodContributionKind::InvalidOverwriteTarget => (
                    "invalid_overwrite_target",
                    "@Overwrite names a method which does not exist in the candidate target class.",
                    OVERWRITE,
                ),
                MethodContributionKind::MissingRequiredOverwriteAnnotation => (
                    "missing_required_overwrite_annotation",
                    "The active Mixin config requires @Overwrite on a method which replaces an existing target method.",
                    "overwrite.requireAnnotations",
                ),
                _ => unreachable!("only invalid merge contributions are recorded"),
            };
            let severity = crate::model::Severity::High;
            let confidence = Confidence::Exact;
            let activation = Activation::Definite;
            let mut evidence =
                Evidence::new(&invalid.key.artifact_id, &invalid.key.mixin_class, reason);
            evidence.method = Some(format!(
                "{}{}",
                invalid.key.method_name, invalid.key.descriptor
            ));
            evidence.annotation = Some(annotation.to_string());
            crate::model::UnaryCompatibilityRisk {
                artifact_id: invalid.key.artifact_id.clone(),
                environment_target: candidates
                    .classes
                    .get(&invalid.key.target_class)
                    .and_then(|class| class.base_artifact.clone())
                    .unwrap_or_else(|| "active class universe".to_string()),
                target,
                rule: rule.to_string(),
                reason: reason.to_string(),
                mutations: vec![MutationKind::ReplaceMethodBody],
                evidence: vec![evidence],
                severity,
                confidence,
                precision: Precision::Method,
                risk_index: effective_risk_index(severity, confidence, activation),
                activation,
            }
        })
        .collect()
}

fn analyze_mixin_structure(
    artifact: &ParsedArtifact,
    mixin: &ParsedClass,
    targets: &[String],
    priority: i32,
    candidates: &CandidateUniverse,
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
            } else {
                let contribution =
                    candidates.contribution_kind(artifact, mixin, target_class, method);
                let winning =
                    candidates.is_winning_contribution(artifact, mixin, target_class, method);
                match contribution {
                    Some(
                        MethodContributionKind::OverwriteExistingMethod
                        | MethodContributionKind::ReplaceExistingMethod,
                    ) if winning => effects.push(structural_effect(
                        artifact,
                        mixin,
                        target,
                        MutationKind::ReplaceMethodBody,
                        if annotation(&method.annotations, OVERWRITE).is_some() {
                            OVERWRITE
                        } else {
                            "Mixin method replacement"
                        },
                        priority,
                    )),
                    Some(MethodContributionKind::AddNewMethod) if winning => {
                        effects.push(structural_effect(
                            artifact,
                            mixin,
                            target,
                            MutationKind::AddMethod,
                            "Mixin method addition",
                            priority,
                        ));
                    }
                    None if !((method.is_synthetic
                        || annotation(&method.annotations, UNIQUE).is_some()
                        || mixin_is_unique)
                        && !method.is_public) =>
                    {
                        // Test-only and degraded callers without a prepared
                        // candidate state retain the conservative shape.
                        effects.push(structural_effect(
                            artifact,
                            mixin,
                            target,
                            MutationKind::AddMethod,
                            "Mixin method merge",
                            priority,
                        ));
                    }
                    _ => {}
                }
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
                config_priority: None,
                mixin_priority: Some(priority),
                injector_order: None,
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
    default_require: u32,
    active_refmap: Option<&str>,
    candidates: &CandidateUniverse,
    effects: &mut Vec<Effect>,
    warnings: &mut Vec<Warning>,
    findings: &mut MixinFindings,
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
    let require = injector_requirement(injector, default_require);
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
            if !parse_selector(raw_selector).supported {
                findings.unsupported_selectors += 1;
                findings.coverage_gaps.push(CoverageGap {
                    artifact_id: Some(artifact.id.clone()),
                    scope: format!("{}::{raw_selector}", mixin.name),
                    kind: CoverageGapKind::UnsupportedSelector,
                    detail: "method selector syntax is not supported by the static evaluator"
                        .to_string(),
                    count: 1,
                });
                continue;
            }
            let method_resolution = resolve_method_reference(
                scanned,
                artifact,
                &mixin.name,
                target_class,
                raw_selector,
                active_refmap,
                candidates,
            );
            if method_resolution.resolution == SoftReferenceResolution::MultiTargetValid {
                findings.valid_multi_target_selectors += 1;
            }
            if method_resolution.methods.is_empty() {
                if require.is_none_or(|minimum| minimum == 0) {
                    findings.optional_unresolved_references += 1;
                    findings.inactive_candidates.push(InactiveCandidate {
                        artifact_id: artifact.id.clone(),
                        class: Some(mixin.name.clone()),
                        config_path: None,
                        kind: InactiveCandidateKind::MissingOptionalTarget,
                        reason: format!(
                            "optional injector selector '{raw_selector}' has no active target"
                        ),
                    });
                    continue;
                }
                warn_for_soft_reference(
                    warnings,
                    artifact,
                    &mixin.name,
                    "method selector",
                    raw_selector,
                    method_resolution.resolution,
                );
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
                            method_selector: selector_model(raw_selector),
                            selector_kind: "CONSTANT".to_string(),
                            target_selectors: Vec::new(),
                            method: method_target.clone(),
                            candidates: selected.clone(),
                            selected: selected.clone(),
                            minimum_matches: require,
                            maximum_matches: allow,
                            expected_matches: expect,
                            ordinal: None,
                            shift: None,
                            slice: None,
                            slice_start: None,
                            slice_end: None,
                            resolution: method_resolution.resolution,
                            local_selector: None,
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
                            config_priority: None,
                            mixin_priority: Some(priority),
                            injector_order: None,
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
                            config_priority: None,
                            mixin_priority: Some(priority),
                            injector_order: None,
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
                // Method selectors and @At target selectors are distinct
                // namespaces. Carrying the former into the instruction query
                // can turn a valid direct member match into a fabricated
                // alternative target.
                let mut target_candidates = Vec::new();
                let mut slices_used = BTreeSet::new();
                let mut slice_ranges = BTreeSet::new();
                let mut ordinals = BTreeSet::new();
                let mut shifts = BTreeSet::new();
                let mut resolution = method_resolution.resolution;
                let mut degradation = None;
                let reference_context = ReferenceContext {
                    artifact,
                    mixin_class: &mixin.name,
                    target_class,
                    active_refmap,
                };

                for at in &ats {
                    let at_kind = at
                        .value("value")
                        .and_then(|value| value.strings().into_iter().next())
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    let support = injection_point_support(&at_kind);
                    if support != InjectionPointSupport::Supported {
                        let label = if support == InjectionPointSupport::KnownUnsupported {
                            "known but unsupported"
                        } else {
                            "custom"
                        };
                        findings.unsupported_injection_points += 1;
                        findings.coverage_gaps.push(CoverageGap {
                            artifact_id: Some(artifact.id.clone()),
                            scope: format!("{}::@At({at_kind})", mixin.name),
                            kind: CoverageGapKind::UnsupportedInjectionPoint,
                            detail: format!(
                                "{label} InjectionPoint kept {mutation_kind:?} semantics at method precision"
                            ),
                            count: 1,
                        });
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
                        reference_context,
                    );
                    if let Some(reason) = &active_slice.unresolved {
                        let gap_kind = if target_method.instructions.is_empty() {
                            CoverageGapKind::UnavailableMethodBody
                        } else {
                            CoverageGapKind::UnresolvedSlice
                        };
                        let consequence = match gap_kind {
                            CoverageGapKind::UnavailableMethodBody => {
                                "the injector remained at method precision because no instruction body was available"
                            }
                            _ => {
                                "the injector remained at method precision instead of searching outside its declared slice"
                            }
                        };
                        findings.coverage_gaps.push(CoverageGap {
                            artifact_id: Some(artifact.id.clone()),
                            scope: format!(
                                "{}::{}{}",
                                mixin.name, target_method.name, target_method.descriptor
                            ),
                            kind: gap_kind,
                            detail: format!("{reason}; {consequence}"),
                            count: 1,
                        });
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
                        at,
                        &at_kind,
                        active_slice.range,
                        reference_context,
                    );
                    if let Some(raw_at_target) = at
                        .value("target")
                        .and_then(|value| value.strings().into_iter().next())
                    {
                        if require.is_some_and(|minimum| minimum > 0) {
                            warn_for_soft_reference(
                                warnings,
                                artifact,
                                &mixin.name,
                                &format!("@At({at_kind}) target"),
                                &raw_at_target,
                                at_resolution,
                            );
                        } else if matches!(
                            at_resolution,
                            SoftReferenceResolution::Ambiguous
                                | SoftReferenceResolution::Unresolved
                        ) {
                            findings.optional_unresolved_references += 1;
                        }
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
                let local_selector = (mutation_kind == MutationKind::ModifyLocalValue)
                    .then(|| resolve_local_selector(injector, handler, target_method));
                let mut selected_references = selected
                    .iter()
                    .map(|instruction| instruction.reference.clone())
                    .collect::<Vec<_>>();
                let mut effect_precision = Precision::Instruction;
                if let Some(local_selector) = &local_selector {
                    if local_selector.args_only
                        && at_kinds.iter().any(|kind| kind == "HEAD")
                        && let Some(slot) = local_selector.slot
                    {
                        let local = local_instruction_reference(
                            target_method,
                            &method_target,
                            slot,
                            local_selector.expected_type.as_deref(),
                        );
                        candidates = vec![local.clone()];
                        selected_references = vec![local];
                    } else if local_selector.slot.is_none() {
                        findings.coverage_gaps.push(CoverageGap {
                            artifact_id: Some(artifact.id.clone()),
                            scope: format!("{}::{}{}", mixin.name, handler.name, handler.descriptor),
                            kind: CoverageGapKind::UnresolvedLocalSelector,
                            detail: "ModifyVariable local discriminator did not resolve to one unique slot"
                                .to_string(),
                            count: 1,
                        });
                        effect_precision = Precision::Method;
                    }
                }
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
                if mutation_kind == MutationKind::ModifyLocalValue && mutations.is_empty() {
                    mutations.push(Mutation::new(
                        mutation_kind,
                        method_target.clone(),
                        Precision::Method,
                    ));
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
                    method_selector: selector_model(raw_selector),
                    selector_kind: selector_kind.clone(),
                    target_selectors: target_candidates.clone(),
                    method: method_target.clone(),
                    candidates,
                    selected: selected_references.clone(),
                    minimum_matches: require,
                    maximum_matches: allow,
                    expected_matches: expect,
                    ordinal: (ordinals.len() == 1)
                        .then(|| ordinals.iter().next().copied())
                        .flatten(),
                    shift: (shifts.len() == 1)
                        .then(|| shifts.iter().next().cloned())
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
                    local_selector,
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
                            effect_precision,
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
                    precision: effect_precision,
                    confidence: confidence_for_resolution(resolution),
                    activation: Activation::Candidate,
                    config_priority: None,
                    mixin_priority: Some(priority),
                    injector_order: None,
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
        config_priority: None,
        mixin_priority: Some(priority),
        injector_order: None,
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
        config_priority: None,
        mixin_priority: Some(priority),
        injector_order: None,
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
        config_priority: None,
        mixin_priority: Some(priority),
        injector_order: None,
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

fn default_injector_order(descriptor: &str) -> i32 {
    match injector_simple_name(descriptor) {
        "Redirect" | "ModifyConstant" => 10_000,
        _ => 1_000,
    }
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
                    && matches!(
                        &instruction.kind,
                        InstructionKind::Type(class)
                            if target_candidates.is_empty()
                                || target_candidates.iter().any(|target| {
                                    new_target_class(target).as_deref() == Some(class)
                                })
                    )
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

fn new_target_class(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some((_, return_type)) = value.split_once(')')
        && let Some(class) = return_type
            .strip_prefix('L')
            .and_then(|class| class.strip_suffix(';'))
    {
        // Mixin's BeforeNew also accepts a constructor descriptor whose
        // object return type identifies the allocated class.
        return normalize_class_name(class);
    }
    if let Some(rest) = value.strip_prefix('L')
        && let Some((owner, _)) = rest.split_once(';')
    {
        return normalize_class_name(owner);
    }
    if value.contains("<init>") {
        return parse_selector(value).owner;
    }
    normalize_class_name(value)
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

#[derive(Clone, Copy)]
struct ReferenceContext<'a> {
    artifact: &'a ParsedArtifact,
    mixin_class: &'a str,
    target_class: &'a str,
    active_refmap: Option<&'a str>,
}

fn resolve_active_slice(
    method: &ParsedMethod,
    method_target: &Target,
    slices: &[&ParsedAnnotation],
    at: &ParsedAnnotation,
    context: ReferenceContext<'_>,
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
            let (target_candidates, _, boundary_resolution) =
                resolve_at_reference(method, boundary_at, &kind, None, context);
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
    candidates: &'a CandidateUniverse,
    target_class: &str,
    selector: &str,
) -> Vec<(String, &'a ParsedMethod)> {
    let selector = parse_selector(selector);
    if !selector.supported {
        return Vec::new();
    }
    let owner = selector.owner.as_deref().unwrap_or(target_class);
    candidates
        .methods(scanned, owner)
        .into_iter()
        .filter(|(_, method)| {
            selector.matches_name(&method.name)
                && selector
                    .descriptor
                    .as_ref()
                    .is_none_or(|descriptor| descriptor == &method.descriptor)
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
    active_refmap: Option<&str>,
    candidate_universe: &'a CandidateUniverse,
) -> ResolvedMethods<'a> {
    let candidates =
        refmap_candidates(artifact, mixin_class, original, target_class, active_refmap);
    let sources = refmap_sources(artifact, mixin_class, original, target_class, active_refmap);
    let direct_methods =
        resolve_target_methods(scanned, candidate_universe, target_class, original);
    if !direct_methods.is_empty() {
        let resolution = if direct_methods.len() == 1 {
            SoftReferenceResolution::DirectExact
        } else {
            SoftReferenceResolution::MultiTargetValid
        };
        return ResolvedMethods {
            candidates,
            sources,
            resolution,
            methods: deduplicate_methods(direct_methods),
        };
    }
    let mut methods = Vec::new();
    let mut active_candidates = BTreeSet::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.as_str() != original)
    {
        let resolved = resolve_target_methods(scanned, candidate_universe, target_class, candidate);
        if !resolved.is_empty() {
            active_candidates.insert(normalize_soft_reference(candidate, target_class));
        }
        for (artifact_id, method) in resolved {
            methods.push((artifact_id, method));
        }
    }
    methods = deduplicate_methods(methods);
    let distinct_methods = methods
        .iter()
        .map(|(artifact_id, method)| format!("{artifact_id}|{}{}", method.name, method.descriptor))
        .collect::<BTreeSet<_>>();
    let resolution = match distinct_methods.len() {
        0 => SoftReferenceResolution::Unresolved,
        1 => SoftReferenceResolution::RefmapExact,
        _ if active_candidates.len() == 1 => SoftReferenceResolution::MultiTargetValid,
        _ => SoftReferenceResolution::Ambiguous,
    };
    ResolvedMethods {
        candidates,
        sources,
        resolution,
        methods,
    }
}

fn deduplicate_methods(mut methods: Vec<(String, &ParsedMethod)>) -> Vec<(String, &ParsedMethod)> {
    methods.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.name.cmp(&right.1.name))
            .then_with(|| left.1.descriptor.cmp(&right.1.descriptor))
    });
    methods.dedup_by(|left, right| {
        left.0 == right.0 && left.1.name == right.1.name && left.1.descriptor == right.1.descriptor
    });
    methods
}

fn resolve_at_reference(
    method: &ParsedMethod,
    at: &ParsedAnnotation,
    kind: &str,
    range: Option<(usize, usize)>,
    context: ReferenceContext<'_>,
) -> (Vec<String>, Vec<String>, SoftReferenceResolution) {
    let originals = at
        .value("target")
        .into_iter()
        .chain(at.value("desc"))
        .flat_map(selector_values)
        .collect::<BTreeSet<_>>();
    let (candidates, sources) = at_selector_candidates(
        context.artifact,
        context.mixin_class,
        at,
        context.target_class,
        context.active_refmap,
    );
    if originals.is_empty() {
        return (candidates, sources, SoftReferenceResolution::NotApplicable);
    }
    let direct_active = originals.iter().any(|original| {
        !match_instructions(method, at, kind, std::slice::from_ref(original), range).is_empty()
    });
    if direct_active {
        return (
            candidates,
            sources,
            if originals.len() == 1 {
                SoftReferenceResolution::DirectExact
            } else {
                SoftReferenceResolution::MultiTargetValid
            },
        );
    }
    let mut active = BTreeSet::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| !originals.contains(candidate.as_str()))
    {
        if !match_instructions(method, at, kind, std::slice::from_ref(candidate), range).is_empty()
        {
            active.insert(normalize_soft_reference(candidate, context.target_class));
        }
    }
    let resolution = match active.len() {
        0 => SoftReferenceResolution::Unresolved,
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
        | SoftReferenceResolution::MultiTargetValid
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
            SoftReferenceResolution::RefmapExact | SoftReferenceResolution::MultiTargetValid => 2,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorNamePattern {
    Exact,
    Prefix,
    Suffix,
    All,
}

#[derive(Debug)]
struct Selector {
    owner: Option<String>,
    name: String,
    descriptor: Option<String>,
    pattern: SelectorNamePattern,
    supported: bool,
}

impl Selector {
    fn matches_name(&self, candidate: &str) -> bool {
        match self.pattern {
            SelectorNamePattern::Exact => candidate == self.name,
            SelectorNamePattern::Prefix => candidate.starts_with(&self.name),
            SelectorNamePattern::Suffix => candidate.ends_with(&self.name),
            SelectorNamePattern::All => true,
        }
    }
}

fn parse_selector(value: &str) -> Selector {
    let value = value.trim();
    if value.starts_with('/')
        || value.starts_with('@')
        || value.contains(" desc=")
        || value.contains(" regex=")
    {
        return Selector {
            owner: None,
            name: value.to_string(),
            descriptor: None,
            pattern: SelectorNamePattern::Exact,
            supported: false,
        };
    }
    let (owner, member) = if let Some(rest) = value.strip_prefix('L') {
        rest.split_once(';')
            .map_or((None, value), |(owner, member)| {
                (Some(owner.to_string()), member)
            })
    } else {
        let descriptor_start = value.find('(').unwrap_or(value.len());
        let prefix = &value[..descriptor_start];
        // Mixin accepts owner/member spellings such as
        // `java/util/List.add(...)` as well as `java/util/List/add(...)`.
        // Pick the right-most separator across both forms; preferring any
        // slash would incorrectly parse the former as owner `java/util` and
        // method `List.add`.
        let owner_separator = prefix.rfind('/').into_iter().chain(prefix.rfind('.')).max();
        owner_separator.map_or((None, value), |position| {
            (
                Some(value[..position].replace('.', "/")),
                &value[position + 1..],
            )
        })
    };
    let (name, descriptor) = if let Some(position) = member.find('(') {
        (&member[..position], Some(member[position..].to_string()))
    } else if let Some((name, descriptor)) = member.split_once(':') {
        (name, Some(descriptor.to_string()))
    } else {
        (member, None)
    };
    let name = name.trim_start_matches(['.', ':']);
    let star_count = name.chars().filter(|character| *character == '*').count();
    let (name, pattern, supported) = if name == "*" {
        (String::new(), SelectorNamePattern::All, true)
    } else if star_count == 0 {
        (
            name.to_string(),
            SelectorNamePattern::Exact,
            !name.chars().any(|character| {
                matches!(character, '[' | ']' | '{' | '}' | '+' | '?' | '|' | '\\')
            }),
        )
    } else if star_count == 1 && name.ends_with('*') {
        (
            name.trim_end_matches('*').to_string(),
            SelectorNamePattern::Prefix,
            true,
        )
    } else if star_count == 1 && name.starts_with('*') {
        (
            name.trim_start_matches('*').to_string(),
            SelectorNamePattern::Suffix,
            true,
        )
    } else {
        (name.to_string(), SelectorNamePattern::Exact, false)
    };
    Selector {
        owner: owner.map(|owner| owner.replace('.', "/")),
        name,
        descriptor,
        pattern,
        supported,
    }
}

fn selector_model(raw: &str) -> crate::model::MethodSelector {
    use crate::model::{GlobPattern, MethodSelector};

    let value = raw.trim();
    if value.starts_with('@') {
        return MethodSelector::Dynamic {
            raw: value.to_string(),
        };
    }
    let selector = parse_selector(value);
    if !selector.supported {
        return MethodSelector::Unsupported {
            raw: value.to_string(),
        };
    }
    match selector.pattern {
        SelectorNamePattern::Exact => MethodSelector::Exact {
            owner: selector.owner,
            name: selector.name,
            descriptor: selector.descriptor,
        },
        SelectorNamePattern::Prefix => MethodSelector::Glob {
            owner: selector.owner,
            pattern: GlobPattern::Prefix,
            value: selector.name,
            descriptor: selector.descriptor,
        },
        SelectorNamePattern::Suffix => MethodSelector::Glob {
            owner: selector.owner,
            pattern: GlobPattern::Suffix,
            value: selector.name,
            descriptor: selector.descriptor,
        },
        SelectorNamePattern::All => MethodSelector::All {
            descriptor: selector.descriptor,
        },
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
    active_refmap: Option<&str>,
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
            active_refmap,
        ));
        sources.extend(refmap_sources(
            artifact,
            mixin_class,
            &original,
            default_owner,
            active_refmap,
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
    selector.supported
        && selector
            .owner
            .as_ref()
            .is_none_or(|owner| owner == &member.owner)
        && selector.matches_name(&member.name)
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
    active_refmap: Option<&str>,
) -> Vec<String> {
    let normalized_mixin = mixin_class.replace('.', "/");
    let mut candidates = artifact
        .refmaps
        .iter()
        .filter(|entry| {
            active_refmap.is_some_and(|path| entry.path.eq_ignore_ascii_case(path))
                && entry.mixin_class == normalized_mixin
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
    active_refmap: Option<&str>,
) -> Vec<String> {
    let normalized_mixin = mixin_class.replace('.', "/");
    artifact
        .refmaps
        .iter()
        .filter(|entry| {
            active_refmap.is_some_and(|path| entry.path.eq_ignore_ascii_case(path))
                && entry.mixin_class == normalized_mixin
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
    crate::mixin_config::normalize_class_name(value)
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

fn injector_requirement(injector: &ParsedAnnotation, default_require: u32) -> Option<u32> {
    match injector.value("require").and_then(AnnotationValue::integer) {
        Some(value) if value >= 0 => u32::try_from(value).ok(),
        _ => (default_require > 0).then_some(default_require),
    }
}

fn resolve_local_selector(
    injector: &ParsedAnnotation,
    handler: &ParsedMethod,
    target: &ParsedMethod,
) -> LocalSelector {
    let args_only = injector
        .value("argsOnly")
        .and_then(AnnotationValue::boolean)
        .unwrap_or(false);
    let explicit_index = injector
        .value("index")
        .and_then(AnnotationValue::integer)
        .filter(|value| *value >= 0)
        .and_then(|value| u16::try_from(value).ok());
    let ordinal = injector
        .value("ordinal")
        .and_then(AnnotationValue::integer)
        .filter(|value| *value >= 0)
        .and_then(|value| u32::try_from(value).ok());
    let names = injector
        .value("name")
        .map(AnnotationValue::strings)
        .unwrap_or_default();
    let expected_type = method_return_descriptor(&handler.descriptor);
    let arguments = method_argument_slots(&target.descriptor, target.is_static);
    let slot = explicit_index.or_else(|| {
        if !args_only || !names.is_empty() {
            return None;
        }
        let matching = arguments
            .iter()
            .filter(|(_, descriptor)| {
                expected_type
                    .as_ref()
                    .is_none_or(|expected| expected == descriptor)
            })
            .map(|(slot, _)| *slot)
            .collect::<Vec<_>>();
        ordinal.map_or_else(
            || (matching.len() == 1).then(|| matching[0]),
            |ordinal| {
                matching
                    .get(usize::try_from(ordinal).unwrap_or(usize::MAX))
                    .copied()
            },
        )
    });
    let argument_slots = arguments
        .iter()
        .map(|(slot, _)| *slot)
        .collect::<BTreeSet<_>>();
    LocalSelector {
        args_only,
        explicit_index,
        ordinal,
        names,
        expected_type,
        slot,
        frame_position: slot.map(|slot| {
            if argument_slots.contains(&slot) {
                FramePosition::Argument
            } else {
                FramePosition::Local
            }
        }),
    }
}

fn method_argument_slots(descriptor: &str, is_static: bool) -> Vec<(u16, String)> {
    let Some(arguments) = descriptor
        .strip_prefix('(')
        .and_then(|descriptor| descriptor.split_once(')').map(|(arguments, _)| arguments))
    else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut offset = 0_usize;
    let mut slot = u16::from(!is_static);
    while offset < arguments.len() {
        let Some((descriptor, consumed)) = next_type_descriptor(&arguments[offset..]) else {
            break;
        };
        result.push((slot, descriptor.clone()));
        slot = slot.saturating_add(if matches!(descriptor.as_str(), "J" | "D") {
            2
        } else {
            1
        });
        offset += consumed;
    }
    result
}

fn method_return_descriptor(descriptor: &str) -> Option<String> {
    descriptor
        .split_once(')')
        .and_then(|(_, result)| next_type_descriptor(result))
        .map(|(descriptor, _)| descriptor)
}

fn next_type_descriptor(value: &str) -> Option<(String, usize)> {
    let bytes = value.as_bytes();
    let first = *bytes.first()?;
    if first == b'[' {
        let mut end = 1;
        while bytes.get(end) == Some(&b'[') {
            end += 1;
        }
        if bytes.get(end) == Some(&b'L') {
            end += value[end..].find(';')? + 1;
        } else {
            end += 1;
        }
        return Some((value[..end].to_string(), end));
    }
    if first == b'L' {
        let end = value.find(';')? + 1;
        return Some((value[..end].to_string(), end));
    }
    matches!(
        first,
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b'V'
    )
    .then(|| (value[..1].to_string(), 1))
}

fn local_instruction_reference(
    target_method: &ParsedMethod,
    target: &Target,
    slot: u16,
    descriptor: Option<&str>,
) -> crate::model::InstructionReference {
    let instruction_index = u32::MAX.saturating_sub(u32::from(slot));
    let identity = target_method
        .instructions
        .first()
        .and_then(|instruction| instruction.reference.identity.as_ref())
        .map(|identity| crate::model::InstructionIdentity {
            definition: identity.definition.clone(),
            method_name: target.member.as_ref().map_or_else(
                || identity.method_name.clone(),
                |member| member.name.clone(),
            ),
            method_descriptor: target.member.as_ref().map_or_else(
                || identity.method_descriptor.clone(),
                |member| member.descriptor.clone(),
            ),
            instruction_index,
        });
    crate::model::InstructionReference {
        identity,
        stable_id: instruction_index,
        original_offset: None,
        opcode: local_load_opcode(descriptor),
        local_slot: Some(slot),
        member: None,
        constant: Some(format!("local:{slot}")),
    }
}

fn local_load_opcode(descriptor: Option<&str>) -> u8 {
    match descriptor
        .map(str::as_bytes)
        .and_then(|bytes| bytes.first())
        .copied()
    {
        Some(b'J') => 22,
        Some(b'F') => 23,
        Some(b'D') => 24,
        Some(b'L' | b'[') => 25,
        _ => 21,
    }
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
        let analysis = analyze_detailed(&mut scanned);
        let injector = analysis
            .effects
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
        let analysis = analyze_detailed(&mut scanned);
        let injector = analysis
            .effects
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
        assert!(analysis.coverage_gaps.iter().any(|gap| {
            gap.kind == CoverageGapKind::UnsupportedInjectionPoint
                && gap.scope.contains("EXAMPLE:CUSTOM")
        }));
    }

    #[test]
    fn only_the_registered_config_refmap_contributes_candidates() {
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
            resources: Vec::new(),
        };
        assert_eq!(
            refmap_candidates(&artifact, "example/Mixin", "tick()V", "game/Target", None),
            vec!["tick()V"]
        );
        let candidates = refmap_candidates(
            &artifact,
            "example/Mixin",
            "tick()V",
            "game/Target",
            Some("mod.refmap.json"),
        );
        assert_eq!(candidates, vec!["a()V", "method_1()V", "tick()V"]);
        let sources = refmap_sources(
            &artifact,
            "example/Mixin",
            "tick()V",
            "game/Target",
            Some("mod.refmap.json"),
        );
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
        fixture_injector_mut(&mut scanned)
            .values
            .insert("require".to_string(), AnnotationValue::Integer(1));

        let analysis = analyze_detailed(&mut scanned);
        let effect = analysis
            .effects
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
                identity: None,
                stable_id: 0,
                original_offset: Some(0),
                opcode: 182,
                local_slot: None,
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

        let analysis = analyze_detailed(&mut scanned);
        let effect = analysis
            .effects
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
    fn external_owner_is_validated_on_the_actual_instruction_without_classpath_presence() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods[0].instructions =
            vec![call_instruction(0, "java/util/List", "add")];
        fixture_at_mut(&mut scanned).values.insert(
            "target".to_string(),
            AnnotationValue::String("java/util/List.add()V".to_string()),
        );
        fixture_injector_mut(&mut scanned)
            .values
            .insert("require".to_string(), AnnotationValue::Integer(1));

        let analysis = analyze_detailed(&mut scanned);
        let query = analysis
            .effects
            .iter()
            .find_map(|effect| effect.queries.first())
            .unwrap();

        assert_eq!(query.resolution, SoftReferenceResolution::DirectExact);
        assert_eq!(query.target_selectors, vec!["java/util/List.add()V"]);
        assert_eq!(query.selected.len(), 1);
        assert!(
            scanned
                .warnings
                .iter()
                .all(|warning| warning.kind != WarningKind::UnresolvedSoftReference)
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
    fn selector_ast_supports_all_prefix_suffix_constructor_and_owner_forms() {
        let all = parse_selector("*");
        assert!(all.supported && all.matches_name("anything"));
        let prefix = parse_selector("disconnect*");
        assert!(prefix.matches_name("disconnectNow"));
        assert!(!prefix.matches_name("connect"));
        let suffix = parse_selector("*Async");
        assert!(suffix.matches_name("loadAsync"));
        let clinit = parse_selector("<clinit>*");
        assert!(clinit.matches_name("<clinit>"));
        let constructor = parse_selector("<init>(I)V");
        assert_eq!(constructor.name, "<init>");
        assert_eq!(constructor.descriptor.as_deref(), Some("(I)V"));
        for raw in [
            "game.Target.tick()V",
            "game/Target/tick()V",
            "game/Target.tick()V",
            "Lgame/Target;tick()V",
        ] {
            let selector = parse_selector(raw);
            assert_eq!(selector.owner.as_deref(), Some("game/Target"), "{raw}");
            assert_eq!(selector.name, "tick", "{raw}");
            assert_eq!(selector.descriptor.as_deref(), Some("()V"), "{raw}");
        }
    }

    #[test]
    fn name_only_selector_matches_every_overload_without_ambiguity_warning() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        let mut overload = scanned.artifacts[0].classes[0].methods[0].clone();
        overload.descriptor = "(I)V".to_string();
        scanned.artifacts[0].classes[0].methods.push(overload);
        fixture_injector_mut(&mut scanned).values.insert(
            "method".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String("tick".to_string())]),
        );

        let analysis = analyze_detailed(&mut scanned);

        assert_eq!(
            analysis
                .effects
                .iter()
                .filter(|effect| !effect.queries.is_empty())
                .count(),
            2
        );
        assert_eq!(scanned.coverage.valid_multi_target_selectors, 1);
        assert!(
            scanned
                .warnings
                .iter()
                .all(|warning| { warning.kind != WarningKind::AmbiguousSoftReference })
        );
    }

    #[test]
    fn wildcard_injector_does_not_target_accessor_generated_after_prepare() {
        let mut scanned = mixin_fixture("RETURN", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        fixture_injector_mut(&mut scanned).values.insert(
            "method".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String("*".to_string())]),
        );
        scanned.artifacts[1].classes[0].methods.push(ParsedMethod {
            name: "generatedAccessor".to_string(),
            descriptor: "()I".to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: vec![ParsedAnnotation {
                descriptor: ACCESSOR.to_string(),
                values: BTreeMap::new(),
            }],
            max_locals: Some(1),
            instructions: Vec::new(),
        });

        let analysis = analyze_detailed(&mut scanned);
        let queried_methods = analysis
            .effects
            .iter()
            .flat_map(|effect| &effect.queries)
            .map(|query| {
                query
                    .method
                    .member
                    .as_ref()
                    .map(|member| member.name.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();

        assert_eq!(queried_methods, vec!["tick"]);
        assert!(
            scanned
                .warnings
                .iter()
                .all(|warning| !warning.message.contains("contains no instructions"))
        );
    }

    #[test]
    fn regex_or_dynamic_selector_is_a_coverage_gap_not_a_warning() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        fixture_injector_mut(&mut scanned).values.insert(
            "method".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String(
                "/^tick/ desc=/Target;$".to_string(),
            )]),
        );

        let analysis = analyze_detailed(&mut scanned);

        assert!(
            analysis
                .coverage_gaps
                .iter()
                .any(|gap| { gap.kind == CoverageGapKind::UnsupportedSelector })
        );
        assert!(
            scanned
                .warnings
                .iter()
                .all(|warning| { warning.kind != WarningKind::UnresolvedSoftReference })
        );
    }

    #[test]
    fn wrap_method_resolves_as_a_composable_method_effect_without_at() {
        let mut scanned = mixin_fixture(
            "HEAD",
            "Lcom/llamalad7/mixinextras/injector/wrapmethod/WrapMethod;",
        );
        let handler = &mut scanned.artifacts[1].classes[0].methods[0];
        handler.annotations[0].values.remove("at");

        let analysis = analyze_detailed(&mut scanned);
        let effect = analysis
            .effects
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
                identity: None,
                stable_id: 0,
                original_offset: Some(0),
                opcode: 8,
                local_slot: None,
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

        let analysis = analyze_detailed(&mut scanned);
        let effect = analysis
            .effects
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
        assert!(analysis.coverage_gaps.iter().any(|gap| {
            gap.kind == CoverageGapKind::UnsupportedInjectionPoint
                && gap.scope.contains("MIXINEXTRAS:EXPRESSION")
        }));
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
        let plain = at_annotation("NEW", Some("game/Thing"), Vec::new(), None, None);
        assert_eq!(
            match_instructions(&method, &plain, "NEW", &["game/Thing".to_string()], None)
                .iter()
                .map(|instruction| instruction.reference.stable_id)
                .collect::<Vec<_>>(),
            vec![0]
        );
        let constructor_descriptor =
            at_annotation("NEW", Some("(I)Lgame/Thing;"), Vec::new(), None, None);
        assert_eq!(
            match_instructions(
                &method,
                &constructor_descriptor,
                "NEW",
                &["(I)Lgame/Thing;".to_string()],
                None
            )
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
        fixture_injector_mut(&mut scanned)
            .values
            .insert("require".to_string(), AnnotationValue::Integer(1));

        let analysis = analyze_detailed(&mut scanned);
        let injector = analysis
            .effects
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
        assert!(
            analysis
                .coverage_gaps
                .iter()
                .any(|gap| gap.kind == CoverageGapKind::UnresolvedSlice)
        );
    }

    #[test]
    fn unavailable_method_body_is_not_mislabeled_as_an_unresolved_slice() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods[0]
            .instructions
            .clear();

        let analysis = analyze_detailed(&mut scanned);

        assert!(
            analysis
                .coverage_gaps
                .iter()
                .any(|gap| gap.kind == CoverageGapKind::UnavailableMethodBody)
        );
        assert!(
            analysis
                .coverage_gaps
                .iter()
                .all(|gap| gap.kind != CoverageGapKind::UnresolvedSlice)
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
        fixture_injector_mut(&mut scanned)
            .values
            .insert("require".to_string(), AnnotationValue::Integer(1));

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
            definition_id: None,
            future_version_best_effort: false,
            name: "example/Mixin".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
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
            resources: Vec::new(),
        };
        let mut effects = Vec::new();
        let candidates = CandidateUniverse::default();

        analyze_mixin_structure(
            &artifact,
            &mixin,
            &["game/Target".to_string()],
            1000,
            &candidates,
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

    #[test]
    fn synthetic_helper_is_unique_even_when_config_requires_overwrite_annotations() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[1].classes[0].methods = vec![ParsedMethod {
            name: "tick".to_string(),
            descriptor: "()V".to_string(),
            is_static: false,
            is_public: false,
            is_synthetic: true,
            annotations: Vec::new(),
            max_locals: Some(1),
            instructions: vec![return_instruction(0)],
        }];
        let mut registry = MixinRegistry::all_annotated(&scanned);
        registry.configs.push(crate::model::RegisteredMixinConfig {
            artifact_id: "mod".to_string(),
            config_path: "<test>".to_string(),
            side: crate::model::SideConstraint::Common,
            registration: crate::model::RegistrationSource::FabricMetadata,
            activation: crate::model::ConfigActivation::Active,
            required_mods: Vec::new(),
            behavior_version: None,
            parsed: Some(crate::model::ParsedMixinConfig {
                required: false,
                min_version: None,
                compatibility_level: None,
                package: None,
                plugin: None,
                refmap: None,
                priority: 1000,
                mixin_priority: 1000,
                mixins: vec!["example.Mixin".to_string()],
                client: Vec::new(),
                server: Vec::new(),
                default_require: 0,
                default_group: "default".to_string(),
                overwrite_require_annotations: true,
            }),
        });

        let candidates = CandidateUniverse::build(&scanned, &registry);

        assert!(candidates.invalid_contributions.is_empty());
        assert!(
            candidates
                .contribution_kinds
                .values()
                .any(|kind| { *kind == MethodContributionKind::UniqueRenamedMethod })
        );
    }

    #[test]
    fn inherited_interface_method_does_not_require_overwrite_annotation() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods.clear();
        scanned.artifacts[0].classes[0]
            .interfaces
            .push("game/Contract".to_string());
        scanned.artifacts[0].classes.push(ParsedClass {
            definition_id: None,
            future_version_best_effort: false,
            name: "game/Contract".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            annotations: Vec::new(),
            fields: Vec::new(),
            methods: vec![ParsedMethod {
                name: "tick".to_string(),
                descriptor: "()V".to_string(),
                is_static: false,
                is_public: true,
                is_synthetic: false,
                annotations: Vec::new(),
                max_locals: None,
                instructions: Vec::new(),
            }],
        });
        scanned.artifacts[1].classes[0].methods = vec![ParsedMethod {
            name: "tick".to_string(),
            descriptor: "()V".to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: Vec::new(),
            max_locals: Some(1),
            instructions: vec![return_instruction(0)],
        }];
        crate::jar::rebuild_universe(&mut scanned);

        let mut registry = MixinRegistry::all_annotated(&scanned);
        registry.configs.push(crate::model::RegisteredMixinConfig {
            artifact_id: "mod".to_string(),
            config_path: "<test>".to_string(),
            side: crate::model::SideConstraint::Common,
            registration: crate::model::RegistrationSource::FabricMetadata,
            activation: crate::model::ConfigActivation::Active,
            required_mods: Vec::new(),
            behavior_version: None,
            parsed: Some(crate::model::ParsedMixinConfig {
                required: false,
                min_version: None,
                compatibility_level: None,
                package: None,
                plugin: None,
                refmap: None,
                priority: 1000,
                mixin_priority: 1000,
                mixins: vec!["example.Mixin".to_string()],
                client: Vec::new(),
                server: Vec::new(),
                default_require: 0,
                default_group: "default".to_string(),
                overwrite_require_annotations: true,
            }),
        });

        let candidates = CandidateUniverse::build(&scanned, &registry);

        assert!(candidates.invalid_contributions.is_empty());
        assert!(
            candidates
                .contribution_kinds
                .values()
                .any(|kind| *kind == MethodContributionKind::AddNewMethod)
        );
    }

    #[test]
    fn overwrite_must_target_a_method_declared_by_the_target_class() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods.clear();
        scanned.artifacts[0].classes[0]
            .interfaces
            .push("game/Contract".to_string());
        scanned.artifacts[0].classes.push(ParsedClass {
            definition_id: None,
            future_version_best_effort: false,
            name: "game/Contract".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            annotations: Vec::new(),
            fields: Vec::new(),
            methods: vec![ParsedMethod {
                name: "tick".to_string(),
                descriptor: "()V".to_string(),
                is_static: false,
                is_public: true,
                is_synthetic: false,
                annotations: Vec::new(),
                max_locals: None,
                instructions: Vec::new(),
            }],
        });
        scanned.artifacts[1].classes[0].methods = vec![ParsedMethod {
            name: "tick".to_string(),
            descriptor: "()V".to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: vec![ParsedAnnotation {
                descriptor: OVERWRITE.to_string(),
                values: BTreeMap::new(),
            }],
            max_locals: Some(1),
            instructions: vec![return_instruction(0)],
        }];
        crate::jar::rebuild_universe(&mut scanned);

        let registry = MixinRegistry::all_annotated(&scanned);
        let candidates = CandidateUniverse::build(&scanned, &registry);

        assert!(
            candidates
                .invalid_contributions
                .iter()
                .any(|invalid| { invalid.kind == MethodContributionKind::InvalidOverwriteTarget })
        );
    }

    #[test]
    fn overwrite_replacement_body_is_used_for_tail_query() {
        let mut scanned = mixin_fixture("TAIL", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts.push(overwrite_artifact(
            "overwrite",
            "example/Overwrite",
            "()V",
            1100,
            vec![return_instruction(0)],
        ));

        let analysis = analyze_detailed(&mut scanned);
        let query = analysis
            .effects
            .iter()
            .find_map(|effect| effect.queries.first())
            .unwrap();

        assert_eq!(query.selected.len(), 1);
        assert!(analysis.risks.is_empty());
    }

    #[test]
    fn overwrite_replacement_body_reports_only_a_hard_query_failure() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![
            call_instruction(0, "game/Owner", "run"),
            return_instruction(1),
        ];
        fixture_at_mut(&mut scanned).values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );
        fixture_injector_mut(&mut scanned)
            .values
            .insert("require".to_string(), AnnotationValue::Integer(1));
        scanned.artifacts.push(overwrite_artifact(
            "overwrite",
            "example/Overwrite",
            "()V",
            1100,
            vec![return_instruction(0)],
        ));

        let analysis = analyze_detailed(&mut scanned);

        assert_eq!(analysis.risks.len(), 1);
        assert_eq!(
            analysis.risks[0].rule,
            "candidate_query_minimum_unsatisfied"
        );
        assert_eq!(analysis.risks[0].activation, Activation::Definite);
    }

    #[test]
    fn equal_priority_replacement_orders_with_partial_success_are_conditional() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![
            call_instruction(0, "game/Owner", "run"),
            return_instruction(1),
        ];
        fixture_at_mut(&mut scanned).values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );
        fixture_injector_mut(&mut scanned)
            .values
            .insert("require".to_string(), AnnotationValue::Integer(1));
        scanned.artifacts.push(overwrite_artifact(
            "retains-anchor",
            "example/RetainsAnchor",
            "()V",
            1100,
            vec![
                call_instruction(0, "game/Owner", "run"),
                return_instruction(1),
            ],
        ));
        scanned.artifacts.push(overwrite_artifact(
            "removes-anchor",
            "example/RemovesAnchor",
            "()V",
            1100,
            vec![return_instruction(0)],
        ));

        let analysis = analyze_detailed(&mut scanned);
        let risk = analysis
            .risks
            .iter()
            .find(|risk| risk.rule == "candidate_query_minimum_unsatisfied")
            .unwrap();

        assert_eq!(risk.activation, Activation::Conditional);
        assert_eq!(risk.order, crate::model::OrderAnalysis::Unknown);
    }

    #[test]
    fn optional_query_removed_by_overwrite_is_an_interaction_not_a_risk() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].methods[0].instructions = vec![
            call_instruction(0, "game/Owner", "run"),
            return_instruction(1),
        ];
        fixture_at_mut(&mut scanned).values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );
        fixture_injector_mut(&mut scanned)
            .values
            .insert("require".to_string(), AnnotationValue::Integer(0));
        scanned.artifacts.push(overwrite_artifact(
            "overwrite",
            "example/Overwrite",
            "()V",
            1100,
            vec![return_instruction(0)],
        ));

        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.risks.is_empty());
        assert!(analysis.interactions.iter().any(|interaction| {
            interaction.kind == BehavioralInteractionKind::OptionalInjectionAffected
        }));
    }

    #[test]
    fn modify_variable_head_args_only_resolves_argument_slot_after_overwrite() {
        let mut scanned = mixin_fixture(
            "HEAD",
            "Lorg/spongepowered/asm/mixin/injection/ModifyVariable;",
        );
        scanned.artifacts[0].classes[0].methods[0].descriptor = "(I)V".to_string();
        let injector = fixture_injector_mut(&mut scanned);
        injector.values.insert(
            "method".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String("tick(I)V".to_string())]),
        );
        injector
            .values
            .insert("argsOnly".to_string(), AnnotationValue::Boolean(true));
        injector
            .values
            .insert("require".to_string(), AnnotationValue::Integer(1));
        scanned.artifacts[1].classes[0].methods[0].descriptor = "(I)I".to_string();
        scanned.artifacts.push(overwrite_artifact(
            "overwrite",
            "example/Overwrite",
            "(I)V",
            1100,
            vec![return_instruction(0)],
        ));

        let analysis = analyze_detailed(&mut scanned);
        let query = analysis
            .effects
            .iter()
            .find_map(|effect| effect.queries.first())
            .unwrap();

        assert_eq!(
            query
                .local_selector
                .as_ref()
                .and_then(|selector| selector.slot),
            Some(1)
        );
        assert_eq!(query.selected[0].local_slot, Some(1));
        assert!(analysis.risks.is_empty());
    }

    #[test]
    fn two_modify_variable_injectors_on_the_same_argument_are_an_interaction() {
        let mut scanned = mixin_fixture(
            "HEAD",
            "Lorg/spongepowered/asm/mixin/injection/ModifyVariable;",
        );
        scanned.artifacts[0].classes[0].methods[0].descriptor = "(I)V".to_string();
        scanned.artifacts[1].classes[0].methods[0].descriptor = "(I)I".to_string();
        fixture_injector_mut(&mut scanned)
            .values
            .insert("argsOnly".to_string(), AnnotationValue::Boolean(true));
        fixture_injector_mut(&mut scanned).values.insert(
            "method".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String("tick(I)V".to_string())]),
        );
        let mut second = scanned.artifacts[1].clone();
        second.id = "second".to_string();
        second.display_name = "second".to_string();
        second.classes[0].name = "example/SecondMixin".to_string();
        scanned.artifacts.push(second);

        let analysis = analyze_detailed(&mut scanned);
        let interactions = crate::conflict::behavioral_interactions(&analysis.effects);

        assert!(analysis.risks.is_empty());
        assert!(
            interactions.iter().any(|interaction| {
                interaction.kind == BehavioralInteractionKind::OrderedValueDecorators
            }),
            "{analysis:#?}"
        );
    }

    #[test]
    fn modify_variable_injectors_on_different_arguments_do_not_overlap() {
        let mut scanned = mixin_fixture(
            "HEAD",
            "Lorg/spongepowered/asm/mixin/injection/ModifyVariable;",
        );
        scanned.artifacts[0].classes[0].methods[0].descriptor = "(II)V".to_string();
        scanned.artifacts[1].classes[0].methods[0].descriptor = "(I)I".to_string();
        let injector = fixture_injector_mut(&mut scanned);
        injector
            .values
            .insert("argsOnly".to_string(), AnnotationValue::Boolean(true));
        injector
            .values
            .insert("index".to_string(), AnnotationValue::Integer(1));
        injector.values.insert(
            "method".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String("tick(II)V".to_string())]),
        );
        let mut second = scanned.artifacts[1].clone();
        second.id = "second".to_string();
        second.display_name = "second".to_string();
        second.classes[0].name = "example/SecondMixin".to_string();
        second.classes[0].methods[0].annotations[0]
            .values
            .insert("index".to_string(), AnnotationValue::Integer(2));
        scanned.artifacts.push(second);

        let analysis = analyze_detailed(&mut scanned);
        let interactions = crate::conflict::behavioral_interactions(&analysis.effects);

        assert!(analysis.risks.is_empty());
        assert!(interactions.iter().all(|interaction| {
            interaction.kind != BehavioralInteractionKind::OrderedValueDecorators
        }));
    }

    #[test]
    fn priority_resolves_two_normal_method_contributions_as_interaction() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts.push(overwrite_artifact(
            "low",
            "example/Low",
            "()V",
            900,
            vec![return_instruction(0)],
        ));
        let mut high = overwrite_artifact(
            "high",
            "example/High",
            "()V",
            1200,
            vec![return_instruction(0)],
        );
        high.classes[0].methods[0].annotations.clear();
        scanned.artifacts.push(high);

        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.interactions.iter().any(|interaction| {
            interaction.kind == BehavioralInteractionKind::OrderedMethodContributions
        }));
        assert_eq!(
            analysis
                .effects
                .iter()
                .filter(|effect| {
                    effect
                        .mutations
                        .iter()
                        .any(|mutation| mutation.kind == MutationKind::ReplaceMethodBody)
                })
                .count(),
            1
        );
    }

    #[test]
    fn invalid_active_overwrite_is_a_structural_merge_risk() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        let mut invalid = overwrite_artifact(
            "invalid",
            "example/InvalidOverwrite",
            "()V",
            1100,
            vec![return_instruction(0)],
        );
        invalid.classes[0].methods[0].name = "missing".to_string();
        scanned.artifacts.push(invalid);

        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.unary_risks.iter().any(|risk| {
            risk.rule == "invalid_overwrite_target"
                && risk.precision == Precision::Method
                && risk.activation == Activation::Definite
        }));
    }

    #[test]
    fn plugin_controlled_overwrite_is_not_a_definite_unary_risk() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        let mut invalid = overwrite_artifact(
            "controlled",
            "example/ControlledOverwrite",
            "()V",
            1100,
            vec![return_instruction(0)],
        );
        invalid.classes[0].methods[0].name = "missing".to_string();
        scanned.artifacts.push(invalid);
        crate::jar::rebuild_universe(&mut scanned);
        let mut registry = MixinRegistry::all_annotated(&scanned);
        registry
            .mixins
            .iter_mut()
            .find(|mixin| mixin.mixin_class == "example/ControlledOverwrite")
            .unwrap()
            .activation = crate::model::MixinActivation::PluginControlled;

        let analysis = analyze_with_progress(&mut scanned, &registry, None);

        assert!(
            analysis
                .unary_risks
                .iter()
                .all(|risk| risk.artifact_id != "controlled")
        );
        assert!(analysis.effects.iter().any(|effect| {
            effect.artifact_id == "controlled" && effect.activation == Activation::Conditional
        }));
    }

    #[test]
    fn ambiguous_target_definition_is_a_coverage_gap_not_a_risk() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        let mut duplicate = scanned.artifacts[0].clone();
        duplicate.id = "duplicate-runtime".to_string();
        duplicate.kind = ArtifactKind::Runtime;
        scanned.artifacts.push(duplicate);

        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.unary_risks.is_empty());
        assert!(analysis.coverage_gaps.iter().any(|gap| {
            gap.kind == CoverageGapKind::AmbiguousClassDefinition
                && gap.scope.contains("game/Target")
        }));
    }

    #[test]
    fn missing_non_pseudo_target_is_inactive_without_reference_warnings() {
        let mut scanned = mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes.clear();

        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.effects.is_empty());
        assert!(analysis.unary_risks.is_empty());
        assert!(
            analysis
                .inactive_candidates
                .iter()
                .any(|candidate| { candidate.kind == InactiveCandidateKind::MissingTarget })
        );
        assert!(
            scanned
                .warnings
                .iter()
                .all(|warning| warning.kind != WarningKind::UnresolvedSoftReference)
        );
    }

    #[test]
    fn overwrite_target_is_resolved_after_runtime_namespace_projection() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[0].classes[0].name = "a".to_string();
        scanned.artifacts[0].classes[0].methods[0].name = "x".to_string();
        let mut overwrite = overwrite_artifact(
            "mapped-overwrite",
            "example/MappedOverwrite",
            "()V",
            1100,
            vec![return_instruction(0)],
        );
        overwrite.classes[0].annotations[0].values.insert(
            "value".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::Class(
                "Lnet/minecraft/class_1;".to_string(),
            )]),
        );
        overwrite.classes[0].methods[0].name = "method_1".to_string();
        scanned.artifacts.push(overwrite);
        scanned.artifacts.push(ParsedArtifact {
            id: "mapping".to_string(),
            display_name: "mapping".to_string(),
            kind: ArtifactKind::Runtime,
            classes: Vec::new(),
            refmaps: Vec::new(),
            resources: vec![crate::jar::ResourceEntry {
                path: "mappings/mappings.tiny".to_string(),
                bytes: b"v1\tofficial\tintermediary\n\
                         CLASS\ta\tnet/minecraft/class_1\n\
                         METHOD\ta\t()V\tx\tmethod_1\n"
                    .to_vec(),
            }],
        });

        crate::namespace::align_runtime_namespace(&mut scanned, crate::model::LoaderFamily::Fabric)
            .unwrap();
        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.unary_risks.iter().all(|risk| {
            risk.artifact_id != "mapped-overwrite" || risk.rule != "invalid_overwrite_target"
        }));
    }

    #[test]
    fn dynamic_member_selector_resolves_after_another_mixin_adds_the_method() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        fixture_injector_mut(&mut scanned).values.insert(
            "method".to_string(),
            AnnotationValue::Array(vec![AnnotationValue::String("dynamic()V".to_string())]),
        );
        scanned.artifacts[1].classes[0].methods[0]
            .annotations
            .push(ParsedAnnotation {
                descriptor: "Lorg/spongepowered/asm/mixin/Dynamic;".to_string(),
                values: BTreeMap::new(),
            });
        let mut provider = overwrite_artifact(
            "provider",
            "example/Provider",
            "()V",
            900,
            vec![return_instruction(0)],
        );
        provider.classes[0].methods[0].name = "dynamic".to_string();
        provider.classes[0].methods[0].annotations.clear();
        scanned.artifacts.push(provider);

        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.effects.iter().any(|effect| {
            effect.queries.iter().any(|query| {
                query
                    .method
                    .member
                    .as_ref()
                    .is_some_and(|member| member.name == "dynamic" && member.descriptor == "()V")
            })
        }));
        assert!(
            scanned
                .warnings
                .iter()
                .all(|warning| { warning.kind != WarningKind::UnresolvedSoftReference })
        );
    }

    #[test]
    fn pseudo_mixin_with_absent_target_is_inactive_without_selector_warnings() {
        let mut scanned = mixin_fixture("HEAD", "Lorg/spongepowered/asm/mixin/injection/Inject;");
        scanned.artifacts[1].classes[0]
            .annotations
            .push(ParsedAnnotation {
                descriptor: PSEUDO.to_string(),
                values: BTreeMap::new(),
            });
        let AnnotationValue::Array(targets) = scanned.artifacts[1].classes[0].annotations[0]
            .values
            .get_mut("value")
            .unwrap()
        else {
            panic!("fixture target list");
        };
        targets[0] = AnnotationValue::Class("Loptional/MissingTarget;".to_string());

        let analysis = analyze_detailed(&mut scanned);

        assert!(analysis.effects.is_empty());
        assert!(
            analysis
                .inactive_candidates
                .iter()
                .any(|candidate| { candidate.kind == InactiveCandidateKind::PseudoTargetMissing })
        );
        assert!(
            scanned
                .warnings
                .iter()
                .all(|warning| { warning.kind != WarningKind::UnresolvedSoftReference })
        );
    }

    #[test]
    fn effects_record_config_mixin_and_injector_order_independently() {
        let mut scanned =
            mixin_fixture("INVOKE", "Lorg/spongepowered/asm/mixin/injection/Redirect;");
        scanned.artifacts[0].classes[0].methods[0].instructions =
            vec![call_instruction(0, "game/Owner", "run")];
        fixture_at_mut(&mut scanned).values.insert(
            "target".to_string(),
            AnnotationValue::String("Lgame/Owner;run()V".to_string()),
        );

        let analysis = analyze_detailed(&mut scanned);
        let effect = analysis
            .effects
            .iter()
            .find(|effect| {
                effect
                    .mutations
                    .iter()
                    .any(|mutation| mutation.kind == MutationKind::RedirectOperation)
            })
            .unwrap();

        assert_eq!(effect.config_priority, Some(1000));
        assert_eq!(effect.mixin_priority, Some(1000));
        assert_eq!(effect.injector_order, Some(10_000));
    }

    fn unique_annotation() -> ParsedAnnotation {
        ParsedAnnotation {
            descriptor: UNIQUE.to_string(),
            values: BTreeMap::new(),
        }
    }

    fn overwrite_artifact(
        artifact_id: &str,
        mixin_name: &str,
        descriptor: &str,
        priority: i64,
        instructions: Vec<ParsedInstruction>,
    ) -> ParsedArtifact {
        ParsedArtifact {
            id: artifact_id.to_string(),
            display_name: artifact_id.to_string(),
            kind: ArtifactKind::Mod,
            classes: vec![ParsedClass {
                definition_id: None,
                future_version_best_effort: false,
                name: mixin_name.to_string(),
                super_name: Some("java/lang/Object".to_string()),
                interfaces: Vec::new(),
                annotations: vec![ParsedAnnotation {
                    descriptor: MIXIN.to_string(),
                    values: BTreeMap::from([
                        (
                            "value".to_string(),
                            AnnotationValue::Array(vec![AnnotationValue::Class(
                                "Lgame/Target;".to_string(),
                            )]),
                        ),
                        ("priority".to_string(), AnnotationValue::Integer(priority)),
                    ]),
                }],
                fields: Vec::new(),
                methods: vec![ParsedMethod {
                    name: "tick".to_string(),
                    descriptor: descriptor.to_string(),
                    is_static: false,
                    is_public: true,
                    is_synthetic: false,
                    annotations: vec![ParsedAnnotation {
                        descriptor: OVERWRITE.to_string(),
                        values: BTreeMap::new(),
                    }],
                    max_locals: Some(2),
                    instructions,
                }],
            }],
            refmaps: Vec::new(),
            resources: Vec::new(),
        }
    }

    fn fixture_at_mut(scanned: &mut ScannedArtifacts) -> &mut ParsedAnnotation {
        let injector = fixture_injector_mut(scanned);
        let AnnotationValue::Array(ats) = injector.values.get_mut("at").unwrap() else {
            panic!("fixture @At must be an array");
        };
        let AnnotationValue::Annotation(at) = &mut ats[0] else {
            panic!("fixture @At must be an annotation");
        };
        at
    }

    fn fixture_injector_mut(scanned: &mut ScannedArtifacts) -> &mut ParsedAnnotation {
        &mut scanned.artifacts[1].classes[0].methods[0].annotations[0]
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
                identity: None,
                stable_id,
                original_offset: Some(stable_id),
                opcode: 182,
                local_slot: None,
                member: Some(member.clone()),
                constant: None,
            },
            kind: InstructionKind::MethodCall(member),
        }
    }

    fn string_instruction(stable_id: u32, value: &str) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                identity: None,
                stable_id,
                original_offset: Some(stable_id),
                opcode: 18,
                local_slot: None,
                member: None,
                constant: Some(value.to_string()),
            },
            kind: InstructionKind::StringConstant(value.to_string()),
        }
    }

    fn integer_instruction(stable_id: u32, value: i64) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                identity: None,
                stable_id,
                original_offset: Some(stable_id),
                opcode: if value == 0 { 3 } else { 16 },
                local_slot: None,
                member: None,
                constant: Some(value.to_string()),
            },
            kind: InstructionKind::IntegerConstant(value),
        }
    }

    fn type_instruction(stable_id: u32, opcode: u8, class: &str) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                identity: None,
                stable_id,
                original_offset: Some(stable_id),
                opcode,
                local_slot: None,
                member: None,
                constant: Some(class.to_string()),
            },
            kind: InstructionKind::Type(class.to_string()),
        }
    }

    fn return_instruction(stable_id: u32) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                identity: None,
                stable_id,
                original_offset: Some(stable_id),
                opcode: 177,
                local_slot: None,
                member: None,
                constant: None,
            },
            kind: InstructionKind::Return,
        }
    }

    fn mixin_fixture(at_kind: &str, injector_descriptor: &str) -> ScannedArtifacts {
        let target = ParsedClass {
            definition_id: None,
            future_version_best_effort: false,
            name: "game/Target".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
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
            definition_id: None,
            future_version_best_effort: false,
            name: "example/Mixin".to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
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
                    resources: Vec::new(),
                },
                ParsedArtifact {
                    id: "mod".to_string(),
                    display_name: "mod".to_string(),
                    kind: ArtifactKind::Mod,
                    classes: vec![mixin],
                    refmaps: Vec::new(),
                    resources: Vec::new(),
                },
            ],
            universe: ClassUniverse::default(),
            limits: AnalysisLimits::default(),
            coverage: Coverage::default(),
            warnings: Vec::new(),
            symbol_mappings: BTreeMap::new(),
        }
    }
}
