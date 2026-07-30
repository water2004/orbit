use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::classfile::{
    AnnotationValue, InstructionKind, ParsedAnnotation, ParsedClass, ParsedField, ParsedMethod,
};
use crate::jar::{ParsedArtifact, ScannedArtifacts};
use crate::model::{
    ArtifactKind, ArtifactSymbolSpace, ClassVisibility, Confidence, CoverageRatio,
    LoaderArtifactUnit, LoaderFamily, MappingSource, NamespaceAlignment, NamespaceEvidence,
    NamespaceReport, Readiness, ReadinessStatus, SymbolMappingEvidence, SymbolNamespace,
};

const MIXIN_ANNOTATION: &str = "Lorg/spongepowered/asm/mixin/Mixin;";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MappingMemberKind {
    Field,
    Method,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MemberKey {
    owner: String,
    name: String,
    descriptor: String,
}

#[derive(Debug, Clone)]
struct NamespaceMapping {
    source: String,
    target: String,
    source_id: String,
    resource_path: String,
    classes: BTreeMap<String, String>,
    fields: BTreeMap<MemberKey, String>,
    methods: BTreeMap<MemberKey, String>,
}

#[derive(Debug, Clone)]
struct MappingTree {
    source_id: String,
    artifact_id: String,
    resource_path: String,
    sha256: String,
    namespaces: Vec<String>,
    classes: Vec<Vec<String>>,
    fields: Vec<(Vec<String>, String, Vec<String>)>,
    methods: Vec<(Vec<String>, String, Vec<String>)>,
}

impl MappingTree {
    fn class_count(&self) -> usize {
        self.classes.len()
    }

    fn mapping(&self, source: &str, target: &str) -> Option<NamespaceMapping> {
        let source_index = self.namespaces.iter().position(|value| value == source)?;
        let target_index = self.namespaces.iter().position(|value| value == target)?;
        let mut classes = BTreeMap::new();
        for names in &self.classes {
            let Some(source_name) = names.get(source_index).filter(|name| !name.is_empty()) else {
                continue;
            };
            let target_name = names
                .get(target_index)
                .filter(|name| !name.is_empty())
                .unwrap_or(source_name);
            classes.insert(source_name.clone(), target_name.clone());
        }
        let members = |entries: &[(Vec<String>, String, Vec<String>)]| {
            entries
                .iter()
                .filter_map(|(owners, descriptor, names)| {
                    let owner = if owners.len() == self.namespaces.len() {
                        owners.get(source_index)?.clone()
                    } else {
                        let source_owner = owners.first()?;
                        self.classes
                            .iter()
                            .find(|class_names| class_names.first() == Some(source_owner))
                            .and_then(|class_names| class_names.get(source_index))
                            .cloned()
                            .filter(|owner| !owner.is_empty())
                            .or_else(|| (source_index == 0).then(|| source_owner.clone()))?
                    };
                    let name = names.get(source_index)?.clone();
                    if owner.is_empty() || name.is_empty() {
                        return None;
                    }
                    let descriptor =
                        map_mapping_descriptor(descriptor, &self.classes, source_index);
                    let mapped = names
                        .get(target_index)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(&name)
                        .clone();
                    Some((
                        MemberKey {
                            owner,
                            name,
                            descriptor,
                        },
                        mapped,
                    ))
                })
                .collect::<BTreeMap<_, _>>()
        };
        Some(NamespaceMapping {
            source: source.to_string(),
            target: target.to_string(),
            source_id: self.source_id.clone(),
            resource_path: self.resource_path.clone(),
            classes,
            fields: members(&self.fields),
            methods: members(&self.methods),
        })
    }

    fn report(&self) -> MappingSource {
        MappingSource {
            id: self.source_id.clone(),
            artifact_id: self.artifact_id.clone(),
            resource_path: self.resource_path.clone(),
            sha256: self.sha256.clone(),
            namespaces: self.namespaces.clone(),
        }
    }
}

pub(crate) fn align_fabric_runtime(
    scanned: &mut ScannedArtifacts,
) -> Result<NamespaceReport, Readiness> {
    align_mapping_runtime(
        scanned,
        LoaderFamily::Fabric,
        "the Fabric mapping configuration contains no effective class mappings; the runtime therefore retains the supplied game symbol space",
    )
}

pub(crate) fn align_quilt_runtime(
    scanned: &mut ScannedArtifacts,
) -> Result<NamespaceReport, Readiness> {
    align_mapping_runtime(
        scanned,
        LoaderFamily::Quilt,
        "the Quilt mapping configuration contains no effective class mappings; the runtime therefore retains the supplied game symbol space",
    )
}

fn align_mapping_runtime(
    scanned: &mut ScannedArtifacts,
    loader: LoaderFamily,
    identity_detail: &str,
) -> Result<NamespaceReport, Readiness> {
    let mapping_trees = discover_tiny_mappings(&scanned.artifacts);
    let mapping_sources = mapping_trees.iter().map(MappingTree::report).collect();
    align_tiny_runtime(
        scanned,
        loader,
        mapping_trees,
        mapping_sources,
        identity_detail,
    )
}

pub(crate) fn align_modlauncher_runtime(
    scanned: &mut ScannedArtifacts,
    loader: LoaderFamily,
) -> Result<NamespaceReport, Readiness> {
    let mapping_sources = discover_tiny_mappings(&scanned.artifacts)
        .iter()
        .map(MappingTree::report)
        .collect();
    align_modlauncher_family(scanned, loader, mapping_sources)
}

fn align_tiny_runtime(
    scanned: &mut ScannedArtifacts,
    loader: LoaderFamily,
    trees: Vec<MappingTree>,
    mapping_sources: Vec<MappingSource>,
    identity_without_mappings: &str,
) -> Result<NamespaceReport, Readiness> {
    let usable = trees
        .iter()
        .filter(|tree| {
            tree.class_count() > 0 && tree.namespaces.iter().any(|value| value == "intermediary")
        })
        .collect::<Vec<_>>();
    if usable.is_empty() {
        ensure_identity_symbol_space(scanned, loader)?;
        let report = identity_report(
            scanned,
            SymbolNamespace::Official,
            mapping_sources,
            identity_without_mappings,
        );
        scanned.coverage.nested_artifact_units = scanned.artifacts.len();
        return Ok(report);
    }

    let minecraft_names = minecraft_class_names(scanned);
    let mut namespace_matches = BTreeMap::<String, usize>::new();
    for tree in &usable {
        for (index, namespace) in tree.namespaces.iter().enumerate() {
            let matches = tree
                .classes
                .iter()
                .filter(|names| {
                    names
                        .get(index)
                        .is_some_and(|name| minecraft_names.contains(name))
                })
                .count();
            if matches > 0 {
                namespace_matches
                    .entry(namespace.clone())
                    .and_modify(|current| *current = (*current).max(matches))
                    .or_insert(matches);
            }
        }
    }
    let maximum = namespace_matches
        .values()
        .copied()
        .max()
        .unwrap_or_default();
    let candidates = namespace_matches
        .into_iter()
        .filter_map(|(namespace, count)| (count == maximum).then_some(namespace))
        .collect::<BTreeSet<_>>();
    if candidates.len() != 1 {
        scanned.coverage.namespace_alignment_failures += 1;
        if candidates.len() > 1 {
            scanned.coverage.namespace_ambiguous_artifacts += 1;
        }
        let namespaces = candidates
            .iter()
            .map(|candidate| namespace_kind(candidate))
            .collect::<Vec<_>>();
        return Err(namespace_not_ready(
            loader,
            if candidates.is_empty() {
                ReadinessStatus::Incomplete
            } else {
                ReadinessStatus::Ambiguous
            },
            if candidates.is_empty() {
                "Bytecode audit could not establish the loader runtime namespace. The selected mapping resources do not describe the supplied Minecraft JAR."
                    .to_string()
            } else {
                format!(
                    "Bytecode audit could not establish the loader runtime namespace. The Minecraft JAR simultaneously matches mapping namespaces: {}.",
                    candidates.into_iter().collect::<Vec<_>>().join(", ")
                )
            },
            namespaces,
        ));
    }
    let source = candidates.into_iter().next().expect("one candidate above");
    let target = "intermediary";
    if source == target {
        let mut report = identity_report(
            scanned,
            SymbolNamespace::Intermediary,
            mapping_sources,
            "the supplied Minecraft classes already match the Loader intermediary runtime namespace",
        );
        for artifact in &mut report.artifacts {
            if artifact.artifact_id == minecraft_artifact_id(scanned).unwrap_or_default() {
                artifact.namespace = SymbolNamespace::Intermediary;
            }
        }
        scanned.coverage.nested_artifact_units = scanned.artifacts.len();
        return Ok(report);
    }

    let mappings = usable
        .into_iter()
        .filter_map(|tree| tree.mapping(&source, target))
        .collect::<Vec<_>>();
    let mut mapping = merge_mappings(&mappings).map_err(|reason| {
        scanned.coverage.namespace_alignment_failures += 1;
        scanned.coverage.namespace_ambiguous_artifacts += 1;
        namespace_not_ready(
            loader,
            ReadinessStatus::Ambiguous,
            format!("Bytecode audit could not establish the loader runtime namespace. {reason}"),
            vec![namespace_kind(&source), SymbolNamespace::Intermediary],
        )
    })?;

    let mut class_coverage = CoverageRatio::default();
    let mut method_coverage = CoverageRatio::default();
    let mut field_coverage = CoverageRatio::default();
    let minecraft_id = minecraft_artifact_id(scanned)
        .unwrap_or_default()
        .to_string();
    let game_kind = effective_game_kind(scanned);
    for artifact in scanned
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == game_kind)
    {
        mapping = expand_hierarchy_mappings(artifact, &mapping);
    }
    for artifact in &mut scanned.artifacts {
        if artifact.kind != game_kind {
            continue;
        }
        project_artifact(
            artifact,
            &mapping,
            &mut class_coverage,
            &mut method_coverage,
            &mut field_coverage,
            &mut scanned.symbol_mappings,
        );
    }
    scanned.coverage.classes_mapped += class_coverage.mapped;
    scanned.coverage.classes_mapping_missing +=
        class_coverage.total.saturating_sub(class_coverage.mapped);
    scanned.coverage.methods_mapped += method_coverage.mapped;
    scanned.coverage.methods_mapping_missing +=
        method_coverage.total.saturating_sub(method_coverage.mapped);
    scanned.coverage.fields_mapped += field_coverage.mapped;
    scanned.coverage.fields_mapping_missing +=
        field_coverage.total.saturating_sub(field_coverage.mapped);
    scanned.coverage.nested_artifact_units = scanned.artifacts.len();
    crate::jar::rebuild_universe(scanned);

    let mut artifacts = artifact_spaces(scanned, SymbolNamespace::Intermediary);
    for artifact in &mut artifacts {
        if artifact.artifact_id == minecraft_id {
            artifact.namespace = namespace_kind(&source);
            artifact.mapping_source = Some(mapping.source_id.clone());
            artifact.confidence = Confidence::Exact;
        }
    }
    Ok(NamespaceReport {
        runtime_namespace: Some(SymbolNamespace::Intermediary),
        artifacts,
        mapping_sources,
        loader_units: loader_units(scanned),
        alignment: NamespaceAlignment::Aligned {
            runtime_namespace: SymbolNamespace::Intermediary,
        },
        class_mapping_coverage: class_coverage,
        method_mapping_coverage: method_coverage,
        field_mapping_coverage: field_coverage,
        evidence: vec![NamespaceEvidence {
            artifact_id: minecraft_id,
            resource_path: Some(mapping.resource_path.clone()),
            detail: format!(
                "Minecraft symbols matched namespace '{}' and were projected to the Loader runtime namespace '{}'",
                mapping.source, mapping.target
            ),
        }],
    })
}

