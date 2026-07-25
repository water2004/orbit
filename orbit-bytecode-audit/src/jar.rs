use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::AuditError;
use crate::classfile::{InstructionKind, ParsedClass};
use crate::model::{
    ArtifactKind, ArtifactReport, AuditRequest, ClassDefinitionId, Coverage, InstructionIdentity,
    LoaderFamily, MemberKind, MemberReference, Readiness, ReadinessStatus, Warning, WarningKind,
};

#[derive(Debug)]
pub(crate) struct ScannedArtifacts {
    pub artifact_reports: Vec<ArtifactReport>,
    pub artifacts: Vec<ParsedArtifact>,
    pub universe: ClassUniverse,
    pub limits: crate::model::AnalysisLimits,
    pub coverage: Coverage,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedArtifact {
    pub id: String,
    pub display_name: String,
    pub kind: ArtifactKind,
    pub classes: Vec<ParsedClass>,
    pub refmaps: Vec<RefmapEntry>,
    pub resources: Vec<ResourceEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResourceEntry {
    /// Normalized path, including `outer.jar!/` prefixes for nested archives.
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct RefmapEntry {
    pub path: String,
    pub context: Option<String>,
    pub mixin_class: String,
    pub original: String,
    pub mapped: String,
}

#[derive(Debug, Default)]
pub(crate) struct ClassUniverse {
    pub classes: BTreeMap<String, Vec<ClassDefinition>>,
}

#[derive(Debug)]
pub(crate) struct ClassDefinition {
    pub definition_id: Option<ClassDefinitionId>,
    pub artifact_id: String,
    pub is_mod: bool,
    pub name: String,
    pub super_name: Option<String>,
    pub interfaces: Vec<String>,
    pub fields: Vec<MemberReference>,
    pub methods: Vec<MemberReference>,
    pub hard_references: Vec<MemberReference>,
}

impl ClassUniverse {
    pub(crate) fn definitions(&self, class: &str) -> &[ClassDefinition] {
        self.classes
            .get(class)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn parsed_definitions<'a>(
        &'a self,
        artifacts: &'a [ParsedArtifact],
        class: &str,
    ) -> Vec<(&'a ParsedArtifact, &'a ParsedClass)> {
        artifacts
            .iter()
            .flat_map(|artifact| {
                artifact
                    .classes
                    .iter()
                    .filter(move |candidate| candidate.name == class)
                    .map(move |candidate| (artifact, candidate))
            })
            .collect()
    }

    pub(crate) fn definition_resolves_member(
        &self,
        definition: &ClassDefinition,
        reference: &MemberReference,
    ) -> bool {
        self.definition_resolves_member_inner(definition, reference, &mut HashSet::new())
    }

    fn definition_resolves_member_inner(
        &self,
        definition: &ClassDefinition,
        reference: &MemberReference,
        visited: &mut HashSet<String>,
    ) -> bool {
        if !visited.insert(definition.name.clone()) {
            return false;
        }
        if definition
            .fields
            .iter()
            .chain(&definition.methods)
            .any(|member| same_member_signature(member, reference))
        {
            return true;
        }
        definition
            .super_name
            .iter()
            .chain(&definition.interfaces)
            .any(|parent| {
                self.definitions(parent).iter().any(|parent_definition| {
                    self.definition_resolves_member_inner(
                        parent_definition,
                        reference,
                        &mut visited.clone(),
                    )
                })
            })
    }
}

fn same_member_signature(left: &MemberReference, right: &MemberReference) -> bool {
    left.name == right.name
        && left.descriptor == right.descriptor
        && left.kind == right.kind
        && (left.is_static.is_none()
            || right.is_static.is_none()
            || left.is_static == right.is_static)
}

pub(crate) fn probe_runtime_abi(scanned: &ScannedArtifacts, loader: LoaderFamily) -> Readiness {
    let runtime_artifacts = scanned
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind != ArtifactKind::Mod)
        .collect::<Vec<_>>();
    let mut classes = BTreeMap::<String, Vec<&ParsedClass>>::new();
    let mut minecraft_classes = 0_usize;
    let mut loader_classes = 0_usize;
    for artifact in runtime_artifacts {
        if artifact.kind == ArtifactKind::Minecraft {
            minecraft_classes += artifact.classes.len();
        }
        if artifact.kind == ArtifactKind::Loader {
            loader_classes += artifact.classes.len();
        }
        for class in &artifact.classes {
            classes.entry(class.name.clone()).or_default().push(class);
        }
    }
    if minecraft_classes == 0 {
        return incomplete(
            loader,
            "the Minecraft JAR contains no parseable base-game classes",
        );
    }
    if loader_classes == 0 {
        return incomplete(loader, "the loader JAR contains no parseable classes");
    }
    let mod_class_count = scanned
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Mod)
        .map(|artifact| artifact.classes.len())
        .sum::<usize>();
    if mod_class_count == 0 {
        return incomplete(loader, "the instance contains no parseable Mod classes");
    }

    let fabric = classes.contains_key("net/fabricmc/loader/impl/FabricLoaderImpl");
    let quilt = classes.contains_key("org/quiltmc/loader/impl/QuiltLoaderImpl");
    let forge = classes.contains_key("net/minecraftforge/fml/loading/FMLLoader");
    let neoforge = classes.contains_key("net/neoforged/fml/loading/FMLLoader");
    let actual_families = [
        (fabric, LoaderFamily::Fabric),
        (quilt, LoaderFamily::Quilt),
        (forge, LoaderFamily::Forge),
        (neoforge, LoaderFamily::NeoForge),
    ]
    .into_iter()
    .filter_map(|(present, family)| present.then_some(family))
    .collect::<Vec<_>>();
    if actual_families.len() > 1 {
        return Readiness {
            status: ReadinessStatus::Ambiguous,
            loader: None,
            message: format!(
                "runtime classpath contains conflicting loader implementations: {}",
                actual_families
                    .iter()
                    .map(|family| format!("{family:?}").to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            capabilities: Vec::new(),
        };
    }
    if actual_families
        .first()
        .is_some_and(|actual| *actual != loader)
    {
        return Readiness {
            status: ReadinessStatus::Ambiguous,
            loader: Some(loader),
            message: format!(
                "detected loader {loader:?}, but runtime classes identify {:?}",
                actual_families[0]
            ),
            capabilities: Vec::new(),
        };
    }
    if actual_families.is_empty() {
        return incomplete(
            loader,
            "the actual loader implementation marker is absent from the runtime classpath",
        );
    }
    let has_mixin = classes.contains_key("org/spongepowered/asm/mixin/Mixin");
    if !has_mixin {
        return incomplete(
            loader,
            "the runtime classpath does not contain the Mixin annotation ABI",
        );
    }

    match loader {
        LoaderFamily::Fabric | LoaderFamily::Quilt => Readiness {
            status: ReadinessStatus::Ready,
            loader: Some(loader),
            message: "runtime loader and Mixin ABI are available".to_string(),
            capabilities: vec!["mixin".to_string()],
        },
        LoaderFamily::Forge | LoaderFamily::NeoForge => probe_modlauncher(loader, &classes),
    }
}

fn probe_modlauncher(
    loader: LoaderFamily,
    classes: &BTreeMap<String, Vec<&ParsedClass>>,
) -> Readiness {
    let legacy = classes.contains_key("net/minecraft/launchwrapper/Launch")
        || classes.contains_key("net/minecraft/launchwrapper/IClassTransformer");
    let transformer = classes
        .get("cpw/mods/modlauncher/api/ITransformer")
        .and_then(|definitions| definitions.first());
    if transformer.is_none() && legacy {
        return Readiness {
            status: ReadinessStatus::Unsupported,
            loader: Some(loader),
            message: "当前实例使用 Legacy Forge/LaunchWrapper。\n字节码风险分析仅支持 ModLauncher 体系的现代 Forge 和 NeoForge。"
                .to_string(),
            capabilities: Vec::new(),
        };
    }
    let Some(transformer) = transformer else {
        return incomplete(
            loader,
            "Forge/NeoForge runtime classpath is missing ModLauncher ITransformer",
        );
    };
    let method = |name: &str, predicate: fn(&str) -> bool| {
        transformer
            .methods
            .iter()
            .any(|method| method.name == name && predicate(&method.descriptor))
    };
    let recognized = method("targets", |descriptor| descriptor == "()Ljava/util/Set;")
        && method("transform", |descriptor| {
            descriptor.contains("Lcpw/mods/modlauncher/api/ITransformerVotingContext;")
                && descriptor.ends_with(")Ljava/lang/Object;")
        })
        && method("getTargetType", |descriptor| {
            descriptor.ends_with("/ITransformer$TargetType;")
        })
        && method("castVote", |descriptor| {
            descriptor
                == "(Lcpw/mods/modlauncher/api/ITransformerVotingContext;)\
                Lcpw/mods/modlauncher/api/TransformerVoteResult;"
        });
    let target = classes
        .get("cpw/mods/modlauncher/api/ITransformer$Target")
        .and_then(|definitions| definitions.first());
    let target_type = classes.contains_key("cpw/mods/modlauncher/api/ITransformer$TargetType");
    let target_factory = |name: &str, descriptor: &str| {
        target.is_some_and(|target| {
            target
                .methods
                .iter()
                .any(|method| method.name == name && method.descriptor == descriptor)
        })
    };
    let target_abi = target_type
        && target_factory(
            "targetClass",
            "(Ljava/lang/String;)\
             Lcpw/mods/modlauncher/api/ITransformer$Target;",
        )
        && target_factory(
            "targetMethod",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)\
             Lcpw/mods/modlauncher/api/ITransformer$Target;",
        )
        && target_factory(
            "targetField",
            "(Ljava/lang/String;Ljava/lang/String;)\
             Lcpw/mods/modlauncher/api/ITransformer$Target;",
        );
    if !recognized || !target_abi {
        return Readiness {
            status: ReadinessStatus::Unsupported,
            loader: Some(loader),
            message: "the runtime contains ModLauncher, but its actual ITransformer/Target ABI is not recognized"
                .to_string(),
            capabilities: Vec::new(),
        };
    }
    let service = classes
        .get("cpw/mods/modlauncher/api/ITransformationService")
        .and_then(|definitions| definitions.first());
    if !service.is_some_and(|service| {
        service.methods.iter().any(|method| {
            method.name == "transformers" && method.descriptor == "()Ljava/util/List;"
        })
    }) {
        return incomplete(
            loader,
            "the ModLauncher ITransformationService transformers() ABI is missing",
        );
    }
    Readiness {
        status: ReadinessStatus::Ready,
        loader: Some(loader),
        message: "runtime Mixin, FML, and ModLauncher transformer ABIs are available".to_string(),
        capabilities: vec![
            "mixin".to_string(),
            "modlauncher_itransformer".to_string(),
            "java_coremod".to_string(),
        ],
    }
}

fn incomplete(loader: LoaderFamily, message: &str) -> Readiness {
    Readiness {
        status: ReadinessStatus::Incomplete,
        loader: Some(loader),
        message: message.to_string(),
        capabilities: Vec::new(),
    }
}

#[cfg(test)]
pub(crate) fn scan_artifacts(request: &AuditRequest) -> Result<ScannedArtifacts, AuditError> {
    scan_artifacts_with_progress(request, None)
}

pub(crate) fn scan_artifacts_with_progress(
    request: &AuditRequest,
    progress: Option<&crate::progress::AuditProgressReporter>,
) -> Result<ScannedArtifacts, AuditError> {
    use crate::progress::{AuditProgressEvent, AuditProgressStage, emit};

    let total = request.artifacts.len();
    emit(
        progress,
        AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::ScanArtifacts,
            total: Some(total),
        },
    );
    let mut artifact_reports = Vec::new();
    let mut artifacts = Vec::new();
    let mut universe = ClassUniverse::default();
    let mut coverage = Coverage::default();
    let mut warnings = Vec::new();
    let mut total_classes = 0_usize;
    let mut seen_paths = HashSet::new();
    let mut seen_hashes = HashSet::new();
    for (index, input) in request.artifacts.iter().enumerate() {
        let scanned_artifact = (|| {
            let normalized_path = normalized_path_identity(&input.path)?;
            let sha256 = hash_file(&input.path).map_err(|error| error.to_string())?;
            if seen_paths.contains(&normalized_path) || seen_hashes.contains(&sha256) {
                return Ok(None);
            }
            seen_paths.insert(normalized_path);
            seen_hashes.insert(sha256.clone());
            scan_artifact(input, request, &sha256, &mut coverage, &mut warnings).map(Some)
        })();
        match scanned_artifact {
            Ok(Some((report, artifact))) => {
                total_classes += artifact.classes.len();
                if total_classes > request.limits.max_classes {
                    return Err(AuditError::InvalidRequest(format!(
                        "class universe exceeds configured limit {}",
                        request.limits.max_classes
                    )));
                }
                for class in &artifact.classes {
                    let fields = class
                        .fields
                        .iter()
                        .map(|field| MemberReference {
                            owner: class.name.clone(),
                            name: field.name.clone(),
                            descriptor: field.descriptor.clone(),
                            kind: MemberKind::Field,
                            is_static: Some(field.is_static),
                        })
                        .collect::<Vec<_>>();
                    let methods = class
                        .methods
                        .iter()
                        .map(|method| method.reference(&class.name))
                        .collect::<Vec<_>>();
                    let hard_references = class
                        .methods
                        .iter()
                        .flat_map(|method| &method.instructions)
                        .filter_map(|instruction| match &instruction.kind {
                            InstructionKind::MethodCall(member)
                            | InstructionKind::FieldRead(member)
                            | InstructionKind::FieldWrite(member) => Some(member.clone()),
                            _ => None,
                        })
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();
                    universe
                        .classes
                        .entry(class.name.clone())
                        .or_default()
                        .push(ClassDefinition {
                            definition_id: class.definition_id.clone(),
                            artifact_id: artifact.id.clone(),
                            is_mod: artifact.kind == ArtifactKind::Mod,
                            name: class.name.clone(),
                            super_name: class.super_name.clone(),
                            interfaces: class.interfaces.clone(),
                            fields,
                            methods,
                            hard_references,
                        });
                }
                artifact_reports.push(report);
                artifacts.push(artifact);
            }
            Ok(None) => {}
            Err(error) if input.kind == ArtifactKind::Mod => {
                coverage.jars_failed += 1;
                warnings.push(Warning {
                    artifact_id: Some(input.id.clone()),
                    scope: "jar".to_string(),
                    kind: WarningKind::DamagedArtifact,
                    message: error,
                });
            }
            Err(error) => {
                return Err(AuditError::InvalidRequest(format!(
                    "required runtime artifact '{}': {error}",
                    input.path.display()
                )));
            }
        }
        emit(
            progress,
            AuditProgressEvent::Advanced {
                stage: AuditProgressStage::ScanArtifacts,
                completed: index + 1,
                total: Some(total),
            },
        );
    }
    if artifacts
        .iter()
        .filter(|artifact| artifact.kind == ArtifactKind::Mod)
        .all(|artifact| artifact.classes.is_empty())
    {
        return Err(AuditError::InvalidRequest(
            "no Mod JAR contains a parseable ClassFile".to_string(),
        ));
    }
    let scanned = ScannedArtifacts {
        artifact_reports,
        artifacts,
        universe,
        limits: request.limits.clone(),
        coverage,
        warnings,
    };
    emit(
        progress,
        AuditProgressEvent::StageFinished {
            stage: AuditProgressStage::ScanArtifacts,
            completed: total,
        },
    );
    Ok(scanned)
}

fn scan_artifact(
    input: &crate::model::ArtifactInput,
    request: &AuditRequest,
    sha256: &str,
    coverage: &mut Coverage,
    warnings: &mut Vec<Warning>,
) -> Result<(ArtifactReport, ParsedArtifact), String> {
    let metadata = std::fs::metadata(&input.path).map_err(|error| error.to_string())?;
    let file = File::open(&input.path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    let mut classes = Vec::new();
    let mut refmaps = Vec::new();
    let mut resources = Vec::new();
    let mut budget = ArchiveBudget::default();
    scan_archive(
        &mut archive,
        "",
        0,
        input,
        request,
        coverage,
        warnings,
        &mut budget,
        &mut classes,
        &mut refmaps,
        &mut resources,
    )?;
    Ok((
        ArtifactReport {
            id: input.id.clone(),
            display_name: input.display_name.clone(),
            path: input.path.to_string_lossy().into_owned(),
            kind: input.kind,
            size: metadata.len(),
            sha256: sha256.to_string(),
        },
        ParsedArtifact {
            id: input.id.clone(),
            display_name: input.display_name.clone(),
            kind: input.kind,
            classes,
            refmaps,
            resources,
        },
    ))
}

fn normalized_path_identity(path: &std::path::Path) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
    let value = canonical.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        Ok(value.to_ascii_lowercase())
    } else {
        Ok(value)
    }
}

