use std::collections::{BTreeSet, HashSet, VecDeque};

use crate::classfile::{InstructionKind, ParsedClass, ParsedInstruction, ParsedMethod};
use crate::jar::{ParsedArtifact, ScannedArtifacts};
use crate::model::{
    Activation, Confidence, CoverageGap, Effect, Evidence, InactiveCandidate,
    InactiveCandidateKind, LoaderFamily, Mechanism, MemberKind, MemberReference, Mutation,
    MutationKind, Precision, Readiness, RequirementKind, ShapeRequirement, Target,
};

const ITRANSFORMER: &str = "cpw/mods/modlauncher/api/ITransformer";
const TRANSFORMATION_SERVICE: &str = "cpw/mods/modlauncher/api/ITransformationService";

#[derive(Debug, Clone)]
struct RecoveredTarget {
    target: Target,
    detail: String,
}

#[derive(Debug, Clone)]
struct InstructionPattern {
    member: Option<MemberReference>,
    constant: Option<String>,
    integer: Option<i64>,
    opcode: Option<u8>,
    detail: String,
}

#[derive(Debug, Clone)]
struct MutationSignal {
    kind: MutationKind,
    source_class: String,
    source_method: String,
    source_instruction: crate::model::InstructionReference,
    pattern: Option<InstructionPattern>,
    detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MethodKey {
    owner: String,
    name: String,
    descriptor: String,
}

#[derive(Debug, Default)]
pub(crate) struct TransformerAnalysis {
    pub effects: Vec<Effect>,
    pub inactive_candidates: Vec<InactiveCandidate>,
    pub coverage_gaps: Vec<CoverageGap>,
}

pub(crate) fn analyze_with_progress(
    scanned: &mut ScannedArtifacts,
    readiness: &Readiness,
    progress: Option<&crate::progress::AuditProgressReporter>,
) -> TransformerAnalysis {
    use crate::progress::{AuditProgressEvent, AuditProgressStage, emit};

    if !matches!(
        readiness.loader,
        Some(LoaderFamily::Forge | LoaderFamily::NeoForge)
    ) {
        emit(
            progress,
            AuditProgressEvent::StageStarted {
                stage: AuditProgressStage::AnalyzeTransformers,
                total: Some(0),
            },
        );
        emit(
            progress,
            AuditProgressEvent::StageFinished {
                stage: AuditProgressStage::AnalyzeTransformers,
                completed: 0,
            },
        );
        return TransformerAnalysis::default();
    }

    let mut effects = Vec::new();
    let mut inactive_candidates = Vec::new();
    let mut coverage_gaps = Vec::new();
    let mod_indexes = scanned
        .artifacts
        .iter()
        .enumerate()
        .filter_map(|(index, artifact)| {
            (artifact.kind == crate::model::ArtifactKind::Mod).then_some(index)
        })
        .collect::<Vec<_>>();
    let total = mod_indexes
        .iter()
        .map(|index| {
            discover_registered_transformer_classes(scanned, &scanned.artifacts[*index]).len()
        })
        .sum();
    emit(
        progress,
        AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::AnalyzeTransformers,
            total: Some(total),
        },
    );
    let mut completed = 0;

    macro_rules! complete_and_continue {
        () => {{
            completed += 1;
            emit(
                progress,
                AuditProgressEvent::Advanced {
                    stage: AuditProgressStage::AnalyzeTransformers,
                    completed,
                    total: Some(total),
                },
            );
            continue;
        }};
    }

    for artifact_index in mod_indexes {
        let artifact = &scanned.artifacts[artifact_index];
        let transformer_classes = discover_registered_transformer_classes(scanned, artifact);
        for class_name in
            discover_transformer_candidates(scanned, artifact).difference(&transformer_classes)
        {
            inactive_candidates.push(InactiveCandidate {
                artifact_id: artifact.id.clone(),
                class: Some(class_name.clone()),
                config_path: None,
                kind: InactiveCandidateKind::UnregisteredTransformer,
                reason: "ITransformer implementation is not reachable from ITransformationService.transformers()"
                    .to_string(),
            });
        }
        for class_name in transformer_classes {
            let Some(class) = artifact
                .classes
                .iter()
                .find(|class| class.name == class_name)
            else {
                complete_and_continue!();
            };
            scanned.coverage.transformers_discovered += 1;
            let mechanism = transformer_mechanism(scanned, class);
            let targets = recover_targets(class);
            if targets.is_empty() {
                scanned.coverage.transformer_effects_unknown += 1;
                coverage_gaps.push(CoverageGap {
                    artifact_id: Some(artifact.id.clone()),
                    scope: class.name.clone(),
                    kind: crate::model::CoverageGapKind::TransformerUnknown,
                    detail: "ITransformer target set is dynamic or could not be recovered; \
                             no global pairwise conflicts were fabricated"
                        .to_string(),
                    count: 1,
                });
                complete_and_continue!();
            }
            scanned.coverage.transformer_targets_recovered += targets.len();

            let (signals, exhausted, ignored_untainted) =
                recover_mutations(scanned, artifact, class, &mut coverage_gaps);
            if ignored_untainted > 0 {
                scanned.coverage.transformer_effects_partial += 1;
                coverage_gaps.push(CoverageGap {
                    artifact_id: Some(artifact.id.clone()),
                    scope: class.name.clone(),
                    kind: crate::model::CoverageGapKind::TransformerPartial,
                    detail: format!(
                        "ignored {ignored_untainted} ASM-looking write(s) whose receiver \
                         could not be traced to the transform() input"
                    ),
                    count: ignored_untainted,
                });
            }
            if exhausted {
                scanned.coverage.budget_exhaustions.push(format!(
                    "{}:{} bounded transformer interpretation",
                    artifact.id, class.name
                ));
                coverage_gaps.push(CoverageGap {
                    artifact_id: Some(artifact.id.clone()),
                    scope: class.name.clone(),
                    kind: crate::model::CoverageGapKind::BudgetExhaustion,
                    detail: "bounded transformer interpretation exhausted its state budget"
                        .to_string(),
                    count: 1,
                });
            }
            if signals.is_empty() {
                coverage_gaps.push(CoverageGap {
                    artifact_id: Some(artifact.id.clone()),
                    scope: class.name.clone(),
                    kind: crate::model::CoverageGapKind::TransformerUnknown,
                    detail: "transform() was found, but no supported ASM mutation was recovered"
                        .to_string(),
                    count: targets.len(),
                });
                for target in targets {
                    scanned.coverage.transformer_effects_unknown += 1;
                    effects.push(unknown_effect(
                        artifact,
                        class,
                        target,
                        mechanism,
                        "transform() was found, but no supported ASM mutation was recovered",
                    ));
                }
                complete_and_continue!();
            }

            if targets.len() > 1 {
                scanned.coverage.transformer_effects_partial += targets.len();
                coverage_gaps.push(CoverageGap {
                    artifact_id: Some(artifact.id.clone()),
                    scope: class.name.clone(),
                    kind: crate::model::CoverageGapKind::TransformerPartial,
                    detail: "multiple transformer targets and mutation branches could not be \
                             associated without path-sensitive stack analysis; effects were \
                             kept as unknown per target"
                        .to_string(),
                    count: targets.len(),
                });
                for target in targets {
                    effects.push(unknown_effect(
                        artifact,
                        class,
                        target,
                        mechanism,
                        "target-to-mutation branch association is heuristic",
                    ));
                }
                complete_and_continue!();
            }

            for target in &targets {
                for signal in &signals {
                    let recovered =
                        effects_for_signal(scanned, artifact, target, signal, mechanism);
                    if recovered
                        .iter()
                        .all(|effect| matches!(effect.precision, Precision::Instruction))
                    {
                        scanned.coverage.transformer_effects_recovered += recovered.len();
                    } else {
                        scanned.coverage.transformer_effects_partial += recovered.len();
                    }
                    effects.extend(recovered);
                }
            }
            completed += 1;
            emit(
                progress,
                AuditProgressEvent::Advanced {
                    stage: AuditProgressStage::AnalyzeTransformers,
                    completed,
                    total: Some(total),
                },
            );
        }
    }