fn expand_hierarchy_mappings(
    artifact: &ParsedArtifact,
    mapping: &NamespaceMapping,
) -> NamespaceMapping {
    let mut expanded = mapping.clone();
    let classes = artifact
        .classes
        .iter()
        .map(|class| (class.name.as_str(), class))
        .collect::<BTreeMap<_, _>>();
    let method_mappings = member_mappings_by_owner(&mapping.methods);
    let field_mappings = member_mappings_by_owner(&mapping.fields);

    for class in &artifact.classes {
        let mut ancestors = BTreeSet::new();
        collect_ancestor_names(&classes, &class.name, &mut BTreeSet::new(), &mut ancestors);
        let declared_fields = class
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.descriptor.as_str()))
            .collect::<BTreeSet<_>>();
        let declared_static_methods = class
            .methods
            .iter()
            .filter(|method| method.is_static)
            .map(|method| (method.name.as_str(), method.descriptor.as_str()))
            .collect::<BTreeSet<_>>();

        let mut inherited_methods = BTreeMap::<(String, String), BTreeSet<String>>::new();
        let mut inherited_fields = BTreeMap::<(String, String), BTreeSet<String>>::new();
        for ancestor in ancestors {
            for (name, descriptor, runtime_name) in
                method_mappings.get(&ancestor).into_iter().flatten()
            {
                inherited_methods
                    .entry((name.clone(), descriptor.clone()))
                    .or_default()
                    .insert(runtime_name.clone());
            }
            for (name, descriptor, runtime_name) in
                field_mappings.get(&ancestor).into_iter().flatten()
            {
                inherited_fields
                    .entry((name.clone(), descriptor.clone()))
                    .or_default()
                    .insert(runtime_name.clone());
            }
        }
        for ((name, descriptor), runtime_names) in inherited_methods {
            let key = MemberKey {
                owner: class.name.clone(),
                name: name.clone(),
                descriptor: descriptor.clone(),
            };
            if expanded.methods.contains_key(&key)
                || declared_static_methods.contains(&(name.as_str(), descriptor.as_str()))
            {
                continue;
            }
            if let Some(runtime_name) = only_value(&runtime_names) {
                expanded.methods.insert(key, runtime_name.clone());
            }
        }
        for ((name, descriptor), runtime_names) in inherited_fields {
            let key = MemberKey {
                owner: class.name.clone(),
                name: name.clone(),
                descriptor: descriptor.clone(),
            };
            if expanded.fields.contains_key(&key)
                || declared_fields.contains(&(name.as_str(), descriptor.as_str()))
            {
                continue;
            }
            if let Some(runtime_name) = only_value(&runtime_names) {
                expanded.fields.insert(key, runtime_name.clone());
            }
        }
    }
    expanded
}