#[derive(Default)]
struct ArchiveBudget {
    entries: usize,
    uncompressed: u64,
}

#[expect(clippy::too_many_arguments)]
fn scan_archive<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    prefix: &str,
    depth: usize,
    input: &crate::model::ArtifactInput,
    request: &AuditRequest,
    coverage: &mut Coverage,
    warnings: &mut Vec<Warning>,
    budget: &mut ArchiveBudget,
    classes: &mut Vec<ParsedClass>,
    refmaps: &mut Vec<RefmapEntry>,
    resources: &mut Vec<ResourceEntry>,
) -> Result<(), String> {
    let selected_class_entries =
        selected_class_entry_indexes(archive, request.environment.java_feature)?;
    budget.entries = budget.entries.saturating_add(archive.len());
    if budget.entries > request.limits.max_entries_per_jar {
        return Err(format!(
            "top-level and nested JAR entry count {} exceeds limit {}",
            budget.entries, request.limits.max_entries_per_jar
        ));
    }
    coverage.jars_scanned += 1;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if !entry.is_file() {
            continue;
        }
        budget.uncompressed = budget.uncompressed.saturating_add(entry.size());
        if budget.uncompressed > request.limits.max_jar_uncompressed_bytes {
            return Err(format!(
                "top-level and nested uncompressed JAR size exceeds limit {}",
                request.limits.max_jar_uncompressed_bytes
            ));
        }
        let entry_name = entry.name().replace('\\', "/");
        let name = if prefix.is_empty() {
            entry_name
        } else {
            format!("{prefix}!/{entry_name}")
        };
        if name.ends_with(".class") {
            if !selected_class_entries.contains(&index) {
                continue;
            }
            let entry_size = entry.size();
            scan_class_entry(
                &mut entry, entry_size, &name, input, request, coverage, warnings, classes,
            )?;
        } else if name.ends_with(".json")
            && entry.size() <= request.limits.max_entry_bytes.min(8 * 1024 * 1024)
        {
            let bytes = read_limited(
                &mut entry,
                usize::try_from(request.limits.max_entry_bytes.min(8 * 1024 * 1024))
                    .unwrap_or(8 * 1024 * 1024),
            )?;
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                refmaps.extend(parse_refmap(&name, &value));
                if name.to_ascii_lowercase().ends_with("coremods.json") {
                    record_unsupported_javascript_coremod(input, &name, coverage, warnings);
                }
            }
            resources.push(ResourceEntry { path: name, bytes });
        } else if is_registration_resource(&name)
            && entry.size() <= request.limits.max_entry_bytes.min(8 * 1024 * 1024)
        {
            let bytes = read_limited(
                &mut entry,
                usize::try_from(request.limits.max_entry_bytes.min(8 * 1024 * 1024))
                    .unwrap_or(8 * 1024 * 1024),
            )?;
            resources.push(ResourceEntry { path: name, bytes });
        } else if name.ends_with(".jar") {
            if !nested_jar_selected(&input.nested_jars, &name) {
                continue;
            }
            if depth >= request.limits.max_nested_jar_depth {
                coverage
                    .budget_exhaustions
                    .push(format!("{}:{name}: nested JAR depth", input.id));
                warnings.push(Warning {
                    artifact_id: Some(input.id.clone()),
                    scope: name,
                    kind: WarningKind::BudgetExhaustion,
                    message: format!(
                        "nested JAR depth exceeds limit {}",
                        request.limits.max_nested_jar_depth
                    ),
                });
                continue;
            }
            if entry.size() > request.limits.max_entry_bytes {
                coverage
                    .budget_exhaustions
                    .push(format!("{}:{name}: nested JAR entry size", input.id));
                warnings.push(Warning {
                    artifact_id: Some(input.id.clone()),
                    scope: name,
                    kind: WarningKind::BudgetExhaustion,
                    message: format!(
                        "nested JAR size {} exceeds entry limit {}",
                        entry.size(),
                        request.limits.max_entry_bytes
                    ),
                });
                continue;
            }
            let bytes = read_limited(
                &mut entry,
                usize::try_from(request.limits.max_entry_bytes).unwrap_or(usize::MAX),
            )?;
            drop(entry);
            match zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
                Ok(mut nested) => {
                    if let Err(error) = scan_archive(
                        &mut nested,
                        &name,
                        depth + 1,
                        input,
                        request,
                        coverage,
                        warnings,
                        budget,
                        classes,
                        refmaps,
                        resources,
                    ) {
                        coverage.jars_failed += 1;
                        coverage
                            .budget_exhaustions
                            .push(format!("{}:{name}: nested JAR scan", input.id));
                        warnings.push(Warning {
                            artifact_id: Some(input.id.clone()),
                            scope: name,
                            kind: WarningKind::BudgetExhaustion,
                            message: format!("nested JAR scan failed: {error}"),
                        });
                    }
                }
                Err(error) => {
                    coverage.jars_failed += 1;
                    warnings.push(Warning {
                        artifact_id: Some(input.id.clone()),
                        scope: name,
                        kind: WarningKind::DamagedArtifact,
                        message: format!("nested JAR is invalid: {error}"),
                    });
                }
            }
        } else if name.to_ascii_lowercase().ends_with(".js")
            && name.to_ascii_lowercase().contains("coremod")
        {
            record_unsupported_javascript_coremod(input, &name, coverage, warnings);
        }
    }
    Ok(())
}