    emit(
        progress,
        AuditProgressEvent::StageFinished {
            stage: AuditProgressStage::AnalyzeTransformers,
            completed,
        },
    );
    TransformerAnalysis {
        effects,
        inactive_candidates,
        coverage_gaps,
    }
}

fn discover_transformer_candidates(
    scanned: &ScannedArtifacts,
    artifact: &ParsedArtifact,
) -> BTreeSet<String> {
    artifact
        .classes
        .iter()
        .filter(|class| inherits(scanned, &class.name, ITRANSFORMER, &mut HashSet::new()))
        .map(|class| class.name.clone())
        .collect()
}

fn discover_registered_transformer_classes(
    scanned: &ScannedArtifacts,
    artifact: &ParsedArtifact,
) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    let registered_services = registered_transformation_services(artifact);

    // ITransformationService#transformers commonly constructs anonymous or
    // nested transformer classes. Inspecting those factories also catches
    // implementations reached through a static helper or invokedynamic.
    for service in artifact.classes.iter().filter(|class| {
        registered_services.contains(&class.name)
            && inherits(
                scanned,
                &class.name,
                TRANSFORMATION_SERVICE,
                &mut HashSet::new(),
            )
    }) {
        let mut queue = service
            .methods
            .iter()
            .filter(|method| method.name == "transformers")
            .map(|method| MethodKey {
                owner: service.name.clone(),
                name: method.name.clone(),
                descriptor: method.descriptor.clone(),
            })
            .collect::<VecDeque<_>>();
        let mut visited = HashSet::new();
        while let Some(key) = queue.pop_front() {
            if !visited.insert(key.clone()) {
                continue;
            }
            let Some((owner, method)) = find_artifact_method(artifact, &key) else {
                continue;
            };
            for instruction in &method.instructions {
                match &instruction.kind {
                    InstructionKind::Type(candidate)
                        if inherits(scanned, candidate, ITRANSFORMER, &mut HashSet::new()) =>
                    {
                        result.insert(candidate.clone());
                    }
                    InstructionKind::MethodCall(member)
                        if artifact
                            .classes
                            .iter()
                            .any(|class| class.name == member.owner) =>
                    {
                        queue.push_back(MethodKey {
                            owner: member.owner.clone(),
                            name: member.name.clone(),
                            descriptor: member.descriptor.clone(),
                        });
                    }
                    InstructionKind::InvokeDynamic {
                        implementation: Some(member),
                        ..
                    } if artifact
                        .classes
                        .iter()
                        .any(|class| class.name == member.owner) =>
                    {
                        queue.push_back(MethodKey {
                            owner: member.owner.clone(),
                            name: member.name.clone(),
                            descriptor: member.descriptor.clone(),
                        });
                    }
                    _ => {}
                }
            }
            if inherits(scanned, &owner.name, ITRANSFORMER, &mut HashSet::new()) {
                result.insert(owner.name.clone());
            }
        }
    }
    result
}

fn registered_transformation_services(artifact: &ParsedArtifact) -> BTreeSet<String> {
    const SERVICE_PATH: &str = "meta-inf/services/cpw.mods.modlauncher.api.itransformationservice";
    let mut services = BTreeSet::new();
    for resource in artifact.resources.iter().filter(|resource| {
        resource
            .path
            .rsplit("!/")
            .next()
            .is_some_and(|path| path.to_ascii_lowercase() == SERVICE_PATH)
    }) {
        for line in String::from_utf8_lossy(&resource.bytes).lines() {
            let service = line.split('#').next().unwrap_or_default().trim();
            if !service.is_empty() {
                services.insert(service.replace('.', "/"));
            }
        }
    }
    services
}

fn inherits(
    scanned: &ScannedArtifacts,
    class: &str,
    expected: &str,
    visited: &mut HashSet<String>,
) -> bool {
    if class == expected {
        return true;
    }
    if !visited.insert(class.to_string()) {
        return false;
    }
    scanned
        .universe
        .definitions(class)
        .iter()
        .any(|definition| {
            definition.interfaces.iter().any(|interface| {
                interface == expected || inherits(scanned, interface, expected, visited)
            }) || definition.super_name.as_deref().is_some_and(|super_name| {
                super_name == expected || inherits(scanned, super_name, expected, visited)
            })
        })
}

fn transformer_mechanism(scanned: &ScannedArtifacts, class: &ParsedClass) -> Mechanism {
    let coremod_name = class.name.to_ascii_lowercase().contains("coremod")
        || class
            .interfaces
            .iter()
            .any(|name| name.to_ascii_lowercase().contains("coremod"))
        || class
            .super_name
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().contains("coremod"));
    if coremod_name
        || inherits(
            scanned,
            &class.name,
            "net/minecraftforge/coremod/api/ICoreMod",
            &mut HashSet::new(),
        )
    {
        Mechanism::JavaCoremod
    } else {
        Mechanism::ModLauncherTransformer
    }
}