fn member_mappings_by_owner(
    mappings: &BTreeMap<MemberKey, String>,
) -> BTreeMap<String, Vec<(String, String, String)>> {
    let mut by_owner = BTreeMap::<String, Vec<(String, String, String)>>::new();
    for (key, runtime_name) in mappings {
        by_owner.entry(key.owner.clone()).or_default().push((
            key.name.clone(),
            key.descriptor.clone(),
            runtime_name.clone(),
        ));
    }
    by_owner
}

fn collect_ancestor_names(
    classes: &BTreeMap<&str, &ParsedClass>,
    class_name: &str,
    visiting: &mut BTreeSet<String>,
    ancestors: &mut BTreeSet<String>,
) {
    if !visiting.insert(class_name.to_string()) {
        return;
    }
    let Some(class) = classes.get(class_name) else {
        return;
    };
    for parent in class.super_name.iter().chain(&class.interfaces) {
        if ancestors.insert(parent.clone()) {
            collect_ancestor_names(classes, parent, visiting, ancestors);
        }
    }
}

fn only_value<T: Ord>(values: &BTreeSet<T>) -> Option<&T> {
    let mut values = values.iter();
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

fn align_modlauncher_family(
    scanned: &mut ScannedArtifacts,
    loader: LoaderFamily,
    mapping_sources: Vec<MappingSource>,
) -> Result<NamespaceReport, Readiness> {
    ensure_identity_symbol_space(scanned, loader)?;
    scanned.coverage.nested_artifact_units = scanned.artifacts.len();
    Ok(identity_report(
        scanned,
        SymbolNamespace::Identity,
        mapping_sources,
        "the supplied ModLauncher runtime and transformation targets share the same observed class symbol space",
    ))
}

fn ensure_identity_symbol_space(
    scanned: &mut ScannedArtifacts,
    loader: LoaderFamily,
) -> Result<(), Readiness> {
    let game_uses_minecraft_names = minecraft_class_names(scanned)
        .iter()
        .any(|name| name.starts_with("net/minecraft/"));
    if mod_references_minecraft_names(scanned) && !game_uses_minecraft_names {
        scanned.coverage.namespace_alignment_failures += 1;
        return Err(namespace_not_ready(
            loader,
            ReadinessStatus::Incomplete,
            "Bytecode audit could not establish the loader runtime namespace. Active mod bytecode uses net/minecraft symbols, but the supplied base-game classes use a different symbol space and no effective runtime mapping was discovered."
                .to_string(),
            vec![SymbolNamespace::Official, SymbolNamespace::Intermediary],
        ));
    }
    Ok(())
}

fn mod_references_minecraft_names(scanned: &ScannedArtifacts) -> bool {
    if annotated_mixin_targets(scanned)
        .iter()
        .any(|target| target.starts_with("net/minecraft/"))
    {
        return true;
    }

    scanned
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Mod)
        .flat_map(|artifact| &artifact.classes)
        .any(|class| {
            class
                .super_name
                .iter()
                .chain(&class.interfaces)
                .any(|name| name.starts_with("net/minecraft/"))
                || class
                    .fields
                    .iter()
                    .any(|field| descriptor_references_minecraft(&field.descriptor))
                || class.methods.iter().any(|method| {
                    descriptor_references_minecraft(&method.descriptor)
                        || method
                            .instructions
                            .iter()
                            .any(|instruction| match &instruction.kind {
                                InstructionKind::MethodCall(member)
                                | InstructionKind::FieldRead(member)
                                | InstructionKind::FieldWrite(member) => {
                                    member.owner.starts_with("net/minecraft/")
                                        || descriptor_references_minecraft(&member.descriptor)
                                }
                                InstructionKind::Type(name) => name.starts_with("net/minecraft/"),
                                InstructionKind::InvokeDynamic {
                                    descriptor,
                                    implementation,
                                    ..
                                } => {
                                    descriptor_references_minecraft(descriptor)
                                        || implementation.as_ref().is_some_and(|member| {
                                            member.owner.starts_with("net/minecraft/")
                                                || descriptor_references_minecraft(
                                                    &member.descriptor,
                                                )
                                        })
                                }
                                _ => false,
                            })
                })
        })
}

fn descriptor_references_minecraft(descriptor: &str) -> bool {
    descriptor.contains("Lnet/minecraft/")
}