fn selected_class_entry_indexes<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    java_feature: u32,
) -> Result<HashSet<usize>, String> {
    let mut multi_release = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if !entry.is_file()
            || !entry
                .name()
                .replace('\\', "/")
                .eq_ignore_ascii_case("META-INF/MANIFEST.MF")
        {
            continue;
        }
        // The Multi-Release flag is a main-section attribute. Some launchers
        // append megabytes of per-entry digest records to MANIFEST.MF, so
        // parsing the bounded main-section prefix must not reject the whole
        // runtime JAR merely because the complete manifest is large.
        let bytes = read_prefix(&mut entry, 1024 * 1024)?;
        multi_release = manifest_attribute(&bytes, "Multi-Release")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        break;
    }
    let mut selected = BTreeMap::<String, (u32, usize)>::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if !entry.is_file() {
            continue;
        }
        let path = entry.name().replace('\\', "/");
        let Some((logical_path, version)) = multi_release_class_path(&path) else {
            if path.ends_with(".class") && !path.starts_with("META-INF/versions/") {
                selected.entry(path).or_insert((0, index));
            }
            continue;
        };
        if multi_release && version <= java_feature {
            let replace = selected
                .get(&logical_path)
                .is_none_or(|(selected_version, _)| version > *selected_version);
            if replace {
                selected.insert(logical_path, (version, index));
            }
        }
    }
    Ok(selected.into_values().map(|(_, index)| index).collect())
}