fn recover_targets(class: &ParsedClass) -> Vec<RecoveredTarget> {
    let mut targets = Vec::new();
    for method in class
        .methods
        .iter()
        .filter(|method| method.name == "targets")
    {
        let mut stack = Vec::<Option<String>>::new();
        for instruction in &method.instructions {
            match &instruction.kind {
                InstructionKind::StringConstant(value) => stack.push(Some(value.clone())),
                InstructionKind::IntegerConstant(_)
                | InstructionKind::DecimalConstant(_)
                | InstructionKind::NullConstant
                | InstructionKind::Type(_)
                | InstructionKind::FieldRead(_)
                | InstructionKind::Load(_) => stack.push(None),
                InstructionKind::Store(_) | InstructionKind::FieldWrite(_) => {
                    stack.pop();
                }
                InstructionKind::MethodCall(member) => {
                    let arguments = pop_call_arguments(&mut stack, member);
                    if (member.owner.ends_with("/ITransformer$Target")
                        || member.owner == "cpw/mods/modlauncher/api/ITransformer$Target")
                        && let Some(arguments) = arguments.as_deref()
                        && let Some(target) = target_from_factory_arguments(&member.name, arguments)
                    {
                        targets.push(RecoveredTarget {
                            detail: format!(
                                "recovered from the operand stack at {}.{}{} call {}",
                                class.name, method.name, method.descriptor, member.name
                            ),
                            target,
                        });
                    }
                    if !method_returns_void(&member.descriptor) {
                        stack.push(None);
                    }
                }
                InstructionKind::InvokeDynamic { descriptor, .. } => {
                    let count = descriptor_argument_count(descriptor).unwrap_or(0);
                    for _ in 0..count {
                        stack.pop();
                    }
                    if !method_returns_void(descriptor) {
                        stack.push(None);
                    }
                }
                InstructionKind::Jump => stack.clear(),
                InstructionKind::Return => stack.clear(),
                InstructionKind::Other => {}
            }
        }
    }
    targets.sort_by_key(|target| target_key(&target.target));
    targets.dedup_by(|left, right| left.target == right.target);
    targets
}

fn pop_call_arguments(
    stack: &mut Vec<Option<String>>,
    member: &MemberReference,
) -> Option<Vec<Option<String>>> {
    let count = descriptor_argument_count(&member.descriptor)?;
    if stack.len() < count {
        stack.clear();
        return None;
    }
    let split = stack.len() - count;
    let arguments = stack.split_off(split);
    if member.is_static == Some(false) {
        stack.pop();
    }
    Some(arguments)
}

fn descriptor_argument_count(descriptor: &str) -> Option<usize> {
    let arguments = descriptor.strip_prefix('(')?.split_once(')')?.0;
    let bytes = arguments.as_bytes();
    let mut count = 0_usize;
    let mut offset = 0_usize;
    while offset < bytes.len() {
        while bytes.get(offset) == Some(&b'[') {
            offset += 1;
        }
        if bytes.get(offset) == Some(&b'L') {
            offset += arguments[offset..].find(';')? + 1;
        } else {
            offset += 1;
        }
        count += 1;
    }
    Some(count)
}

fn method_returns_void(descriptor: &str) -> bool {
    descriptor
        .split_once(')')
        .is_some_and(|(_, result)| result == "V")
}

fn target_from_factory_arguments(name: &str, values: &[Option<String>]) -> Option<Target> {
    let string = |index: usize| values.get(index)?.as_deref();
    match name {
        "targetClass" | "targetPreClass" => Some(Target::class(normalize_class(string(0)?)?)),
        "targetMethod" => Some(Target::method(
            normalize_class(string(0)?)?,
            string(1)?,
            string(2)?,
        )),
        "targetField" => {
            let owner = normalize_class(string(0)?)?;
            Some(Target {
                class: owner.clone(),
                member: Some(MemberReference {
                    owner,
                    name: string(1)?.to_string(),
                    descriptor: String::new(),
                    kind: MemberKind::Field,
                    is_static: None,
                }),
                instruction: None,
            })
        }
        _ => None,
    }
}

fn last_strings(strings: &VecDeque<String>, count: usize) -> Option<Vec<String>> {
    if strings.len() < count {
        return None;
    }
    Some(
        strings
            .iter()
            .skip(strings.len() - count)
            .cloned()
            .collect(),
    )
}

fn recover_mutations(
    scanned: &ScannedArtifacts,
    artifact: &ParsedArtifact,
    transformer: &ParsedClass,
    coverage_gaps: &mut Vec<CoverageGap>,
) -> (Vec<MutationSignal>, bool, usize) {
    let mut queue = transformer
        .methods
        .iter()
        .filter(|method| method.name == "transform")
        .map(|method| {
            (
                MethodKey {
                    owner: transformer.name.clone(),
                    name: method.name.clone(),
                    descriptor: method.descriptor.clone(),
                },
                0_usize,
                true,
            )
        })
        .collect::<VecDeque<_>>();
    let mut visited = HashSet::new();
    let mut states = 0_usize;
    let mut signals = Vec::new();
    let mut exhausted = false;
    let mut ignored_untainted = 0_usize;

    while let Some((key, depth, tainted_input)) = queue.pop_front() {
        if !visited.insert((key.clone(), tainted_input)) {
            continue;
        }
        if depth > scanned.limits.max_helper_depth {
            exhausted = true;
            continue;
        }
        let Some((owner, method)) = find_artifact_method(artifact, &key) else {
            continue;
        };
        let mut recent_strings = VecDeque::<String>::new();
        let mut recent_integers = VecDeque::<i64>::new();
        let mut recent_types = VecDeque::<String>::new();
        let mut patterns = VecDeque::<InstructionPattern>::new();
        let mut tainted_locals = HashSet::<u16>::new();
        if tainted_input {
            tainted_locals.insert(if method.is_static { 0 } else { 1 });
        }
        let mut taint_window = usize::from(tainted_input) * 12;

        // Locals and load/store operations are deliberately included in the
        // interpreter budget: obfuscated helper chains must not evade bounds
        // merely because they contain few calls.
        states = states.saturating_add(usize::from(method.max_locals.unwrap_or(0)));
        for (index, instruction) in method.instructions.iter().enumerate() {
            states = states.saturating_add(match instruction.kind {
                InstructionKind::Load(local) | InstructionKind::Store(local) => {
                    usize::from(local).saturating_add(1)
                }
                _ => 1,
            });
            if states > scanned.limits.max_interpreter_states {
                exhausted = true;
                break;
            }
            match &instruction.kind {
                InstructionKind::Load(local) => {
                    if tainted_locals.contains(local) {
                        taint_window = 12;
                    }
                }
                InstructionKind::Store(local) => {
                    if taint_window > 0 {
                        tainted_locals.insert(*local);
                    }
                }
                InstructionKind::StringConstant(value) => {
                    push_bounded(&mut recent_strings, value.clone(), 12);
                }
                InstructionKind::IntegerConstant(value) => {
                    push_bounded(&mut recent_integers, *value, 12);
                }
                InstructionKind::FieldRead(member)
                    if taint_window > 0 && asm_node_owner(&member.owner) =>
                {
                    taint_window = 12;
                }
                InstructionKind::Type(value) => {
                    push_bounded(&mut recent_types, value.clone(), 8);
                }
                InstructionKind::InvokeDynamic {
                    name,
                    descriptor,
                    implementation,
                } => {
                    if let Some(member) = implementation {
                        if artifact
                            .classes
                            .iter()
                            .any(|class| class.name == member.owner)
                        {
                            queue.push_back((
                                MethodKey {
                                    owner: member.owner.clone(),
                                    name: member.name.clone(),
                                    descriptor: member.descriptor.clone(),
                                },
                                depth + 1,
                                taint_window > 0,
                            ));
                        }
                    } else {
                        coverage_gaps.push(CoverageGap {
                            artifact_id: Some(artifact.id.clone()),
                            scope: format!("{}.{}{}", owner.name, method.name, method.descriptor),
                            kind: crate::model::CoverageGapKind::TransformerPartial,
                            detail: format!(
                                "invokedynamic {name}{descriptor} has no recoverable implementation handle"
                            ),
                            count: 1,
                        });
                    }
                }
                InstructionKind::MethodCall(member) => {
                    if let Some(pattern) =
                        pattern_from_constructor(member, &recent_strings, &recent_integers)
                    {
                        push_bounded(&mut patterns, pattern, 8);
                    }
                    if let Some((kind, description)) = classify_call(member, &recent_types) {
                        if taint_window > 0 {
                            signals.push(MutationSignal {
                                kind,
                                source_class: owner.name.clone(),
                                source_method: format!("{}{}", method.name, method.descriptor),
                                source_instruction: instruction.reference.clone(),
                                pattern: patterns.back().cloned(),
                                detail: description,
                            });
                        } else {
                            ignored_untainted += 1;
                        }
                    }
                    if artifact
                        .classes
                        .iter()
                        .any(|class| class.name == member.owner)
                        && member.name != "<init>"
                        && member.name != "<clinit>"
                    {
                        queue.push_back((
                            MethodKey {
                                owner: member.owner.clone(),
                                name: member.name.clone(),
                                descriptor: member.descriptor.clone(),
                            },
                            depth + 1,
                            taint_window > 0,
                        ));
                    }
                }
                InstructionKind::FieldWrite(member) => {
                    if let Some((kind, description)) = classify_field_write(member) {
                        if taint_window > 0 {
                            signals.push(MutationSignal {
                                kind,
                                source_class: owner.name.clone(),
                                source_method: format!("{}{}", method.name, method.descriptor),
                                source_instruction: instruction.reference.clone(),
                                pattern: patterns.back().cloned(),
                                detail: description,
                            });
                        } else {
                            ignored_untainted += 1;
                        }
                    }
                }
                _ => {}
            }
            taint_window = taint_window.saturating_sub(1);
            if index % 128 == 127 {
                recent_strings.truncate(6);
                recent_integers.truncate(6);
                recent_types.truncate(4);
                patterns.truncate(4);
            }
        }
        if exhausted && states > scanned.limits.max_interpreter_states {
            break;
        }
    }

    signals.sort_by(|left, right| {
        left.source_class
            .cmp(&right.source_class)
            .then_with(|| left.source_method.cmp(&right.source_method))
            .then_with(|| {
                left.source_instruction
                    .stable_id
                    .cmp(&right.source_instruction.stable_id)
            })
            .then_with(|| left.kind.cmp(&right.kind))
    });
    signals.dedup_by(|left, right| {
        left.source_class == right.source_class
            && left.source_method == right.source_method
            && left.source_instruction == right.source_instruction
            && left.kind == right.kind
    });
    (signals, exhausted, ignored_untainted)
}