fn identity_report(
    scanned: &ScannedArtifacts,
    runtime_namespace: SymbolNamespace,
    mapping_sources: Vec<MappingSource>,
    detail: &str,
) -> NamespaceReport {
    NamespaceReport {
        runtime_namespace: Some(runtime_namespace),
        artifacts: artifact_spaces(scanned, runtime_namespace),
        mapping_sources,
        loader_units: loader_units(scanned),
        alignment: NamespaceAlignment::Aligned { runtime_namespace },
        class_mapping_coverage: CoverageRatio::default(),
        method_mapping_coverage: CoverageRatio::default(),
        field_mapping_coverage: CoverageRatio::default(),
        evidence: vec![NamespaceEvidence {
            artifact_id: minecraft_artifact_id(scanned)
                .unwrap_or("minecraft")
                .to_string(),
            resource_path: None,
            detail: detail.to_string(),
        }],
    }
}

fn artifact_spaces(
    scanned: &ScannedArtifacts,
    runtime_namespace: SymbolNamespace,
) -> Vec<ArtifactSymbolSpace> {
    scanned
        .artifacts
        .iter()
        .map(|artifact| ArtifactSymbolSpace {
            artifact_id: artifact.id.clone(),
            namespace: match artifact.kind {
                ArtifactKind::Minecraft | ArtifactKind::RuntimeGame | ArtifactKind::Mod => {
                    runtime_namespace
                }
                ArtifactKind::Loader | ArtifactKind::Runtime => SymbolNamespace::Runtime,
            },
            confidence: Confidence::High,
            mapping_source: None,
        })
        .collect()
}

fn loader_units(scanned: &ScannedArtifacts) -> Vec<LoaderArtifactUnit> {
    scanned
        .artifacts
        .iter()
        .map(|artifact| {
            let mut members = BTreeSet::from([artifact.id.clone()]);
            for path in artifact
                .classes
                .iter()
                .filter_map(|class| {
                    class
                        .definition_id
                        .as_ref()
                        .map(|definition| definition.entry_path.as_str())
                })
                .chain(
                    artifact
                        .resources
                        .iter()
                        .map(|resource| resource.path.as_str()),
                )
            {
                let archives = path.split("!/").collect::<Vec<_>>();
                for depth in 1..archives.len() {
                    members.insert(format!("{}!/{}", artifact.id, archives[..depth].join("!/")));
                }
            }
            LoaderArtifactUnit {
                id: artifact.id.clone(),
                root_artifact: artifact.id.clone(),
                members: members.into_iter().collect(),
                class_visibility: ClassVisibility::SharedWithinUnit,
            }
        })
        .collect()
}

fn namespace_not_ready(
    loader: LoaderFamily,
    status: ReadinessStatus,
    message: String,
    candidates: Vec<SymbolNamespace>,
) -> Readiness {
    Readiness {
        status,
        loader: Some(loader),
        message,
        capabilities: candidates
            .into_iter()
            .map(|candidate| format!("namespace_candidate:{candidate:?}").to_ascii_lowercase())
            .collect(),
    }
}

fn minecraft_artifact_id(scanned: &ScannedArtifacts) -> Option<&str> {
    scanned
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == effective_game_kind(scanned))
        .map(|artifact| artifact.id.as_str())
}

fn minecraft_class_names(scanned: &ScannedArtifacts) -> BTreeSet<String> {
    let kind = effective_game_kind(scanned);
    scanned
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == kind)
        .flat_map(|artifact| artifact.classes.iter().map(|class| class.name.clone()))
        .collect()
}

fn effective_game_kind(scanned: &ScannedArtifacts) -> ArtifactKind {
    if scanned
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == ArtifactKind::RuntimeGame)
    {
        ArtifactKind::RuntimeGame
    } else {
        ArtifactKind::Minecraft
    }
}

fn annotated_mixin_targets(scanned: &ScannedArtifacts) -> BTreeSet<String> {
    scanned
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Mod)
        .flat_map(|artifact| &artifact.classes)
        .filter_map(|class| {
            class
                .annotations
                .iter()
                .find(|annotation| annotation.descriptor == MIXIN_ANNOTATION)
        })
        .flat_map(|annotation| {
            annotation
                .value("value")
                .into_iter()
                .chain(annotation.value("targets"))
                .flat_map(AnnotationValue::strings)
        })
        .filter_map(|value| crate::mixin_config::normalize_class_name(&value))
        .collect()
}

fn namespace_kind(value: &str) -> SymbolNamespace {
    match value.to_ascii_lowercase().as_str() {
        "official" | "clientofficial" | "serverofficial" => SymbolNamespace::Official,
        "intermediary" => SymbolNamespace::Intermediary,
        "srg" | "searge" => SymbolNamespace::Srg,
        "named" => SymbolNamespace::Named,
        "runtime" => SymbolNamespace::Runtime,
        "identity" => SymbolNamespace::Identity,
        _ => SymbolNamespace::Unknown,
    }
}

fn merge_mappings(mappings: &[NamespaceMapping]) -> Result<NamespaceMapping, String> {
    let Some(first) = mappings.first() else {
        return Err(
            "no usable mapping source connects the observed and runtime namespaces".to_string(),
        );
    };
    let mut merged = first.clone();
    for mapping in &mappings[1..] {
        merge_table(&mut merged.classes, &mapping.classes, "class")?;
        merge_table(&mut merged.fields, &mapping.fields, "field")?;
        merge_table(&mut merged.methods, &mapping.methods, "method")?;
    }
    Ok(merged)
}