fn read_prefix(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(limit).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn multi_release_class_path(path: &str) -> Option<(String, u32)> {
    let rest = path.strip_prefix("META-INF/versions/")?;
    let (version, logical_path) = rest.split_once('/')?;
    let version = version.parse::<u32>().ok()?;
    logical_path
        .ends_with(".class")
        .then(|| (logical_path.to_string(), version))
}

fn manifest_attribute(bytes: &[u8], requested: &str) -> Option<String> {
    let manifest = std::str::from_utf8(bytes).ok()?.replace("\r\n", "\n");
    let mut unfolded = Vec::<String>::new();
    for line in manifest.lines() {
        if let Some(continuation) = line.strip_prefix(' ') {
            if let Some(previous) = unfolded.last_mut() {
                previous.push_str(continuation);
            }
        } else {
            unfolded.push(line.to_string());
        }
    }
    unfolded.into_iter().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(requested)
            .then(|| value.trim().to_string())
    })
}

fn is_registration_resource(path: &str) -> bool {
    let leaf = path.rsplit("!/").next().unwrap_or(path);
    leaf.eq_ignore_ascii_case("META-INF/MANIFEST.MF")
        || leaf.eq_ignore_ascii_case("META-INF/mods.toml")
        || leaf.eq_ignore_ascii_case("META-INF/neoforge.mods.toml")
        || leaf.to_ascii_lowercase().starts_with("meta-inf/services/")
}