fn classify_call(
    member: &MemberReference,
    recent_types: &VecDeque<String>,
) -> Option<(MutationKind, String)> {
    let owner = member.owner.as_str();
    let kind = if owner.ends_with("/InsnList") {
        match member.name.as_str() {
            "add" | "insert" | "insertBefore" => MutationKind::InsertInstructions,
            "remove" => MutationKind::RemoveInstruction,
            "set" => MutationKind::ReplaceInstruction,
            "clear" => MutationKind::ReplaceMethodBody,
            _ => return None,
        }
    } else if owner.ends_with("/MethodVisitor") || owner.ends_with("/MethodNode") {
        match member.name.as_str() {
            "visitInsn"
            | "visitIntInsn"
            | "visitVarInsn"
            | "visitTypeInsn"
            | "visitFieldInsn"
            | "visitMethodInsn"
            | "visitInvokeDynamicInsn"
            | "visitJumpInsn"
            | "visitLdcInsn"
            | "visitIincInsn"
            | "visitTableSwitchInsn"
            | "visitLookupSwitchInsn"
            | "visitMultiANewArrayInsn" => MutationKind::InsertInstructions,
            "visitMaxs" | "visitLocalVariable" => MutationKind::ChangeLocalLayout,
            _ => return None,
        }
    } else if owner.ends_with("/ClassVisitor") || owner.ends_with("/ClassNode") {
        match member.name.as_str() {
            "visitMethod" => MutationKind::AddMethod,
            "visitField" => MutationKind::AddField,
            // Without evaluating the visit() operand stack we cannot prove
            // that superclass or access values differ from the input class.
            "visit" => MutationKind::UnknownClass,
            _ => return None,
        }
    } else if owner == "java/util/List" || owner.ends_with("/ArrayList") {
        let node = recent_types.back().map(String::as_str).unwrap_or_default();
        match (member.name.as_str(), node) {
            ("add", node) if node.ends_with("/MethodNode") => MutationKind::AddMethod,
            ("add", node) if node.ends_with("/FieldNode") => MutationKind::AddField,
            ("remove", node) if node.ends_with("/MethodNode") => MutationKind::RemoveMethod,
            ("remove", node) if node.ends_with("/FieldNode") => MutationKind::RemoveField,
            _ => return None,
        }
    } else if owner == "java/util/Iterator" && member.name == "remove" {
        // The iterator may originate from methods, fields, interfaces,
        // annotations, or an instruction list. Its collection provenance is
        // not available in this bounded interpreter.
        MutationKind::UnknownMethod
    } else {
        return None;
    };
    Some((
        kind,
        format!(
            "ASM mutation call {}.{}{}",
            member.owner, member.name, member.descriptor
        ),
    ))
}

fn classify_field_write(member: &MemberReference) -> Option<(MutationKind, String)> {
    let owner = member.owner.as_str();
    let kind = if owner.ends_with("/ClassNode") {
        match member.name.as_str() {
            // A raw field write does not prove that the new value differs
            // from the original class shape.
            "superName" | "interfaces" => MutationKind::UnknownClass,
            "access" => MutationKind::ChangeAccess,
            "methods" => MutationKind::AddMethod,
            "fields" => MutationKind::AddField,
            _ => return None,
        }
    } else if owner.ends_with("/MethodNode") {
        match member.name.as_str() {
            "instructions" => MutationKind::ReplaceMethodBody,
            "access" => MutationKind::ChangeAccess,
            "maxLocals" | "localVariables" => MutationKind::ChangeLocalLayout,
            "tryCatchBlocks" => MutationKind::ChangeControlFlow,
            _ => return None,
        }
    } else if owner.ends_with("/FieldNode") && member.name == "access" {
        MutationKind::ChangeAccess
    } else {
        return None;
    };
    Some((
        kind,
        format!(
            "ASM tree field write {}.{}:{}",
            member.owner, member.name, member.descriptor
        ),
    ))
}

