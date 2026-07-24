use std::collections::{BTreeSet, HashSet, VecDeque};

use crate::classfile::{InstructionKind, ParsedClass, ParsedInstruction, ParsedMethod};
use crate::jar::{ParsedArtifact, ScannedArtifacts};
use crate::model::{
    Activation, Confidence, Effect, Evidence, LoaderFamily, Mechanism, MemberKind, MemberReference,
    Mutation, MutationKind, Precision, Readiness, RequirementKind, ShapeRequirement, Target,
    Warning,
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
    exclusive: bool,
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

pub(crate) fn analyze(scanned: &mut ScannedArtifacts, readiness: &Readiness) -> Vec<Effect> {
    if !matches!(
        readiness.loader,
        Some(LoaderFamily::Forge | LoaderFamily::NeoForge)
    ) {
        return Vec::new();
    }

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

    for artifact_index in mod_indexes {
        let artifact = &scanned.artifacts[artifact_index];
        let transformer_classes = discover_transformer_classes(scanned, artifact);
        for class_name in transformer_classes {
            let Some(class) = artifact
                .classes
                .iter()
                .find(|class| class.name == class_name)
            else {
                continue;
            };
            scanned.coverage.transformers_discovered += 1;
            let mechanism = transformer_mechanism(scanned, class);
            let targets = recover_targets(class);
            if targets.is_empty() {
                scanned.coverage.transformer_effects_unknown += 1;
                warnings.push(Warning {
                    artifact_id: Some(artifact.id.clone()),
                    scope: class.name.clone(),
                    message: "ITransformer target set is dynamic or could not be recovered; \
                              no global pairwise conflicts were fabricated"
                        .to_string(),
                });
                continue;
            }
            scanned.coverage.transformer_targets_recovered += targets.len();

            let (signals, exhausted, ignored_untainted) =
                recover_mutations(scanned, artifact, class, &mut warnings);
            if ignored_untainted > 0 {
                scanned.coverage.transformer_effects_partial += 1;
                warnings.push(Warning {
                    artifact_id: Some(artifact.id.clone()),
                    scope: class.name.clone(),
                    message: format!(
                        "ignored {ignored_untainted} ASM-looking write(s) whose receiver \
                         could not be traced to the transform() input"
                    ),
                });
            }
            if exhausted {
                scanned.coverage.budget_exhaustions.push(format!(
                    "{}:{} bounded transformer interpretation",
                    artifact.id, class.name
                ));
            }
            if signals.is_empty() {
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
                continue;
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
        }
    }

    scanned.warnings.extend(warnings);
    effects
}

fn discover_transformer_classes(
    scanned: &ScannedArtifacts,
    artifact: &ParsedArtifact,
) -> BTreeSet<String> {
    let mut result = artifact
        .classes
        .iter()
        .filter(|class| inherits(scanned, &class.name, ITRANSFORMER, &mut HashSet::new()))
        .map(|class| class.name.clone())
        .collect::<BTreeSet<_>>();

    // ITransformationService#transformers commonly constructs anonymous or
    // nested transformer classes. Inspecting those factories also catches
    // implementations reached through a static helper or invokedynamic.
    for service in artifact.classes.iter().filter(|class| {
        inherits(
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
        let mut strings = VecDeque::<String>::new();
        for instruction in &method.instructions {
            match &instruction.kind {
                InstructionKind::StringConstant(value) => {
                    strings.push_back(value.clone());
                    if strings.len() > 8 {
                        strings.pop_front();
                    }
                }
                InstructionKind::MethodCall(member)
                    if member.owner.ends_with("/ITransformer$Target")
                        || member.owner == "cpw/mods/modlauncher/api/ITransformer$Target" =>
                {
                    if let Some(target) = target_from_factory(&member.name, &strings) {
                        targets.push(RecoveredTarget {
                            detail: format!(
                                "recovered from {}.{}{} call {}",
                                class.name, method.name, method.descriptor, member.name
                            ),
                            target,
                        });
                    }
                    strings.clear();
                }
                _ => {}
            }
        }
    }
    targets.sort_by_key(|target| target_key(&target.target));
    targets.dedup_by(|left, right| left.target == right.target);
    targets
}

fn target_from_factory(name: &str, strings: &VecDeque<String>) -> Option<Target> {
    match name {
        "targetClass" | "targetPreClass" => {
            let class = normalize_class(strings.back()?)?;
            Some(Target::class(class))
        }
        "targetMethod" => {
            let values = last_strings(strings, 3)?;
            let owner = normalize_class(&values[0])?;
            Some(Target::method(owner, values[1].clone(), values[2].clone()))
        }
        "targetField" => {
            let values = last_strings(strings, 2)?;
            let owner = normalize_class(&values[0])?;
            Some(Target {
                class: owner.clone(),
                member: Some(MemberReference {
                    owner,
                    name: values[1].clone(),
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
    warnings: &mut Vec<Warning>,
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
                        warnings.push(Warning {
                            artifact_id: Some(artifact.id.clone()),
                            scope: format!("{}.{}{}", owner.name, method.name, method.descriptor),
                            message: format!(
                                "invokedynamic {name}{descriptor} has no recoverable implementation handle"
                            ),
                        });
                    }
                }
                InstructionKind::MethodCall(member) => {
                    if let Some(pattern) =
                        pattern_from_constructor(member, &recent_strings, &recent_integers)
                    {
                        push_bounded(&mut patterns, pattern, 8);
                    }
                    if let Some((kind, exclusive, description)) =
                        classify_call(member, &recent_types)
                    {
                        if taint_window > 0 {
                            signals.push(MutationSignal {
                                kind,
                                exclusive,
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
                                exclusive: matches!(
                                    kind,
                                    MutationKind::ChangeSuperclass
                                        | MutationKind::ChangeInterfaces
                                        | MutationKind::ReplaceMethodBody
                                ),
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
) -> Option<(MutationKind, bool, String)> {
    let owner = member.owner.as_str();
    let (kind, exclusive) = if owner.ends_with("/InsnList") {
        match member.name.as_str() {
            "add" | "insert" | "insertBefore" => (MutationKind::InsertInstructions, false),
            "remove" => (MutationKind::RemoveInstruction, true),
            "set" => (MutationKind::ReplaceInstruction, true),
            "clear" => (MutationKind::ReplaceMethodBody, true),
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
            | "visitMultiANewArrayInsn" => (MutationKind::InsertInstructions, false),
            "visitMaxs" | "visitLocalVariable" => (MutationKind::ChangeLocalLayout, false),
            _ => return None,
        }
    } else if owner.ends_with("/ClassVisitor") || owner.ends_with("/ClassNode") {
        match member.name.as_str() {
            "visitMethod" => (MutationKind::AddMethod, false),
            "visitField" => (MutationKind::AddField, false),
            "visit" => (MutationKind::ChangeSuperclass, true),
            _ => return None,
        }
    } else if owner == "java/util/List" || owner.ends_with("/ArrayList") {
        let node = recent_types.back().map(String::as_str).unwrap_or_default();
        match (member.name.as_str(), node) {
            ("add", node) if node.ends_with("/MethodNode") => (MutationKind::AddMethod, false),
            ("add", node) if node.ends_with("/FieldNode") => (MutationKind::AddField, false),
            ("remove", node) if node.ends_with("/MethodNode") => (MutationKind::RemoveMethod, true),
            ("remove", node) if node.ends_with("/FieldNode") => (MutationKind::RemoveField, true),
            _ => return None,
        }
    } else if owner == "java/util/Iterator" && member.name == "remove" {
        (MutationKind::RemoveInstruction, true)
    } else {
        return None;
    };
    Some((
        kind,
        exclusive,
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
            "superName" => MutationKind::ChangeSuperclass,
            "interfaces" => MutationKind::ChangeInterfaces,
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
                        Precision::Instruction,
                        Confidence::High,
                        vec![ShapeRequirement {
                            kind: RequirementKind::InstructionExists,
                            target: {
                                let mut required = base_target.clone();
                                required.instruction = Some(instruction);
                                required
                            },
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
            if signal.pattern.is_some() {
                Confidence::Medium
            } else {
                Confidence::Low
            },
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
        mutations: vec![Mutation {
            kind: signal.kind,
            target,
            exclusive: signal.exclusive,
        }],
        evidence: vec![Evidence {
            artifact_id: artifact.id.clone(),
            class: signal.source_class.clone(),
            method: Some(signal.source_method.clone()),
            annotation: None,
            instruction: Some(signal.source_instruction.clone()),
            detail,
        }],
        precision,
        confidence,
        activation: Activation::Candidate,
        priority: None,
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
            minimum_matches: Some(1),
            maximum_matches: None,
            ordinal: None,
            slice: None,
        }],
        mutations: vec![Mutation {
            kind: mutation_kind,
            target: target.target.clone(),
            exclusive: false,
        }],
        evidence: vec![Evidence {
            artifact_id: artifact.id.clone(),
            class: transformer.name.clone(),
            method: Some("transform".to_string()),
            annotation: None,
            instruction: None,
            detail: format!("{}; {reason}", target.detail),
        }],
        precision: if target.target.member.is_some() {
            Precision::Method
        } else {
            Precision::Class
        },
        confidence: Confidence::Low,
        activation: Activation::Candidate,
        priority: None,
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
    use crate::jar::{ClassDefinition, ClassUniverse, ParsedArtifact};
    use crate::model::{AnalysisLimits, ArtifactKind, Coverage, InstructionReference};

    use super::*;

    #[test]
    fn target_factories_recover_class_method_and_field() {
        let mut strings = VecDeque::from([
            "net.minecraft.client.Minecraft".to_string(),
            "runTick".to_string(),
            "()V".to_string(),
        ]);
        assert_eq!(
            target_from_factory("targetMethod", &strings)
                .unwrap()
                .member
                .unwrap()
                .name,
            "runTick"
        );
        strings = VecDeque::from([
            "net.minecraft.client.Minecraft".to_string(),
            "level".to_string(),
        ]);
        assert_eq!(
            target_from_factory("targetField", &strings)
                .unwrap()
                .member
                .unwrap()
                .name,
            "level"
        );
        strings = VecDeque::from(["net.minecraft.client.Minecraft".to_string()]);
        assert_eq!(
            target_from_factory("targetClass", &strings).unwrap().class,
            "net/minecraft/client/Minecraft"
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
        for (name, expected, exclusive) in [
            ("add", MutationKind::InsertInstructions, false),
            ("insert", MutationKind::InsertInstructions, false),
            ("insertBefore", MutationKind::InsertInstructions, false),
            ("remove", MutationKind::RemoveInstruction, true),
            ("set", MutationKind::ReplaceInstruction, true),
            ("clear", MutationKind::ReplaceMethodBody, true),
        ] {
            let call = MemberReference {
                owner: "org/objectweb/asm/tree/InsnList".to_string(),
                name: name.to_string(),
                descriptor: "(Lorg/objectweb/asm/tree/AbstractInsnNode;)V".to_string(),
                kind: MemberKind::Method,
                is_static: Some(false),
            };
            let (kind, is_exclusive, _) = classify_call(&call, &VecDeque::new()).unwrap();
            assert_eq!(kind, expected);
            assert_eq!(is_exclusive, exclusive);
        }
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
    fn anonymous_itransformer_is_discovered_by_actual_hierarchy() {
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

        let discovered = discover_transformer_classes(&scanned, &scanned.artifacts[0]);

        assert_eq!(
            discovered,
            BTreeSet::from(["example/Service$1".to_string()])
        );
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
            minor: 0,
            major: 61,
            future_version_best_effort: false,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces,
            is_interface: false,
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
                stable_id: id,
                original_offset: Some(id),
                opcode: 0,
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
            artifact_id: "mod".to_string(),
            is_mod: true,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces,
            is_interface: false,
            fields: Vec::new(),
            methods: Vec::new(),
            hard_references: Vec::new(),
        }
    }
}