fn record_unsupported_javascript_coremod(
    input: &crate::model::ArtifactInput,
    scope: &str,
    coverage: &mut Coverage,
    warnings: &mut Vec<Warning>,
) {
    let mechanism = format!(
        "{}: JavaScript CoreMod is outside bytecode audit scope",
        input.id
    );
    if !coverage.unsupported_mechanisms.contains(&mechanism) {
        coverage.unsupported_mechanisms.push(mechanism);
    }
    warnings.push(Warning {
        artifact_id: Some(input.id.clone()),
        scope: scope.to_string(),
        kind: WarningKind::UnsupportedMechanism,
        message: "JavaScript CoreMod was not analyzed; only .class transformers are supported"
            .to_string(),
    });
}

fn nested_jar_selected(policy: &crate::model::NestedJarPolicy, path: &str) -> bool {
    match policy {
        crate::model::NestedJarPolicy::None => false,
        crate::model::NestedJarPolicy::All => true,
        crate::model::NestedJarPolicy::Selected(selected) => {
            selected.contains(path)
                || selected
                    .iter()
                    .any(|candidate| candidate.starts_with(&format!("{path}!/")))
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn scan_class_entry(
    entry: &mut impl Read,
    entry_size: u64,
    name: &str,
    input: &crate::model::ArtifactInput,
    request: &AuditRequest,
    coverage: &mut Coverage,
    warnings: &mut Vec<Warning>,
    classes: &mut Vec<ParsedClass>,
) -> Result<(), String> {
    coverage.classes_discovered += 1;
    if entry_size > u64::try_from(request.limits.max_class_bytes).unwrap_or(u64::MAX) {
        coverage.classes_failed += 1;
        warnings.push(Warning {
            artifact_id: Some(input.id.clone()),
            scope: name.to_string(),
            kind: WarningKind::BudgetExhaustion,
            message: format!(
                "ClassFile size {entry_size} exceeds limit {}",
                request.limits.max_class_bytes
            ),
        });
        return Ok(());
    }
    let bytes = read_limited(entry, request.limits.max_class_bytes)?;
    if class_constant_pool_count(&bytes)
        .is_some_and(|count| count > request.limits.max_constant_pool_entries)
    {
        coverage.classes_failed += 1;
        warnings.push(Warning {
            artifact_id: Some(input.id.clone()),
            scope: name.to_string(),
            kind: WarningKind::BudgetExhaustion,
            message: format!(
                "constant-pool count exceeds limit {}",
                request.limits.max_constant_pool_entries
            ),
        });
        return Ok(());
    }
    match crate::classfile::parse(&bytes, request.limits.max_annotation_depth) {
        Ok(mut class) => {
            let definition_id = ClassDefinitionId {
                artifact_id: input.id.clone(),
                entry_path: name.to_string(),
                class_name: class.name.clone(),
                content_hash: format!("{:x}", Sha256::digest(&bytes)),
            };
            for method in &mut class.methods {
                for instruction in &mut method.instructions {
                    instruction.reference.identity = Some(InstructionIdentity {
                        definition: definition_id.clone(),
                        method_name: method.name.clone(),
                        method_descriptor: method.descriptor.clone(),
                        instruction_index: instruction.reference.stable_id,
                    });
                }
            }
            class.definition_id = Some(definition_id);
            if class.future_version_best_effort {
                coverage.future_classfiles += 1;
            }
            if class.methods.len() > request.limits.max_methods_per_class {
                coverage.classes_failed += 1;
                warnings.push(Warning {
                    artifact_id: Some(input.id.clone()),
                    scope: class.name,
                    kind: WarningKind::BudgetExhaustion,
                    message: "method count exceeds configured limit".to_string(),
                });
                return Ok(());
            }
            for method in &mut class.methods {
                if method.instructions.len() > request.limits.max_instructions_per_method {
                    method.instructions.clear();
                    coverage.method_budget_degradations += 1;
                    warnings.push(Warning {
                        artifact_id: Some(input.id.clone()),
                        scope: format!("{}.{}{}", class.name, method.name, method.descriptor),
                        kind: WarningKind::BudgetExhaustion,
                        message: "instruction count exceeds configured limit; \
                                  method degraded to shape-only"
                            .to_string(),
                    });
                } else {
                    coverage.methods_parsed += 1;
                }
            }
            coverage.classes_parsed += 1;
            classes.push(class);
        }
        Err(error) => {
            coverage.classes_failed += 1;
            warnings.push(Warning {
                artifact_id: Some(input.id.clone()),
                scope: name.to_string(),
                kind: WarningKind::DamagedClass,
                message: format!("ClassFile parse failed: {error}"),
            });
        }
    }
    Ok(())
}

fn class_constant_pool_count(bytes: &[u8]) -> Option<usize> {
    (bytes.len() >= 10).then(|| usize::from(u16::from_be_bytes([bytes[8], bytes[9]])))
}

fn parse_refmap(path: &str, value: &Value) -> Vec<RefmapEntry> {
    let mut entries = Vec::new();
    if let Some(mappings) = value.get("mappings").and_then(Value::as_object) {
        flatten_refmap_context(path, None, mappings, &mut entries);
    }
    if let Some(contexts) = value.get("data").and_then(Value::as_object) {
        for (context, mappings) in contexts {
            if let Some(mappings) = mappings.as_object() {
                flatten_refmap_context(path, Some(context), mappings, &mut entries);
            }
        }
    }
    entries
}

fn flatten_refmap_context(
    path: &str,
    context: Option<&str>,
    mappings: &serde_json::Map<String, Value>,
    entries: &mut Vec<RefmapEntry>,
) {
    for (mixin_class, references) in mappings {
        let Some(references) = references.as_object() else {
            continue;
        };
        for (original, mapped) in references {
            let Some(mapped) = mapped.as_str() else {
                continue;
            };
            entries.push(RefmapEntry {
                path: path.to_string(),
                context: context.map(str::to_string),
                mixin_class: mixin_class.replace('.', "/"),
                original: original.clone(),
                mapped: mapped.to_string(),
            });
        }
    }
}

fn read_limited(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err(format!("entry exceeds read limit {limit}"));
    }
    Ok(bytes)
}

fn hash_file(path: &std::path::Path) -> Result<String, std::io::Error> {
    let mut file = File::open(path)?;
    file.rewind()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::{Cursor, Write};
    use std::path::Path;

    use zip::write::SimpleFileOptions;

    pub(crate) fn minimal_class(name: &str) -> Vec<u8> {
        class_with_abstract_methods(name, false, &[])
    }

    pub(crate) fn class_with_abstract_methods(
        name: &str,
        is_interface: bool,
        methods: &[(&str, &str)],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xCAFEBABE_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes.extend_from_slice(&61_u16.to_be_bytes());
        bytes.extend_from_slice(&u16::try_from(5 + methods.len() * 2).unwrap().to_be_bytes());
        push_utf8(&mut bytes, name);
        bytes.push(7);
        bytes.extend_from_slice(&1_u16.to_be_bytes());
        push_utf8(&mut bytes, "java/lang/Object");
        bytes.push(7);
        bytes.extend_from_slice(&3_u16.to_be_bytes());
        for (method, descriptor) in methods {
            push_utf8(&mut bytes, method);
            push_utf8(&mut bytes, descriptor);
        }
        bytes
            .extend_from_slice(&(if is_interface { 0x0601_u16 } else { 0x0421_u16 }).to_be_bytes());
        bytes.extend_from_slice(&2_u16.to_be_bytes());
        bytes.extend_from_slice(&4_u16.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // interfaces
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // fields
        bytes.extend_from_slice(&u16::try_from(methods.len()).unwrap().to_be_bytes());
        for index in 0..methods.len() {
            bytes.extend_from_slice(&0x0401_u16.to_be_bytes());
            bytes.extend_from_slice(&u16::try_from(5 + index * 2).unwrap().to_be_bytes());
            bytes.extend_from_slice(&u16::try_from(6 + index * 2).unwrap().to_be_bytes());
            bytes.extend_from_slice(&0_u16.to_be_bytes());
        }
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // attributes
        bytes
    }

    pub(crate) fn jar_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            archive
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    pub(crate) fn write_jar(path: &Path, classes: &[&str]) {
        let entries = classes
            .iter()
            .map(|name| (format!("{name}.class"), minimal_class(name)))
            .collect::<Vec<_>>();
        std::fs::write(path, jar_bytes(&entries)).unwrap();
    }

    pub(crate) fn write_class_entries(path: &Path, entries: Vec<(String, Vec<u8>)>) {
        std::fs::write(path, jar_bytes(&entries)).unwrap();
    }

    fn push_utf8(bytes: &mut Vec<u8>, value: &str) {
        bytes.push(1);
        bytes.extend_from_slice(&u16::try_from(value.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::model::{
        AnalysisLimits, ArtifactInput, ArtifactKind, AuditEnvironment, AuditRequest,
        NestedJarPolicy, PhysicalSide,
    };

    use super::test_support::{jar_bytes, minimal_class, write_jar};
    use super::*;

    #[test]
    fn nested_jar_classes_belong_to_the_outer_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let outer = directory.path().join("outer.jar");
        let nested = jar_bytes(&[(
            "nested/Only.class".to_string(),
            minimal_class("nested/Only"),
        )]);
        std::fs::write(
            &outer,
            jar_bytes(&[("META-INF/jars/nested.jar".to_string(), nested)]),
        )
        .unwrap();
        let scanned =
            scan_artifacts(&request(vec![input("outer", outer, ArtifactKind::Mod)])).unwrap();
        assert_eq!(scanned.artifacts[0].classes[0].name, "nested/Only");
        assert_eq!(scanned.coverage.jars_scanned, 2);
    }

    #[test]
    fn inactive_nested_jar_is_not_added_to_the_class_universe() {
        let directory = tempfile::tempdir().unwrap();
        let outer = directory.path().join("outer.jar");
        let nested = jar_bytes(&[(
            "nested/Inactive.class".to_string(),
            minimal_class("nested/Inactive"),
        )]);
        std::fs::write(
            &outer,
            jar_bytes(&[
                (
                    "root/Active.class".to_string(),
                    minimal_class("root/Active"),
                ),
                ("META-INF/jars/inactive.jar".to_string(), nested),
            ]),
        )
        .unwrap();
        let mut artifact = input("outer", outer, ArtifactKind::Mod);
        artifact.nested_jars = NestedJarPolicy::Selected(Default::default());

        let scanned = scan_artifacts(&request(vec![artifact])).unwrap();

        assert_eq!(scanned.artifacts[0].classes.len(), 1);
        assert_eq!(scanned.artifacts[0].classes[0].name, "root/Active");
        assert_eq!(scanned.coverage.jars_scanned, 1);
    }

    #[test]
    fn one_bad_mod_jar_does_not_discard_other_mods() {
        let directory = tempfile::tempdir().unwrap();
        let good = directory.path().join("good.jar");
        let bad = directory.path().join("bad.jar");
        write_jar(&good, &["mod/Good"]);
        std::fs::write(&bad, b"not a zip").unwrap();
        let scanned = scan_artifacts(&request(vec![
            input("bad", bad, ArtifactKind::Mod),
            input("good", good, ArtifactKind::Mod),
        ]))
        .unwrap();
        assert_eq!(scanned.coverage.jars_failed, 1);
        assert_eq!(scanned.artifacts.len(), 1);
        assert_eq!(scanned.artifacts[0].classes[0].name, "mod/Good");
    }

    #[test]
    fn top_level_artifact_progress_uses_the_exact_request_count() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.jar");
        let second = directory.path().join("second.jar");
        write_jar(&first, &["mod/First"]);
        write_jar(&second, &["mod/Second"]);
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let reporter: crate::progress::AuditProgressReporter = Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });

        scan_artifacts_with_progress(
            &request(vec![
                input("first", first, ArtifactKind::Mod),
                input("second", second, ArtifactKind::Mod),
            ]),
            Some(&reporter),
        )
        .unwrap();

        let events = events.lock().unwrap();
        assert_eq!(
            events.first(),
            Some(&crate::progress::AuditProgressEvent::StageStarted {
                stage: crate::progress::AuditProgressStage::ScanArtifacts,
                total: Some(2),
            })
        );
        assert!(
            events.contains(&crate::progress::AuditProgressEvent::Advanced {
                stage: crate::progress::AuditProgressStage::ScanArtifacts,
                completed: 1,
                total: Some(2),
            })
        );
        assert_eq!(
            events.last(),
            Some(&crate::progress::AuditProgressEvent::StageFinished {
                stage: crate::progress::AuditProgressStage::ScanArtifacts,
                completed: 2,
            })
        );
    }

    #[test]
    fn every_scan_reopens_the_current_file_without_a_persistent_cache() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mod.jar");
        write_jar(&path, &["mod/Before"]);
        let first = scan_artifacts(&request(vec![input(
            "mod",
            path.clone(),
            ArtifactKind::Mod,
        )]))
        .unwrap();
        write_jar(&path, &["mod/After"]);
        let second = scan_artifacts(&request(vec![input("mod", path, ArtifactKind::Mod)])).unwrap();
        assert_eq!(first.artifacts[0].classes[0].name, "mod/Before");
        assert_eq!(second.artifacts[0].classes[0].name, "mod/After");
        assert_ne!(
            first.artifact_reports[0].sha256,
            second.artifact_reports[0].sha256
        );
    }

    #[test]
    fn multi_release_jar_selects_the_highest_runtime_eligible_class() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("multi-release.jar");
        let mut base = minimal_class("example/Selected");
        base[6..8].copy_from_slice(&52_u16.to_be_bytes());
        let mut java_17 = minimal_class("example/Selected");
        java_17[6..8].copy_from_slice(&61_u16.to_be_bytes());
        let mut java_21 = minimal_class("example/Selected");
        java_21[6..8].copy_from_slice(&65_u16.to_be_bytes());
        std::fs::write(
            &path,
            jar_bytes(&[
                (
                    "META-INF/MANIFEST.MF".to_string(),
                    b"Manifest-Version: 1.0\r\nMulti-Release: true\r\n\r\n".to_vec(),
                ),
                ("example/Selected.class".to_string(), base),
                (
                    "META-INF/versions/17/example/Selected.class".to_string(),
                    java_17,
                ),
                (
                    "META-INF/versions/21/example/Selected.class".to_string(),
                    java_21,
                ),
            ]),
        )
        .unwrap();

        let scanned =
            scan_artifacts(&request(vec![input("mod", path, ArtifactKind::Mod)])).unwrap();

        assert_eq!(scanned.artifacts[0].classes.len(), 1);
        assert_eq!(
            scanned.artifacts[0].classes[0]
                .definition_id
                .as_ref()
                .unwrap()
                .entry_path,
            "META-INF/versions/17/example/Selected.class"
        );
    }

    #[test]
    fn duplicate_classpath_inputs_are_parsed_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("same.jar");
        write_jar(&path, &["example/Mod"]);
        let scanned = scan_artifacts(&request(vec![
            input("first", path.clone(), ArtifactKind::Mod),
            input("second", path, ArtifactKind::Mod),
        ]))
        .unwrap();

        assert_eq!(scanned.artifacts.len(), 1);
        assert_eq!(scanned.coverage.jars_scanned, 1);
    }

    fn request(artifacts: Vec<ArtifactInput>) -> AuditRequest {
        AuditRequest {
            environment: AuditEnvironment {
                minecraft_version: "test".to_string(),
                declared_loader: "fabric".to_string(),
                detected_loader: "fabric".to_string(),
                loader_version: "test".to_string(),
                physical_side: PhysicalSide::Unknown,
                java_feature: 17,
            },
            artifacts,
            active_mod_ids: Default::default(),
            limits: AnalysisLimits::default(),
        }
    }

    fn input(id: &str, path: std::path::PathBuf, kind: ArtifactKind) -> ArtifactInput {
        ArtifactInput {
            id: id.to_string(),
            display_name: id.to_string(),
            path,
            kind,
            nested_jars: NestedJarPolicy::All,
        }
    }
}