fn merge_table<K>(
    target: &mut BTreeMap<K, String>,
    incoming: &BTreeMap<K, String>,
    kind: &str,
) -> Result<(), String>
where
    K: Clone + Ord + std::fmt::Debug,
{
    for (key, value) in incoming {
        if let Some(existing) = target.get(key)
            && existing != value
        {
            return Err(format!(
                "multiple mapping sources disagree about {kind} symbol {key:?}"
            ));
        }
        target.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn project_artifact(
    artifact: &mut ParsedArtifact,
    mapping: &NamespaceMapping,
    class_coverage: &mut CoverageRatio,
    method_coverage: &mut CoverageRatio,
    field_coverage: &mut CoverageRatio,
    evidence: &mut BTreeMap<String, SymbolMappingEvidence>,
) {
    for class in &mut artifact.classes {
        project_class(
            class,
            mapping,
            class_coverage,
            method_coverage,
            field_coverage,
            evidence,
        );
    }
}

fn project_class(
    class: &mut ParsedClass,
    mapping: &NamespaceMapping,
    class_coverage: &mut CoverageRatio,
    method_coverage: &mut CoverageRatio,
    field_coverage: &mut CoverageRatio,
    evidence: &mut BTreeMap<String, SymbolMappingEvidence>,
) {
    let original_owner = class.name.clone();
    let runtime_owner = map_class_name(&original_owner, mapping);
    class_coverage.total += 1;
    if mapping.classes.contains_key(&original_owner) {
        class_coverage.mapped += 1;
    }
    if runtime_owner != original_owner {
        let item = SymbolMappingEvidence {
            original_symbol: original_owner.clone(),
            runtime_symbol: runtime_owner.clone(),
            mapping_source: mapping.source_id.clone(),
            confidence: Confidence::Exact,
        };
        evidence.insert(runtime_owner.clone(), item);
    }

    class.name = runtime_owner.clone();
    class.super_name = class
        .super_name
        .as_deref()
        .map(|name| map_class_name(name, mapping));
    class.interfaces = class
        .interfaces
        .iter()
        .map(|name| map_class_name(name, mapping))
        .collect();
    map_annotations(&mut class.annotations, mapping);
    for field in &mut class.fields {
        project_field(
            field,
            &original_owner,
            &runtime_owner,
            mapping,
            field_coverage,
            evidence,
        );
    }
    for method in &mut class.methods {
        project_method(
            method,
            &original_owner,
            &runtime_owner,
            mapping,
            method_coverage,
            evidence,
        );
    }
    if let Some(definition) = &mut class.definition_id {
        definition.original_name = original_owner.clone();
        definition.runtime_name = runtime_owner.clone();
    }
    for method in &mut class.methods {
        for instruction in &mut method.instructions {
            if let Some(identity) = &mut instruction.reference.identity {
                identity.definition.original_name = original_owner.clone();
                identity.definition.runtime_name = runtime_owner.clone();
                identity.method_name = method.name.clone();
                identity.method_descriptor = method.descriptor.clone();
            }
        }
    }
}

fn project_field(
    field: &mut ParsedField,
    original_owner: &str,
    runtime_owner: &str,
    mapping: &NamespaceMapping,
    coverage: &mut CoverageRatio,
    evidence: &mut BTreeMap<String, SymbolMappingEvidence>,
) {
    let original_name = field.name.clone();
    let original_descriptor = field.descriptor.clone();
    coverage.total += 1;
    let key = MemberKey {
        owner: original_owner.to_string(),
        name: original_name.clone(),
        descriptor: original_descriptor.clone(),
    };
    field.name = map_member_name(
        MappingMemberKind::Field,
        original_owner,
        &original_name,
        &original_descriptor,
        mapping,
    );
    field.descriptor = map_descriptor(&original_descriptor, mapping);
    if mapping.fields.contains_key(&key) || field.descriptor != original_descriptor {
        coverage.mapped += 1;
    }
    if field.name != original_name || field.descriptor != original_descriptor {
        evidence.insert(
            format!("{runtime_owner}::{}:{}", field.name, field.descriptor),
            SymbolMappingEvidence {
                original_symbol: format!("{original_owner}::{original_name}:{original_descriptor}"),
                runtime_symbol: format!("{runtime_owner}::{}:{}", field.name, field.descriptor),
                mapping_source: mapping.source_id.clone(),
                confidence: Confidence::Exact,
            },
        );
    }
    map_annotations(&mut field.annotations, mapping);
}

fn project_method(
    method: &mut ParsedMethod,
    original_owner: &str,
    runtime_owner: &str,
    mapping: &NamespaceMapping,
    coverage: &mut CoverageRatio,
    evidence: &mut BTreeMap<String, SymbolMappingEvidence>,
) {
    let original_name = method.name.clone();
    let original_descriptor = method.descriptor.clone();
    coverage.total += 1;
    let key = MemberKey {
        owner: original_owner.to_string(),
        name: original_name.clone(),
        descriptor: original_descriptor.clone(),
    };
    method.name = map_member_name(
        MappingMemberKind::Method,
        original_owner,
        &original_name,
        &original_descriptor,
        mapping,
    );
    method.descriptor = map_descriptor(&original_descriptor, mapping);
    if mapping.methods.contains_key(&key)
        || method.descriptor != original_descriptor
        || matches!(original_name.as_str(), "<init>" | "<clinit>")
    {
        coverage.mapped += 1;
    }
    if method.name != original_name || method.descriptor != original_descriptor {
        evidence.insert(
            format!("{runtime_owner}::{}{}", method.name, method.descriptor),
            SymbolMappingEvidence {
                original_symbol: format!("{original_owner}::{original_name}{original_descriptor}"),
                runtime_symbol: format!("{runtime_owner}::{}{}", method.name, method.descriptor),
                mapping_source: mapping.source_id.clone(),
                confidence: Confidence::Exact,
            },
        );
    }
    map_annotations(&mut method.annotations, mapping);
    for instruction in &mut method.instructions {
        match &mut instruction.kind {
            InstructionKind::MethodCall(member)
            | InstructionKind::FieldRead(member)
            | InstructionKind::FieldWrite(member) => {
                map_member_reference(member, mapping);
                instruction.reference.member = Some(member.clone());
            }
            InstructionKind::Type(name) => {
                *name = if name.starts_with('[') {
                    map_descriptor(name, mapping)
                } else {
                    map_class_name(name, mapping)
                };
            }
            InstructionKind::InvokeDynamic {
                descriptor,
                implementation,
                ..
            } => {
                *descriptor = map_descriptor(descriptor, mapping);
                if let Some(member) = implementation {
                    map_member_reference(member, mapping);
                }
            }
            _ => {}
        }
    }
}

fn map_member_reference(member: &mut crate::model::MemberReference, mapping: &NamespaceMapping) {
    let original_owner = member.owner.clone();
    let original_name = member.name.clone();
    let original_descriptor = member.descriptor.clone();
    member.name = map_member_name(
        if member.kind == crate::model::MemberKind::Field {
            MappingMemberKind::Field
        } else {
            MappingMemberKind::Method
        },
        &original_owner,
        &original_name,
        &original_descriptor,
        mapping,
    );
    member.owner = map_class_name(&original_owner, mapping);
    member.descriptor = map_descriptor(&original_descriptor, mapping);
}

fn map_member_name(
    kind: MappingMemberKind,
    owner: &str,
    name: &str,
    descriptor: &str,
    mapping: &NamespaceMapping,
) -> String {
    let table = match kind {
        MappingMemberKind::Field => &mapping.fields,
        MappingMemberKind::Method => &mapping.methods,
    };
    table
        .get(&MemberKey {
            owner: owner.to_string(),
            name: name.to_string(),
            descriptor: descriptor.to_string(),
        })
        .cloned()
        .unwrap_or_else(|| name.to_string())
}

fn map_class_name(name: &str, mapping: &NamespaceMapping) -> String {
    mapping
        .classes
        .get(name)
        .cloned()
        .unwrap_or_else(|| name.to_string())
}

fn map_descriptor(descriptor: &str, mapping: &NamespaceMapping) -> String {
    let mut output = String::with_capacity(descriptor.len());
    let mut rest = descriptor;
    while let Some(index) = rest.find('L') {
        output.push_str(&rest[..index + 1]);
        rest = &rest[index + 1..];
        let Some(end) = rest.find(';') else {
            output.push_str(rest);
            return output;
        };
        output.push_str(&map_class_name(&rest[..end], mapping));
        output.push(';');
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

fn map_annotations(annotations: &mut [ParsedAnnotation], mapping: &NamespaceMapping) {
    for annotation in annotations {
        annotation.descriptor = map_descriptor(&annotation.descriptor, mapping);
        for value in annotation.values.values_mut() {
            map_annotation_value(value, mapping);
        }
    }
}

fn map_annotation_value(value: &mut AnnotationValue, mapping: &NamespaceMapping) {
    match value {
        AnnotationValue::Class(descriptor) => {
            *descriptor = map_descriptor(descriptor, mapping);
        }
        AnnotationValue::Enum { descriptor, .. } => {
            *descriptor = map_descriptor(descriptor, mapping);
        }
        AnnotationValue::Annotation(annotation) => {
            map_annotations(std::slice::from_mut(annotation), mapping);
        }
        AnnotationValue::Array(values) => {
            for value in values {
                map_annotation_value(value, mapping);
            }
        }
        _ => {}
    }
}

fn discover_tiny_mappings(artifacts: &[ParsedArtifact]) -> Vec<MappingTree> {
    let mut seen = BTreeSet::new();
    let mut mappings = Vec::new();
    for artifact in artifacts {
        for resource in &artifact.resources {
            if !resource
                .path
                .to_ascii_lowercase()
                .ends_with("mappings/mappings.tiny")
            {
                continue;
            }
            let sha256 = format!("{:x}", Sha256::digest(&resource.bytes));
            if !seen.insert(sha256.clone()) {
                continue;
            }
            if let Some(tree) = parse_tiny(&resource.bytes, &artifact.id, &resource.path, &sha256) {
                mappings.push(tree);
            }
        }
    }
    mappings
}

fn parse_tiny(
    bytes: &[u8],
    artifact_id: &str,
    resource_path: &str,
    sha256: &str,
) -> Option<MappingTree> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.lines();
    let header = lines.next()?.trim_end_matches('\r');
    let columns = header.split('\t').collect::<Vec<_>>();
    let (version, namespaces) = if columns.first() == Some(&"v1") && columns.len() >= 3 {
        (
            1_u8,
            columns[1..]
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        )
    } else if columns.first() == Some(&"tiny") && columns.get(1) == Some(&"2") && columns.len() >= 5
    {
        (
            2_u8,
            columns[3..]
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        )
    } else {
        return None;
    };
    let mut tree = MappingTree {
        source_id: format!("{artifact_id}:{resource_path}"),
        artifact_id: artifact_id.to_string(),
        resource_path: resource_path.to_string(),
        sha256: sha256.to_string(),
        namespaces,
        classes: Vec::new(),
        fields: Vec::new(),
        methods: Vec::new(),
    };
    if version == 1 {
        parse_tiny_v1(lines, &mut tree);
    } else {
        parse_tiny_v2(lines, &mut tree);
    }
    Some(tree)
}

fn parse_tiny_v1<'a>(lines: impl Iterator<Item = &'a str>, tree: &mut MappingTree) {
    for line in lines {
        let columns = line.trim_end_matches('\r').split('\t').collect::<Vec<_>>();
        match columns.first().copied() {
            Some("CLASS") if columns.len() > tree.namespaces.len() => {
                tree.classes.push(
                    columns[1..=tree.namespaces.len()]
                        .iter()
                        .map(|value| unescape_tiny(value))
                        .collect(),
                );
            }
            Some("FIELD") | Some("METHOD") if columns.len() >= 3 + tree.namespaces.len() => {
                let owner = unescape_tiny(columns[1]);
                let descriptor = unescape_tiny(columns[2]);
                let names = columns[3..3 + tree.namespaces.len()]
                    .iter()
                    .map(|value| unescape_tiny(value))
                    .collect::<Vec<_>>();
                // Tiny v1 stores the owner and descriptor only in namespace 0.
                // MappingTree::mapping projects both when another observed
                // source namespace is selected.
                let owners = vec![owner];
                if columns[0] == "FIELD" {
                    tree.fields.push((owners, descriptor, names));
                } else {
                    tree.methods.push((owners, descriptor, names));
                }
            }
            _ => {}
        }
    }
}

fn parse_tiny_v2<'a>(lines: impl Iterator<Item = &'a str>, tree: &mut MappingTree) {
    let mut current_class = None::<Vec<String>>;
    for line in lines {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("c\t") {
            let names = rest
                .split('\t')
                .take(tree.namespaces.len())
                .map(unescape_tiny)
                .collect::<Vec<_>>();
            if names.len() == tree.namespaces.len() {
                current_class = Some(names.clone());
                tree.classes.push(names);
            }
            continue;
        }
        let Some(class_names) = current_class.as_ref() else {
            continue;
        };
        let columns = line
            .trim_start_matches('\t')
            .split('\t')
            .collect::<Vec<_>>();
        if columns.len() < 2 + tree.namespaces.len() {
            continue;
        }
        let descriptor = unescape_tiny(columns[1]);
        let names = columns[2..2 + tree.namespaces.len()]
            .iter()
            .map(|value| unescape_tiny(value))
            .collect::<Vec<_>>();
        match columns[0] {
            "f" => tree.fields.push((class_names.clone(), descriptor, names)),
            "m" => tree.methods.push((class_names.clone(), descriptor, names)),
            _ => {}
        }
    }
}

fn map_mapping_descriptor(
    descriptor: &str,
    classes: &[Vec<String>],
    target_index: usize,
) -> String {
    if target_index == 0 {
        return descriptor.to_string();
    }
    let class_names = classes
        .iter()
        .filter_map(|names| {
            let source = names.first()?.clone();
            let target = names.get(target_index)?.clone();
            (!source.is_empty() && !target.is_empty()).then_some((source, target))
        })
        .collect::<BTreeMap<_, _>>();
    map_descriptor_classes(descriptor, &class_names)
}

fn map_descriptor_classes(descriptor: &str, classes: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(descriptor.len());
    let mut rest = descriptor;
    while let Some(index) = rest.find('L') {
        output.push_str(&rest[..index + 1]);
        rest = &rest[index + 1..];
        let Some(end) = rest.find(';') else {
            output.push_str(rest);
            return output;
        };
        output.push_str(
            classes
                .get(&rest[..end])
                .map(String::as_str)
                .unwrap_or(&rest[..end]),
        );
        output.push(';');
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

fn unescape_tiny(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('0') => output.push('\0'),
            Some('\\') => output.push('\\'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use crate::classfile::{ParsedAnnotation, ParsedClass, ParsedMethod};
    use crate::jar::{ClassUniverse, ResourceEntry};
    use crate::model::{AnalysisLimits, Coverage};

    use super::*;

    #[test]
    fn tiny_v1_maps_classes_members_and_descriptors() {
        let tree = parse_tiny(
            b"v1\tofficial\tintermediary\n\
              CLASS\ta\tnet/minecraft/class_1\n\
              CLASS\tb\tnet/minecraft/class_2\n\
              FIELD\ta\tLb;\tx\tfield_1\n\
              METHOD\ta\t(Lb;)Lb;\ty\tmethod_1\n",
            "mapping",
            "mappings/mappings.tiny",
            "hash",
        )
        .unwrap();
        let mapping = tree.mapping("official", "intermediary").unwrap();

        assert_eq!(map_class_name("a", &mapping), "net/minecraft/class_1");
        assert_eq!(
            map_descriptor("(Lb;)Lb;", &mapping),
            "(Lnet/minecraft/class_2;)Lnet/minecraft/class_2;"
        );
        assert_eq!(
            map_member_name(MappingMemberKind::Method, "a", "y", "(Lb;)Lb;", &mapping),
            "method_1"
        );

        let reverse = tree.mapping("intermediary", "official").unwrap();
        assert_eq!(
            map_member_name(
                MappingMemberKind::Method,
                "net/minecraft/class_1",
                "method_1",
                "(Lnet/minecraft/class_2;)Lnet/minecraft/class_2;",
                &reverse,
            ),
            "y"
        );
    }

    #[test]
    fn conflicting_mapping_sources_are_ambiguous() {
        let first = NamespaceMapping {
            source: "official".to_string(),
            target: "intermediary".to_string(),
            source_id: "first".to_string(),
            resource_path: "first.tiny".to_string(),
            classes: BTreeMap::from([("a".to_string(), "class_1".to_string())]),
            fields: BTreeMap::new(),
            methods: BTreeMap::new(),
        };
        let mut second = first.clone();
        second.source_id = "second".to_string();
        second
            .classes
            .insert("a".to_string(), "different".to_string());

        assert!(merge_mappings(&[first, second]).is_err());
    }

    #[test]
    fn empty_mapping_artifact_is_a_real_identity_capability() {
        let tree = parse_tiny(
            b"v1\tofficial\tintermediary\n",
            "mapping",
            "mappings/mappings.tiny",
            "hash",
        )
        .unwrap();

        assert_eq!(tree.class_count(), 0);
        assert_eq!(tree.namespaces, vec!["official", "intermediary"]);
    }

    #[test]
    fn fabric_projects_the_complete_game_universe_before_analysis() {
        let mut game = class("a");
        game.methods.push(method("x", "()V"));
        let mut scanned = scanned(vec![
            artifact("minecraft", ArtifactKind::Minecraft, vec![game], Vec::new()),
            artifact(
                "intermediary",
                ArtifactKind::Runtime,
                Vec::new(),
                vec![ResourceEntry {
                    path: "mappings/mappings.tiny".to_string(),
                    bytes: b"v1\tofficial\tintermediary\n\
                             CLASS\ta\tnet/minecraft/class_1\n\
                             METHOD\ta\t()V\tx\tmethod_1\n"
                        .to_vec(),
                }],
            ),
        ]);

        let report = align_fabric_runtime(&mut scanned).unwrap();

        assert_eq!(
            report.runtime_namespace,
            Some(SymbolNamespace::Intermediary)
        );
        assert_eq!(
            scanned.artifacts[0].classes[0].name,
            "net/minecraft/class_1"
        );
        assert_eq!(scanned.artifacts[0].classes[0].methods[0].name, "method_1");
        assert_eq!(report.class_mapping_coverage.mapped, 1);
        assert_eq!(report.method_mapping_coverage.mapped, 1);
        assert!(scanned.universe.definitions("net/minecraft/class_1").len() == 1);
    }

    #[test]
    fn inherited_tiny_method_mapping_projects_an_overriding_declaration() {
        let mut implementation = class("a");
        implementation.interfaces.push("i".to_string());
        implementation.methods.push(method("x", "()V"));
        let mut contract = class("i");
        contract.methods.push(method("x", "()V"));
        let mut scanned = scanned(vec![
            artifact(
                "minecraft",
                ArtifactKind::Minecraft,
                vec![implementation, contract],
                Vec::new(),
            ),
            artifact(
                "intermediary",
                ArtifactKind::Runtime,
                Vec::new(),
                vec![ResourceEntry {
                    path: "mappings/mappings.tiny".to_string(),
                    bytes: b"v1\tofficial\tintermediary\n\
                             CLASS\ta\tnet/minecraft/class_1\n\
                             CLASS\ti\tnet/minecraft/class_2\n\
                             METHOD\ti\t()V\tx\tmethod_1\n"
                        .to_vec(),
                }],
            ),
        ]);

        align_fabric_runtime(&mut scanned).unwrap();

        assert_eq!(scanned.artifacts[0].classes[0].methods[0].name, "method_1");
        assert_eq!(scanned.artifacts[0].classes[1].methods[0].name, "method_1");
    }

    #[test]
    fn partial_mapping_is_reported_without_claiming_missing_symbols_are_mapped() {
        let mut scanned = scanned(vec![
            artifact(
                "minecraft",
                ArtifactKind::Minecraft,
                vec![class("a"), class("unmapped")],
                Vec::new(),
            ),
            artifact(
                "intermediary",
                ArtifactKind::Runtime,
                Vec::new(),
                vec![ResourceEntry {
                    path: "mappings/mappings.tiny".to_string(),
                    bytes: b"v1\tofficial\tintermediary\n\
                             CLASS\ta\tnet/minecraft/class_1\n"
                        .to_vec(),
                }],
            ),
        ]);

        let report = align_fabric_runtime(&mut scanned).unwrap();

        assert_eq!(report.class_mapping_coverage.total, 2);
        assert_eq!(report.class_mapping_coverage.mapped, 1);
        assert_eq!(scanned.coverage.classes_mapping_missing, 1);
        assert_eq!(scanned.artifacts[0].classes[1].name, "unmapped");
    }

    #[test]
    fn fabric_without_mapping_data_uses_official_identity_namespace() {
        let mut scanned = scanned(vec![artifact(
            "minecraft",
            ArtifactKind::Minecraft,
            vec![class("net/minecraft/Game")],
            Vec::new(),
        )]);

        let report = align_fabric_runtime(&mut scanned).unwrap();

        assert_eq!(report.runtime_namespace, Some(SymbolNamespace::Official));
        assert!(matches!(
            report.alignment,
            NamespaceAlignment::Aligned {
                runtime_namespace: SymbolNamespace::Official
            }
        ));
        assert_eq!(scanned.coverage.namespace_alignment_failures, 0);
        assert_eq!(scanned.artifacts[0].classes[0].name, "net/minecraft/Game");
    }

    #[test]
    fn quilt_empty_mapping_configuration_uses_identity_namespace() {
        let mut scanned = scanned(vec![artifact(
            "minecraft",
            ArtifactKind::Minecraft,
            vec![class("net/minecraft/Game")],
            Vec::new(),
        )]);

        let report = align_quilt_runtime(&mut scanned).unwrap();

        assert_eq!(report.runtime_namespace, Some(SymbolNamespace::Official));
        assert_eq!(scanned.coverage.namespace_alignment_failures, 0);
    }

    #[test]
    fn mapping_loader_rejects_identity_when_mod_and_game_symbol_spaces_differ() {
        let mut mod_class = class("example/ModClass");
        mod_class.super_name = Some("net/minecraft/Game".to_string());
        let mut scanned = scanned(vec![
            artifact(
                "minecraft",
                ArtifactKind::Minecraft,
                vec![class("a")],
                Vec::new(),
            ),
            artifact("mod", ArtifactKind::Mod, vec![mod_class], Vec::new()),
        ]);

        let readiness = align_quilt_runtime(&mut scanned).unwrap_err();

        assert_eq!(readiness.status, ReadinessStatus::Incomplete);
        assert_eq!(scanned.coverage.namespace_alignment_failures, 1);
        assert!(readiness.message.contains("different symbol space"));
    }

    #[test]
    fn loader_unit_contains_only_scanned_nested_members() {
        let mut scanned = scanned(vec![
            artifact(
                "minecraft",
                ArtifactKind::Minecraft,
                vec![class("net/minecraft/Game")],
                Vec::new(),
            ),
            artifact(
                "mod",
                ArtifactKind::Mod,
                Vec::new(),
                vec![
                    ResourceEntry {
                        path: "fabric.mod.json".to_string(),
                        bytes: Vec::new(),
                    },
                    ResourceEntry {
                        path: "META-INF/jars/library.jar!/nested.refmap.json".to_string(),
                        bytes: Vec::new(),
                    },
                ],
            ),
            artifact(
                "mapping",
                ArtifactKind::Runtime,
                Vec::new(),
                vec![ResourceEntry {
                    path: "mappings/mappings.tiny".to_string(),
                    bytes: b"v1\tofficial\tintermediary\n".to_vec(),
                }],
            ),
        ]);

        let report = align_fabric_runtime(&mut scanned).unwrap();
        let unit = report
            .loader_units
            .iter()
            .find(|unit| unit.id == "mod")
            .unwrap();

        assert_eq!(
            unit.members,
            vec![
                "mod".to_string(),
                "mod!/META-INF/jars/library.jar".to_string()
            ]
        );
    }

    #[test]
    fn modlauncher_uses_the_loader_selected_runtime_game_artifact() {
        let mut mixin = class("example/Mixin");
        mixin.annotations.push(ParsedAnnotation {
            descriptor: MIXIN_ANNOTATION.to_string(),
            values: BTreeMap::from([(
                "targets".to_string(),
                AnnotationValue::String("net.minecraft.Game".to_string()),
            )]),
        });
        let mut scanned = scanned(vec![
            artifact(
                "raw-minecraft",
                ArtifactKind::Minecraft,
                vec![class("a")],
                Vec::new(),
            ),
            artifact(
                "runtime-game",
                ArtifactKind::RuntimeGame,
                vec![class("net/minecraft/Game")],
                Vec::new(),
            ),
            artifact("mod", ArtifactKind::Mod, vec![mixin], Vec::new()),
        ]);
        crate::jar::rebuild_universe(&mut scanned);

        let report = align_modlauncher_runtime(&mut scanned, LoaderFamily::NeoForge).unwrap();

        assert_eq!(report.runtime_namespace, Some(SymbolNamespace::Identity));
        assert!(scanned.universe.definitions("a").is_empty());
        assert_eq!(scanned.universe.definitions("net/minecraft/Game").len(), 1);
    }

    fn scanned(artifacts: Vec<ParsedArtifact>) -> ScannedArtifacts {
        let mut scanned = ScannedArtifacts {
            artifact_reports: Vec::new(),
            artifacts,
            universe: ClassUniverse::default(),
            limits: AnalysisLimits::default(),
            coverage: Coverage::default(),
            warnings: Vec::new(),
            symbol_mappings: BTreeMap::new(),
        };
        crate::jar::rebuild_universe(&mut scanned);
        scanned
    }

    fn artifact(
        id: &str,
        kind: ArtifactKind,
        classes: Vec<ParsedClass>,
        resources: Vec<ResourceEntry>,
    ) -> ParsedArtifact {
        ParsedArtifact {
            id: id.to_string(),
            display_name: id.to_string(),
            kind,
            classes,
            refmaps: Vec::new(),
            resources,
            service_providers: Vec::new(),
        }
    }

    fn class(name: &str) -> ParsedClass {
        ParsedClass {
            definition_id: None,
            future_version_best_effort: false,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            annotations: Vec::new(),
            service_providers: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
        }
    }

    fn method(name: &str, descriptor: &str) -> ParsedMethod {
        ParsedMethod {
            name: name.to_string(),
            descriptor: descriptor.to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: Vec::new(),
            max_locals: None,
            instructions: Vec::new(),
        }
    }
}