fn pattern_from_constructor(
    member: &MemberReference,
    strings: &VecDeque<String>,
    integers: &VecDeque<i64>,
) -> Option<InstructionPattern> {
    if member.name != "<init>" {
        return None;
    }
    if member.owner.ends_with("/MethodInsnNode") {
        let values = last_strings(strings, 3)?;
        let owner = normalize_class(&values[0])?;
        let reference = MemberReference {
            owner,
            name: values[1].clone(),
            descriptor: values[2].clone(),
            kind: MemberKind::Method,
            is_static: None,
        };
        Some(InstructionPattern {
            detail: format!(
                "constructed MethodInsnNode {}.{}{}",
                reference.owner, reference.name, reference.descriptor
            ),
            member: Some(reference),
            constant: None,
            integer: None,
            opcode: integers.back().and_then(|value| u8::try_from(*value).ok()),
        })
    } else if member.owner.ends_with("/FieldInsnNode") {
        let values = last_strings(strings, 3)?;
        let owner = normalize_class(&values[0])?;
        let reference = MemberReference {
            owner,
            name: values[1].clone(),
            descriptor: values[2].clone(),
            kind: MemberKind::Field,
            is_static: None,
        };
        Some(InstructionPattern {
            detail: format!(
                "constructed FieldInsnNode {}.{}:{}",
                reference.owner, reference.name, reference.descriptor
            ),
            member: Some(reference),
            constant: None,
            integer: None,
            opcode: integers.back().and_then(|value| u8::try_from(*value).ok()),
        })
    } else if member.owner.ends_with("/LdcInsnNode") {
        let value = strings.back().cloned();
        let integer = integers.back().copied();
        if value.is_none() && integer.is_none() {
            return None;
        }
        Some(InstructionPattern {
            detail: format!(
                "constructed LdcInsnNode for constant {}",
                value.as_ref().map_or_else(
                    || integer.unwrap_or_default().to_string(),
                    |value| { format!("{value:?}") }
                )
            ),
            member: None,
            constant: value,
            integer,
            opcode: None,
        })
    } else {
        None
    }
}

fn effects_for_signal(
    scanned: &ScannedArtifacts,
    artifact: &ParsedArtifact,
    recovered_target: &RecoveredTarget,
    signal: &MutationSignal,
    mechanism: Mechanism,
) -> Vec<Effect> {
    let base_target = &recovered_target.target;
    if let (Some(member), Some(pattern)) = (&base_target.member, &signal.pattern)
        && member.kind == MemberKind::Method
    {
        let matches = match_actual_instructions(scanned, member, pattern);
        if !matches.is_empty() {
            return matches
                .into_iter()
                .map(|instruction| {
                    let mut target = base_target.clone();
                    target.instruction = Some(instruction.clone());
                    effect(
                        artifact,
                        target,
                        signal,
                        mechanism,
                        Precision::Pattern,
                        Confidence::Low,
                        vec![ShapeRequirement {
                            kind: RequirementKind::InstructionExists,
                            target: {
                                let mut required = base_target.clone();
                                required.instruction = Some(instruction);
                                required
                            },
                            precision: Precision::Pattern,
                            minimum_matches: Some(1),
                            maximum_matches: None,
                            ordinal: None,
                            slice: None,
                        }],
                        &recovered_target.detail,
                    )
                })
                .collect();
        }
    }

    let (precision, confidence, requirement) = if base_target.member.is_some() {
        (
            if signal.pattern.is_some() {
                Precision::Pattern
            } else {
                Precision::Method
            },
            Confidence::Low,
            RequirementKind::MemberExists,
        )
    } else {
        (
            Precision::Class,
            Confidence::Low,
            RequirementKind::ClassExists,
        )
    };
    vec![effect(
        artifact,
        base_target.clone(),
        signal,
        mechanism,
        precision,
        confidence,
        vec![ShapeRequirement {
            kind: requirement,
            target: base_target.clone(),
            precision,
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
        &recovered_target.detail,
    )]
}

fn match_actual_instructions(
    scanned: &ScannedArtifacts,
    target_member: &MemberReference,
    pattern: &InstructionPattern,
) -> Vec<crate::model::InstructionReference> {
    let mut result = Vec::new();
    for (_, class) in scanned
        .universe
        .parsed_definitions(&scanned.artifacts, &target_member.owner)
    {
        for method in class.methods.iter().filter(|method| {
            method.name == target_member.name
                && (target_member.descriptor.is_empty()
                    || method.descriptor == target_member.descriptor)
        }) {
            for instruction in &method.instructions {
                let member_matches = pattern.member.as_ref().is_none_or(|expected| {
                    instruction_member(instruction)
                        .is_some_and(|actual| member_equivalent(actual, expected))
                });
                let constant_matches = pattern.constant.as_ref().is_none_or(|expected| {
                    matches!(
                        &instruction.kind,
                        InstructionKind::StringConstant(actual) if actual == expected
                    )
                });
                let integer_matches = pattern.integer.is_none_or(|expected| {
                    matches!(
                        instruction.kind,
                        InstructionKind::IntegerConstant(actual) if actual == expected
                    )
                });
                let opcode_matches = pattern
                    .opcode
                    .is_none_or(|opcode| instruction.reference.opcode == opcode);
                if member_matches && constant_matches && integer_matches && opcode_matches {
                    result.push(instruction.reference.clone());
                }
            }
        }
    }
    result.sort_by_key(|instruction| instruction.stable_id);
    result.dedup();
    result
}

fn instruction_member(instruction: &ParsedInstruction) -> Option<&MemberReference> {
    match &instruction.kind {
        InstructionKind::MethodCall(member)
        | InstructionKind::FieldRead(member)
        | InstructionKind::FieldWrite(member) => Some(member),
        _ => None,
    }
}

fn member_equivalent(left: &MemberReference, right: &MemberReference) -> bool {
    left.owner == right.owner
        && left.name == right.name
        && left.descriptor == right.descriptor
        && left.kind == right.kind
}

fn asm_node_owner(owner: &str) -> bool {
    owner.starts_with("org/objectweb/asm/")
        && (owner.ends_with("Node") || owner.ends_with("InsnList") || owner.ends_with("Visitor"))
}

#[allow(clippy::too_many_arguments)]
fn effect(
    artifact: &ParsedArtifact,
    target: Target,
    signal: &MutationSignal,
    mechanism: Mechanism,
    precision: Precision,
    confidence: Confidence,
    requirements: Vec<ShapeRequirement>,
    target_detail: &str,
) -> Effect {
    let mut detail = format!(
        "{}; {}; source artifact {}",
        target_detail, signal.detail, artifact.display_name
    );
    if let Some(pattern) = &signal.pattern {
        detail.push_str("; ");
        detail.push_str(&pattern.detail);
    }
    Effect {
        artifact_id: artifact.id.clone(),
        mechanism,
        target: target.clone(),
        requirements,
        queries: Vec::new(),
        mutations: vec![Mutation::new(signal.kind, target, precision)],
        evidence: vec![{
            let mut evidence = Evidence::new(&artifact.id, &signal.source_class, detail);
            evidence.method = Some(signal.source_method.clone());
            evidence.instruction = Some(signal.source_instruction.clone());
            evidence.mechanism = Some(mechanism);
            evidence.composition_semantics = Some(signal.kind.default_composition());
            evidence.analysis_precision = Some(precision);
            evidence
        }],
        precision,
        confidence,
        activation: Activation::Candidate,
        config_priority: None,
        mixin_priority: None,
        injector_order: None,
    }
}

fn unknown_effect(
    artifact: &ParsedArtifact,
    transformer: &ParsedClass,
    target: RecoveredTarget,
    mechanism: Mechanism,
    reason: &str,
) -> Effect {
    let mutation_kind = if target.target.member.is_some() {
        MutationKind::UnknownMethod
    } else {
        MutationKind::UnknownClass
    };
    Effect {
        artifact_id: artifact.id.clone(),
        mechanism,
        target: target.target.clone(),
        requirements: vec![ShapeRequirement {
            kind: if target.target.member.is_some() {
                RequirementKind::MemberExists
            } else {
                RequirementKind::ClassExists
            },
            target: target.target.clone(),
            precision: if target.target.member.is_some() {
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
            mutation_kind,
            target.target.clone(),
            if target.target.member.is_some() {
                Precision::Method
            } else {
                Precision::Class
            },
        )],
        evidence: vec![{
            let mut evidence = Evidence::new(
                &artifact.id,
                &transformer.name,
                format!("{}; {reason}", target.detail),
            );
            evidence.method = Some("transform".to_string());
            evidence.mechanism = Some(mechanism);
            evidence.composition_semantics = Some(mutation_kind.default_composition());
            evidence.analysis_precision = Some(if target.target.member.is_some() {
                Precision::Method
            } else {
                Precision::Class
            });
            evidence
        }],
        precision: if target.target.member.is_some() {
            Precision::Method
        } else {
            Precision::Class
        },
        confidence: Confidence::Low,
        activation: Activation::Candidate,
        config_priority: None,
        mixin_priority: None,
        injector_order: None,
    }
}

fn find_artifact_method<'a>(
    artifact: &'a ParsedArtifact,
    key: &MethodKey,
) -> Option<(&'a ParsedClass, &'a ParsedMethod)> {
    let class = artifact
        .classes
        .iter()
        .find(|class| class.name == key.owner)?;
    let method = class
        .methods
        .iter()
        .find(|method| method.name == key.name && method.descriptor == key.descriptor)?;
    Some((class, method))
}

fn normalize_class(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.strip_prefix('L').unwrap_or(value);
    let value = value.strip_suffix(';').unwrap_or(value);
    (!value.is_empty()).then(|| value.replace('.', "/"))
}

fn target_key(target: &Target) -> String {
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

fn push_bounded<T>(values: &mut VecDeque<T>, value: T, limit: usize) {
    values.push_back(value);
    while values.len() > limit {
        values.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use crate::classfile::{ParsedClass, ParsedInstruction, ParsedMethod};
    use crate::jar::{ClassDefinition, ClassUniverse, ParsedArtifact, ResourceEntry};
    use crate::model::{AnalysisLimits, ArtifactKind, Coverage, InstructionReference};

    use super::*;

    #[test]
    fn target_factories_recover_class_method_and_field() {
        let mut values = vec![
            Some("net.minecraft.client.Minecraft".to_string()),
            Some("runTick".to_string()),
            Some("()V".to_string()),
        ];
        assert_eq!(
            target_from_factory_arguments("targetMethod", &values)
                .unwrap()
                .member
                .unwrap()
                .name,
            "runTick"
        );
        values = vec![
            Some("net.minecraft.client.Minecraft".to_string()),
            Some("level".to_string()),
        ];
        assert_eq!(
            target_from_factory_arguments("targetField", &values)
                .unwrap()
                .member
                .unwrap()
                .name,
            "level"
        );
        values = vec![Some("net.minecraft.client.Minecraft".to_string())];
        assert_eq!(
            target_from_factory_arguments("targetClass", &values)
                .unwrap()
                .class,
            "net/minecraft/client/Minecraft"
        );
    }

    #[test]
    fn target_recovery_uses_operand_stack_and_ignores_consumed_log_strings() {
        let logger = MemberReference {
            owner: "org/slf4j/Logger".to_string(),
            name: "info".to_string(),
            descriptor: "(Ljava/lang/String;)V".to_string(),
            kind: MemberKind::Method,
            is_static: Some(false),
        };
        let target_class = MemberReference {
            owner: "cpw/mods/modlauncher/api/ITransformer$Target".to_string(),
            name: "targetClass".to_string(),
            descriptor: "(Ljava/lang/String;)Lcpw/mods/modlauncher/api/ITransformer$Target;"
                .to_string(),
            kind: MemberKind::Method,
            is_static: Some(true),
        };
        let class = empty_class(
            "example/Transformer",
            vec![ITRANSFORMER.to_string()],
            vec![method(
                "targets",
                vec![
                    instruction(
                        0,
                        InstructionKind::StringConstant("not.a.Target".to_string()),
                    ),
                    instruction(1, InstructionKind::MethodCall(logger)),
                    instruction(2, InstructionKind::Load(1)),
                    instruction(3, InstructionKind::MethodCall(target_class)),
                ],
            )],
        );

        assert!(recover_targets(&class).is_empty());
    }

    #[test]
    fn target_recovery_binds_factory_arguments_without_a_recent_string_window() {
        let target_method = MemberReference {
            owner: "cpw/mods/modlauncher/api/ITransformer$Target".to_string(),
            name: "targetMethod".to_string(),
            descriptor: "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Lcpw/mods/modlauncher/api/ITransformer$Target;".to_string(),
            kind: MemberKind::Method,
            is_static: Some(true),
        };
        let class = empty_class(
            "example/Transformer",
            vec![ITRANSFORMER.to_string()],
            vec![method(
                "targets",
                vec![
                    instruction(
                        0,
                        InstructionKind::StringConstant("game.Target".to_string()),
                    ),
                    instruction(1, InstructionKind::StringConstant("tick".to_string())),
                    instruction(2, InstructionKind::StringConstant("()V".to_string())),
                    instruction(3, InstructionKind::MethodCall(target_method)),
                ],
            )],
        );

        let targets = recover_targets(&class);

        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].target,
            Target::method("game/Target", "tick", "()V")
        );
    }

    #[test]
    fn recovered_target_with_unknown_effect_degrades_without_fabricating_precision() {
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: Vec::new(),
            refmaps: Vec::new(),
            resources: Vec::new(),
        };
        let transformer = empty_class("example/Transformer", Vec::new(), Vec::new());
        let target = RecoveredTarget {
            target: Target::method("game/Foo", "tick", "()V"),
            detail: "ITransformer.Target.targetMethod".to_string(),
        };

        let effect = unknown_effect(
            &artifact,
            &transformer,
            target,
            Mechanism::ModLauncherTransformer,
            "unsupported test write",
        );

        assert_eq!(effect.precision, Precision::Method);
        assert_eq!(effect.confidence, Confidence::Low);
        assert_eq!(effect.mutations[0].kind, MutationKind::UnknownMethod);
        assert_eq!(effect.requirements[0].kind, RequirementKind::MemberExists);
    }

    #[test]
    fn asm_tree_mutations_are_classified() {
        for (name, expected) in [
            ("add", MutationKind::InsertInstructions),
            ("insert", MutationKind::InsertInstructions),
            ("insertBefore", MutationKind::InsertInstructions),
            ("remove", MutationKind::RemoveInstruction),
            ("set", MutationKind::ReplaceInstruction),
            ("clear", MutationKind::ReplaceMethodBody),
        ] {
            let call = MemberReference {
                owner: "org/objectweb/asm/tree/InsnList".to_string(),
                name: name.to_string(),
                descriptor: "(Lorg/objectweb/asm/tree/AbstractInsnNode;)V".to_string(),
                kind: MemberKind::Method,
                is_static: Some(false),
            };
            let (kind, _) = classify_call(&call, &VecDeque::new()).unwrap();
            assert_eq!(kind, expected);
        }
    }

    #[test]
    fn iterator_remove_without_collection_provenance_is_not_an_instruction_removal() {
        let call = MemberReference {
            owner: "java/util/Iterator".to_string(),
            name: "remove".to_string(),
            descriptor: "()V".to_string(),
            kind: MemberKind::Method,
            is_static: Some(false),
        };

        let (kind, _) = classify_call(&call, &VecDeque::new()).unwrap();

        assert_eq!(kind, MutationKind::UnknownMethod);
    }

    #[test]
    fn heuristic_transformer_pattern_never_claims_exact_instruction_precision() {
        let called = member("game/Owner", "run", "()V");
        let target_class = empty_class(
            "game/Foo",
            Vec::new(),
            vec![method(
                "tick",
                vec![instruction(0, InstructionKind::MethodCall(called.clone()))],
            )],
        );
        let runtime = ParsedArtifact {
            id: "minecraft".to_string(),
            display_name: "minecraft".to_string(),
            kind: ArtifactKind::Minecraft,
            classes: vec![target_class],
            refmaps: Vec::new(),
            resources: Vec::new(),
        };
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: Vec::new(),
            refmaps: Vec::new(),
            resources: Vec::new(),
        };
        let scanned = scanned(vec![runtime, artifact], ClassUniverse::default());
        let recovered_target = RecoveredTarget {
            target: Target::method("game/Foo", "tick", "()V"),
            detail: "heuristic target factory".to_string(),
        };
        let signal = MutationSignal {
            kind: MutationKind::RemoveInstruction,
            source_class: "example/Transformer".to_string(),
            source_method: "transform()V".to_string(),
            source_instruction: InstructionReference {
                identity: None,
                stable_id: 5,
                original_offset: Some(5),
                opcode: 182,
                local_slot: None,
                member: None,
                constant: None,
            },
            pattern: Some(InstructionPattern {
                member: Some(called),
                constant: None,
                integer: None,
                opcode: Some(182),
                detail: "recent constructor heuristic".to_string(),
            }),
            detail: "heuristic ASM call".to_string(),
        };

        let effects = effects_for_signal(
            &scanned,
            &scanned.artifacts[1],
            &recovered_target,
            &signal,
            Mechanism::ModLauncherTransformer,
        );

        assert!(!effects.is_empty());
        assert!(
            effects
                .iter()
                .all(|effect| effect.precision == Precision::Pattern)
        );
        assert!(
            effects
                .iter()
                .all(|effect| effect.confidence == Confidence::Low)
        );
        assert!(
            effects
                .iter()
                .flat_map(|effect| &effect.mutations)
                .all(|mutation| mutation.precision == Precision::Pattern)
        );
    }

    #[test]
    fn method_instruction_constructor_recovers_pattern() {
        let call = MemberReference {
            owner: "org/objectweb/asm/tree/MethodInsnNode".to_string(),
            name: "<init>".to_string(),
            descriptor: String::new(),
            kind: MemberKind::Method,
            is_static: Some(false),
        };
        let strings = VecDeque::from([
            "net/minecraft/client/Minecraft".to_string(),
            "runTick".to_string(),
            "()V".to_string(),
        ]);
        let pattern = pattern_from_constructor(&call, &strings, &VecDeque::new()).unwrap();
        let member = pattern.member.unwrap();
        assert_eq!(member.owner, "net/minecraft/client/Minecraft");
        assert_eq!(member.name, "runTick");
        assert_eq!(member.descriptor, "()V");
    }

    #[test]
    fn unregistered_itransformer_is_only_a_candidate() {
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: vec![empty_class(
                "example/Service$1",
                vec![ITRANSFORMER.to_string()],
                Vec::new(),
            )],
            refmaps: Vec::new(),
            resources: Vec::new(),
        };
        let mut universe = ClassUniverse::default();
        universe.classes.insert(
            "example/Service$1".to_string(),
            vec![definition(
                "example/Service$1",
                vec![ITRANSFORMER.to_string()],
            )],
        );
        let scanned = scanned(vec![artifact], universe);

        let candidates = discover_transformer_candidates(&scanned, &scanned.artifacts[0]);
        let registered = discover_registered_transformer_classes(&scanned, &scanned.artifacts[0]);

        assert_eq!(
            candidates,
            BTreeSet::from(["example/Service$1".to_string()])
        );
        assert!(registered.is_empty());
    }

    #[test]
    fn service_loaded_transformation_service_registers_only_returned_transformers() {
        let service = empty_class(
            "example/Service",
            vec![TRANSFORMATION_SERVICE.to_string()],
            vec![method(
                "transformers",
                vec![instruction(
                    0,
                    InstructionKind::Type("example/Transformer".to_string()),
                )],
            )],
        );
        let transformer = empty_class(
            "example/Transformer",
            vec![ITRANSFORMER.to_string()],
            Vec::new(),
        );
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: vec![service, transformer],
            refmaps: Vec::new(),
            resources: vec![ResourceEntry {
                path: "META-INF/services/cpw.mods.modlauncher.api.ITransformationService"
                    .to_string(),
                bytes: b"# registered by ServiceLoader\nexample.Service\n".to_vec(),
            }],
        };
        let mut universe = ClassUniverse::default();
        universe.classes.insert(
            "example/Service".to_string(),
            vec![definition(
                "example/Service",
                vec![TRANSFORMATION_SERVICE.to_string()],
            )],
        );
        universe.classes.insert(
            "example/Transformer".to_string(),
            vec![definition(
                "example/Transformer",
                vec![ITRANSFORMER.to_string()],
            )],
        );
        let scanned = scanned(vec![artifact], universe);

        assert_eq!(
            discover_registered_transformer_classes(&scanned, &scanned.artifacts[0]),
            BTreeSet::from(["example/Transformer".to_string()])
        );
    }

    #[test]
    fn multiple_targets_and_mutations_do_not_form_a_cartesian_product() {
        let service = empty_class(
            "example/Service",
            vec![TRANSFORMATION_SERVICE.to_string()],
            vec![method(
                "transformers",
                vec![instruction(
                    0,
                    InstructionKind::Type("example/Transformer".to_string()),
                )],
            )],
        );
        let target_class = MemberReference {
            owner: "cpw/mods/modlauncher/api/ITransformer$Target".to_string(),
            name: "targetClass".to_string(),
            descriptor: "(Ljava/lang/String;)Lcpw/mods/modlauncher/api/ITransformer$Target;"
                .to_string(),
            kind: MemberKind::Method,
            is_static: Some(true),
        };
        let remove = member(
            "org/objectweb/asm/tree/InsnList",
            "remove",
            "(Lorg/objectweb/asm/tree/AbstractInsnNode;)V",
        );
        let transformer = empty_class(
            "example/Transformer",
            vec![ITRANSFORMER.to_string()],
            vec![
                method(
                    "targets",
                    vec![
                        instruction(0, InstructionKind::StringConstant("game.First".to_string())),
                        instruction(1, InstructionKind::MethodCall(target_class.clone())),
                        instruction(
                            2,
                            InstructionKind::StringConstant("game.Second".to_string()),
                        ),
                        instruction(3, InstructionKind::MethodCall(target_class)),
                    ],
                ),
                method(
                    "transform",
                    vec![
                        instruction(0, InstructionKind::MethodCall(remove.clone())),
                        instruction(1, InstructionKind::MethodCall(remove)),
                    ],
                ),
            ],
        );
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: vec![service, transformer],
            refmaps: Vec::new(),
            resources: vec![ResourceEntry {
                path: "META-INF/services/cpw.mods.modlauncher.api.ITransformationService"
                    .to_string(),
                bytes: b"example.Service\n".to_vec(),
            }],
        };
        let mut universe = ClassUniverse::default();
        universe.classes.insert(
            "example/Service".to_string(),
            vec![definition(
                "example/Service",
                vec![TRANSFORMATION_SERVICE.to_string()],
            )],
        );
        universe.classes.insert(
            "example/Transformer".to_string(),
            vec![definition(
                "example/Transformer",
                vec![ITRANSFORMER.to_string()],
            )],
        );
        let mut scanned = scanned(vec![artifact], universe);
        let readiness = Readiness {
            status: crate::model::ReadinessStatus::Ready,
            loader: Some(LoaderFamily::Forge),
            message: "ready".to_string(),
            capabilities: Vec::new(),
        };

        let analysis = analyze_with_progress(&mut scanned, &readiness, None);

        assert_eq!(analysis.effects.len(), 2);
        assert!(analysis.effects.iter().all(|effect| {
            effect.precision == Precision::Class
                && effect.mutations[0].kind == MutationKind::UnknownClass
        }));
        assert_eq!(analysis.coverage_gaps.len(), 1);
        assert_eq!(
            analysis.coverage_gaps[0].kind,
            crate::model::CoverageGapKind::TransformerPartial
        );
        assert!(scanned.warnings.is_empty());
    }

    #[test]
    fn lambda_helper_is_followed_but_untainted_temporary_asm_is_ignored() {
        let remove = member(
            "org/objectweb/asm/tree/InsnList",
            "remove",
            "(Lorg/objectweb/asm/tree/AbstractInsnNode;)V",
        );
        let lambda = member(
            "example/Transformer",
            "lambda$transform$0",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
        );
        let temporary = member(
            "example/Transformer",
            "temporary",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
        );
        let mut transform_instructions = vec![instruction(
            0,
            InstructionKind::InvokeDynamic {
                name: "accept".to_string(),
                descriptor: "()Ljava/util/function/Consumer;".to_string(),
                implementation: Some(lambda),
            },
        )];
        transform_instructions.extend((1..=13).map(|id| instruction(id, InstructionKind::Other)));
        transform_instructions.push(instruction(14, InstructionKind::MethodCall(temporary)));
        let transformer = empty_class(
            "example/Transformer",
            vec![ITRANSFORMER.to_string()],
            vec![
                method("transform", transform_instructions),
                method(
                    "lambda$transform$0",
                    vec![
                        instruction(0, InstructionKind::Load(1)),
                        instruction(1, InstructionKind::MethodCall(remove.clone())),
                    ],
                ),
                method(
                    "temporary",
                    vec![instruction(0, InstructionKind::MethodCall(remove))],
                ),
            ],
        );
        let artifact = ParsedArtifact {
            id: "mod".to_string(),
            display_name: "mod".to_string(),
            kind: ArtifactKind::Mod,
            classes: vec![transformer.clone()],
            refmaps: Vec::new(),
            resources: Vec::new(),
        };
        let scanned = scanned(vec![artifact], ClassUniverse::default());
        let (signals, exhausted, ignored) = recover_mutations(
            &scanned,
            &scanned.artifacts[0],
            &transformer,
            &mut Vec::new(),
        );

        assert!(!exhausted);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].kind, MutationKind::RemoveInstruction);
        assert_eq!(ignored, 1);
    }

    fn scanned(artifacts: Vec<ParsedArtifact>, universe: ClassUniverse) -> ScannedArtifacts {
        ScannedArtifacts {
            artifact_reports: Vec::new(),
            artifacts,
            universe,
            limits: AnalysisLimits::default(),
            coverage: Coverage::default(),
            warnings: Vec::new(),
        }
    }

    fn empty_class(name: &str, interfaces: Vec<String>, methods: Vec<ParsedMethod>) -> ParsedClass {
        ParsedClass {
            definition_id: None,
            future_version_best_effort: false,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces,
            annotations: Vec::new(),
            fields: Vec::new(),
            methods,
        }
    }

    fn method(name: &str, instructions: Vec<ParsedInstruction>) -> ParsedMethod {
        ParsedMethod {
            name: name.to_string(),
            descriptor: "(Ljava/lang/Object;)Ljava/lang/Object;".to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: Vec::new(),
            max_locals: Some(2),
            instructions,
        }
    }

    fn instruction(id: u32, kind: InstructionKind) -> ParsedInstruction {
        ParsedInstruction {
            reference: InstructionReference {
                identity: None,
                stable_id: id,
                original_offset: Some(id),
                opcode: 0,
                local_slot: None,
                member: None,
                constant: None,
            },
            kind,
        }
    }

    fn member(owner: &str, name: &str, descriptor: &str) -> MemberReference {
        MemberReference {
            owner: owner.to_string(),
            name: name.to_string(),
            descriptor: descriptor.to_string(),
            kind: MemberKind::Method,
            is_static: Some(false),
        }
    }

    fn definition(name: &str, interfaces: Vec<String>) -> ClassDefinition {
        ClassDefinition {
            definition_id: None,
            artifact_id: "mod".to_string(),
            is_mod: true,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces,
            fields: Vec::new(),
            methods: Vec::new(),
            hard_references: Vec::new(),
        }
    }
}
