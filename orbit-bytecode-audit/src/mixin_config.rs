use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::Value;

use crate::classfile::InstructionKind;
use crate::jar::{ParsedArtifact, ResourceEntry, ScannedArtifacts};
use crate::model::{
    ConfigActivation, CoverageGap, CoverageGapKind, InactiveCandidate, InactiveCandidateKind,
    MixinActivation, ParsedMixinConfig, PluginDecision, RegisteredMixin, RegisteredMixinConfig,
    RegistrationSource, SideConstraint, Warning, WarningKind,
};

const MIXIN_ANNOTATION: &str = "Lorg/spongepowered/asm/mixin/Mixin;";
const DEFAULT_PRIORITY: i32 = 1000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct MixinDiscoveryPlan {
    fabric_metadata: bool,
    quilt_metadata: bool,
    manifest: bool,
    neoforge_metadata: bool,
}

impl MixinDiscoveryPlan {
    pub(crate) const FABRIC: Self = Self {
        fabric_metadata: true,
        quilt_metadata: false,
        manifest: false,
        neoforge_metadata: false,
    };
    pub(crate) const QUILT: Self = Self {
        fabric_metadata: true,
        quilt_metadata: true,
        manifest: false,
        neoforge_metadata: false,
    };
    pub(crate) const FORGE: Self = Self {
        fabric_metadata: false,
        quilt_metadata: false,
        manifest: true,
        neoforge_metadata: false,
    };
    pub(crate) const NEOFORGE: Self = Self {
        fabric_metadata: false,
        quilt_metadata: false,
        manifest: true,
        neoforge_metadata: true,
    };
}

#[derive(Debug, Default)]
pub(crate) struct MixinRegistry {
    pub configs: Vec<RegisteredMixinConfig>,
    pub mixins: Vec<RegisteredMixin>,
    pub inactive_candidates: Vec<InactiveCandidate>,
    pub coverage_gaps: Vec<CoverageGap>,
    active: HashMap<(String, String), usize>,
}

impl MixinRegistry {
    pub(crate) fn active_mixin(
        &self,
        artifact_id: &str,
        mixin_class: &str,
    ) -> Option<&RegisteredMixin> {
        self.active
            .get(&(artifact_id.to_string(), normalize_class(mixin_class)))
            .and_then(|index| self.mixins.get(*index))
    }

    #[cfg(test)]
    pub(crate) fn all_annotated(scanned: &ScannedArtifacts) -> Self {
        let mut registry = Self::default();
        for artifact in &scanned.artifacts {
            let refmap_paths = artifact
                .refmaps
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<BTreeSet<_>>();
            let test_refmap = (refmap_paths.len() == 1)
                .then(|| refmap_paths.first().cloned())
                .flatten();
            for class in &artifact.classes {
                if !class
                    .annotations
                    .iter()
                    .any(|annotation| annotation.descriptor == MIXIN_ANNOTATION)
                {
                    continue;
                }
                let registered = RegisteredMixin {
                    artifact_id: artifact.id.clone(),
                    config_path: "<test>".to_string(),
                    mixin_class: class.name.clone(),
                    side: SideConstraint::Common,
                    config_priority: DEFAULT_PRIORITY,
                    class_priority: DEFAULT_PRIORITY,
                    refmap: test_refmap.clone(),
                    required_config: false,
                    default_require: 0,
                    plugin: None,
                    plugin_decision: None,
                    activation: MixinActivation::RegisteredForCurrentSide,
                };
                let index = registry.mixins.len();
                registry.active.insert(
                    (
                        registered.artifact_id.clone(),
                        registered.mixin_class.clone(),
                    ),
                    index,
                );
                registry.mixins.push(registered);
            }
        }
        registry
    }
}

#[derive(Debug)]
struct ConfigDeclaration {
    artifact_id: String,
    metadata_path: String,
    config_path: String,
    side: SideConstraint,
    source: RegistrationSource,
    required_mods: Vec<String>,
    behavior_version: Option<String>,
    activation: ConfigActivation,
}

#[derive(Debug)]
struct PluginEvaluation {
    class_found: bool,
    nested_class: bool,
    dynamic_mixins: Vec<String>,
    coverage_gaps: Vec<CoverageGap>,
}

pub(crate) fn discover(
    scanned: &mut ScannedArtifacts,
    request: &crate::model::AuditRequest,
    plan: MixinDiscoveryPlan,
) -> MixinRegistry {
    let mut declarations = Vec::new();
    for artifact in &scanned.artifacts {
        if plan.fabric_metadata {
            discover_fabric(artifact, &mut declarations);
        }
        if plan.quilt_metadata {
            discover_quilt(artifact, &mut declarations);
        }
        if plan.neoforge_metadata {
            discover_neoforge(artifact, &request.active_mod_ids, &mut declarations);
        }
        if plan.manifest {
            discover_manifest(artifact, &mut declarations);
        }
        discover_static_calls(artifact, &mut declarations);
    }

    let mut registry = MixinRegistry::default();
    let mut seen_configs = HashSet::new();
    for declaration in declarations {
        let key = (
            declaration.artifact_id.clone(),
            scoped_resource_path(&declaration.metadata_path, &declaration.config_path),
        );
        if !seen_configs.insert(key) {
            continue;
        }
        register_config(scanned, request, declaration, &mut registry);
    }
    record_unregistered_mixins(scanned, &mut registry);
    scanned.coverage.mixin_configs_registered = registry
        .configs
        .iter()
        .filter(|config| config.activation == ConfigActivation::Active)
        .count();
    scanned.coverage.mixins_registered = registry.active.len();
    scanned.coverage.inactive_mixins = registry.inactive_candidates.len();
    scanned.coverage.plugin_controlled_mixins = registry
        .mixins
        .iter()
        .filter(|mixin| mixin.activation == MixinActivation::PluginControlled)
        .count();
    registry
}

fn discover_fabric(artifact: &ParsedArtifact, output: &mut Vec<ConfigDeclaration>) {
    for resource in artifact
        .resources
        .iter()
        .filter(|resource| leaf(&resource.path).eq_ignore_ascii_case("fabric.mod.json"))
    {
        let Ok(value) = orbit_loader_json::from_slice::<Value>(&resource.bytes) else {
            continue;
        };
        let Some(mixins) = value.get("mixins") else {
            continue;
        };
        if let Some(object) = mixins.as_object() {
            // Fabric schema 0 used separate common/client/server lists.
            for (key, side) in [
                ("common", SideConstraint::Common),
                ("client", SideConstraint::Client),
                ("server", SideConstraint::DedicatedServer),
            ] {
                for config in string_values(object.get(key)) {
                    output.push(declaration(
                        artifact,
                        resource,
                        config,
                        side,
                        RegistrationSource::FabricMetadata,
                    ));
                }
            }
            continue;
        }
        for entry in mixins.as_array().into_iter().flatten() {
            match entry {
                Value::String(config) => output.push(declaration(
                    artifact,
                    resource,
                    config.clone(),
                    SideConstraint::Common,
                    RegistrationSource::FabricMetadata,
                )),
                Value::Object(entry) => {
                    let Some(config) = entry.get("config").and_then(Value::as_str) else {
                        continue;
                    };
                    let side = match entry
                        .get("environment")
                        .and_then(Value::as_str)
                        .unwrap_or("*")
                    {
                        "client" => SideConstraint::Client,
                        "server" => SideConstraint::DedicatedServer,
                        _ => SideConstraint::Common,
                    };
                    output.push(declaration(
                        artifact,
                        resource,
                        config.to_string(),
                        side,
                        RegistrationSource::FabricMetadata,
                    ));
                }
                _ => {}
            }
        }
    }
}

fn discover_quilt(artifact: &ParsedArtifact, output: &mut Vec<ConfigDeclaration>) {
    for resource in artifact
        .resources
        .iter()
        .filter(|resource| leaf(&resource.path).eq_ignore_ascii_case("quilt.mod.json"))
    {
        let Ok(value) = orbit_loader_json::from_slice::<Value>(&resource.bytes) else {
            continue;
        };
        let Some(mixins) = value.get("mixin") else {
            continue;
        };
        let entries = match mixins {
            Value::Array(entries) => entries.iter().collect::<Vec<_>>(),
            value => vec![value],
        };
        for entry in entries {
            match entry {
                Value::String(config) => output.push(declaration(
                    artifact,
                    resource,
                    config.clone(),
                    SideConstraint::Common,
                    RegistrationSource::QuiltMetadata,
                )),
                Value::Object(entry) => {
                    let Some(config) = entry.get("config").and_then(Value::as_str) else {
                        continue;
                    };
                    let side = match entry
                        .get("environment")
                        .and_then(Value::as_str)
                        .unwrap_or("*")
                    {
                        "client" => SideConstraint::Client,
                        "dedicated_server" => SideConstraint::DedicatedServer,
                        _ => SideConstraint::Common,
                    };
                    output.push(declaration(
                        artifact,
                        resource,
                        config.to_string(),
                        side,
                        RegistrationSource::QuiltMetadata,
                    ));
                }
                _ => {}
            }
        }
    }
}

fn discover_manifest(artifact: &ParsedArtifact, output: &mut Vec<ConfigDeclaration>) {
    for resource in artifact
        .resources
        .iter()
        .filter(|resource| leaf(&resource.path).eq_ignore_ascii_case("META-INF/MANIFEST.MF"))
    {
        let Ok(manifest) = std::str::from_utf8(&resource.bytes) else {
            continue;
        };
        let attributes = manifest_attributes(manifest);
        let Some(configs) = attributes.get("mixinconfigs") else {
            continue;
        };
        for config in configs
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            output.push(declaration(
                artifact,
                resource,
                config.to_string(),
                SideConstraint::Common,
                RegistrationSource::ForgeManifest,
            ));
        }
    }
}

fn discover_neoforge(
    artifact: &ParsedArtifact,
    active_mod_ids: &BTreeSet<String>,
    output: &mut Vec<ConfigDeclaration>,
) {
    for resource in artifact.resources.iter().filter(|resource| {
        matches!(
            leaf(&resource.path).to_ascii_lowercase().as_str(),
            "meta-inf/neoforge.mods.toml" | "meta-inf/mods.toml"
        )
    }) {
        let Ok(value) = std::str::from_utf8(&resource.bytes)
            .ok()
            .and_then(|content| content.parse::<toml::Value>().ok())
            .ok_or(())
        else {
            continue;
        };
        for entry in value
            .get("mixins")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(table) = entry.as_table() else {
                continue;
            };
            let Some(config) = table.get("config").and_then(toml::Value::as_str) else {
                continue;
            };
            let required_mods = table
                .get("requiredMods")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            let missing = required_mods
                .iter()
                .filter(|mod_id| !active_mod_ids.contains(mod_id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let activation = if missing.is_empty() {
                ConfigActivation::Active
            } else {
                ConfigActivation::MissingRequiredMods { mod_ids: missing }
            };
            output.push(ConfigDeclaration {
                artifact_id: artifact.id.clone(),
                metadata_path: resource.path.clone(),
                config_path: config.to_string(),
                side: SideConstraint::Common,
                source: RegistrationSource::NeoForgeMetadata,
                required_mods,
                behavior_version: table
                    .get("behaviorVersion")
                    .and_then(toml::Value::as_str)
                    .map(str::to_string),
                activation,
            });
        }
    }
}

fn discover_static_calls(artifact: &ParsedArtifact, output: &mut Vec<ConfigDeclaration>) {
    for class in &artifact.classes {
        for method in &class.methods {
            for pair in method.instructions.windows(2) {
                let [constant, call] = pair else {
                    continue;
                };
                let (
                    InstructionKind::StringConstant(config),
                    InstructionKind::MethodCall(reference),
                ) = (&constant.kind, &call.kind)
                else {
                    continue;
                };
                if reference.owner == "org/spongepowered/asm/mixin/Mixins"
                    && reference.name == "addConfiguration"
                    && reference.descriptor == "(Ljava/lang/String;)V"
                {
                    output.push(ConfigDeclaration {
                        artifact_id: artifact.id.clone(),
                        metadata_path: String::new(),
                        config_path: config.clone(),
                        side: SideConstraint::Common,
                        source: RegistrationSource::StaticCode,
                        required_mods: Vec::new(),
                        behavior_version: None,
                        activation: ConfigActivation::Dynamic,
                    });
                }
            }
        }
    }
}

fn register_config(
    scanned: &mut ScannedArtifacts,
    request: &crate::model::AuditRequest,
    mut declaration: ConfigDeclaration,
    registry: &mut MixinRegistry,
) {
    if request.environment.physical_side == crate::model::PhysicalSide::Unknown
        && declaration.side != SideConstraint::Common
    {
        declaration.activation = ConfigActivation::PhysicalSideUnknown;
    } else if !declaration
        .side
        .applies_to(request.environment.physical_side)
    {
        declaration.activation = ConfigActivation::SideMismatch;
    }
    let resource_path = scoped_resource_path(&declaration.metadata_path, &declaration.config_path);
    let resource =
        find_config_resource(&scanned.artifacts, &declaration.artifact_id, &resource_path);
    let parsed = resource.and_then(|resource| parse_config(&resource.bytes).ok());
    if resource.is_none() {
        declaration.activation = ConfigActivation::MissingConfig;
        scanned.warnings.push(Warning::new(
            Some(declaration.artifact_id.clone()),
            resource_path.clone(),
            WarningKind::MalformedConfig,
            "registered Mixin config does not exist in the active runtime content",
        ));
    } else if parsed.is_none() {
        declaration.activation = ConfigActivation::MalformedConfig;
        scanned.warnings.push(Warning::new(
            Some(declaration.artifact_id.clone()),
            resource_path.clone(),
            WarningKind::MalformedConfig,
            "registered Mixin config is not valid JSON or has invalid field types",
        ));
    }
    let plugin_evaluation = if declaration.activation == ConfigActivation::Active {
        parsed
            .as_ref()
            .and_then(|config| config.plugin.as_deref())
            .map(|plugin| {
                evaluate_plugin(scanned, &declaration.artifact_id, plugin, &resource_path)
            })
    } else {
        None
    };
    if let Some(evaluation) = &plugin_evaluation {
        if evaluation.class_found {
            if evaluation.nested_class {
                scanned.coverage.nested_plugin_classes_resolved += 1;
            }
        } else {
            scanned.coverage.nested_plugin_classes_missing += 1;
        }
        registry
            .coverage_gaps
            .extend(evaluation.coverage_gaps.clone());
    }

    registry.configs.push(RegisteredMixinConfig {
        artifact_id: declaration.artifact_id.clone(),
        config_path: resource_path.clone(),
        side: declaration.side,
        registration: declaration.source,
        activation: declaration.activation.clone(),
        required_mods: declaration.required_mods,
        behavior_version: declaration.behavior_version,
        parsed: parsed.clone(),
    });
    let Some(mut config) = parsed else {
        return;
    };
    if let Some(evaluation) = &plugin_evaluation {
        config
            .mixins
            .extend(evaluation.dynamic_mixins.iter().cloned());
        config.mixins.sort();
        config.mixins.dedup();
    }
    if let Some(registered_config) = registry.configs.last_mut() {
        registered_config.parsed = Some(config.clone());
    }

    if matches!(
        declaration.activation,
        ConfigActivation::SideMismatch
            | ConfigActivation::PhysicalSideUnknown
            | ConfigActivation::MissingRequiredMods { .. }
    ) {
        if declaration.activation == ConfigActivation::PhysicalSideUnknown {
            registry.coverage_gaps.push(CoverageGap {
                artifact_id: Some(declaration.artifact_id.clone()),
                scope: resource_path.clone(),
                kind: CoverageGapKind::PhysicalSideUnknown,
                detail: "side-specific Mixin config was not activated because the physical side is unknown"
                    .to_string(),
                count: 1,
            });
        }
        registry.inactive_candidates.push(InactiveCandidate {
            artifact_id: declaration.artifact_id.clone(),
            class: None,
            config_path: Some(resource_path.clone()),
            kind: match declaration.activation {
                ConfigActivation::SideMismatch | ConfigActivation::PhysicalSideUnknown => {
                    InactiveCandidateKind::SideMismatch
                }
                _ => InactiveCandidateKind::MissingRequiredMods,
            },
            reason: "Mixin config is registered by metadata but inactive in this runtime"
                .to_string(),
        });
    }
    if declaration.activation == ConfigActivation::Dynamic {
        scanned.coverage.dynamically_registered_configs += 1;
        registry.coverage_gaps.push(CoverageGap {
            artifact_id: Some(declaration.artifact_id.clone()),
            scope: resource_path.clone(),
            kind: CoverageGapKind::DynamicMixinConfigRegistration,
            detail:
                "Mixins.addConfiguration call was recovered, but execution reachability is dynamic"
                    .to_string(),
            count: 1,
        });
    }

    let config_activation = declaration.activation.clone();
    let entries = config
        .mixins
        .iter()
        .map(|name| (name, SideConstraint::Common))
        .chain(
            config
                .client
                .iter()
                .map(|name| (name, SideConstraint::Client)),
        )
        .chain(
            config
                .server
                .iter()
                .map(|name| (name, SideConstraint::DedicatedServer)),
        );
    for (name, side) in entries {
        let Some(mixin_class) = qualify_mixin_class(config.package.as_deref(), name) else {
            scanned.coverage.invalid_mixin_class_names += 1;
            registry.coverage_gaps.push(CoverageGap {
                artifact_id: Some(declaration.artifact_id.clone()),
                scope: resource_path.clone(),
                kind: CoverageGapKind::MissingMixinClass,
                detail: format!("Mixin config contains an invalid class name: {name}"),
                count: 1,
            });
            continue;
        };
        let mixin_present = scanned
            .artifacts
            .iter()
            .find(|artifact| artifact.id == declaration.artifact_id)
            .is_some_and(|artifact| {
                artifact
                    .classes
                    .iter()
                    .any(|class| class.name == mixin_class)
            });
        let plugin_decision = (mixin_present && side.applies_to(request.environment.physical_side))
            .then(|| {
                config.plugin.as_deref().map(|plugin| {
                    evaluate_plugin_for_mixin(
                        scanned,
                        &declaration.artifact_id,
                        plugin,
                        &mixin_class,
                    )
                })
            })
            .flatten();
        if let Some(decision) = &plugin_decision {
            match decision {
                PluginDecision::AlwaysApply => {
                    scanned.coverage.plugin_decisions_proven_true += 1;
                }
                PluginDecision::NeverApply => {
                    scanned.coverage.plugin_decisions_proven_false += 1;
                }
                PluginDecision::Conditional { .. } => {
                    scanned.coverage.plugin_decisions_conditional += 1;
                }
                PluginDecision::Unknown { .. } => {
                    scanned.coverage.plugin_decisions_unknown += 1;
                }
            }
        }
        let activation = if !mixin_present
            || (request.environment.physical_side == crate::model::PhysicalSide::Unknown
                && side != SideConstraint::Common)
        {
            MixinActivation::Unknown
        } else if !side.applies_to(request.environment.physical_side) {
            MixinActivation::Inactive
        } else {
            match declaration.activation {
                ConfigActivation::Active => match plugin_decision.as_ref() {
                    Some(PluginDecision::AlwaysApply) => MixinActivation::PluginAccepted,
                    Some(PluginDecision::NeverApply) => MixinActivation::PluginRejected,
                    Some(PluginDecision::Conditional { .. } | PluginDecision::Unknown { .. }) => {
                        MixinActivation::PluginControlled
                    }
                    None => MixinActivation::RegisteredForCurrentSide,
                },
                ConfigActivation::PluginControlled => MixinActivation::PluginControlled,
                ConfigActivation::Dynamic => MixinActivation::Dynamic,
                ConfigActivation::PhysicalSideUnknown => MixinActivation::Unknown,
                _ => MixinActivation::Inactive,
            }
        };
        let registered = RegisteredMixin {
            artifact_id: declaration.artifact_id.clone(),
            config_path: resource_path.clone(),
            mixin_class,
            side,
            config_priority: config.priority,
            class_priority: config.mixin_priority,
            refmap: config
                .refmap
                .as_ref()
                .map(|path| scoped_resource_path(&resource_path, path))
                .map(|expected| {
                    find_config_resource(&scanned.artifacts, &declaration.artifact_id, &expected)
                        .map(|resource| resource.path.clone())
                        .unwrap_or(expected)
                }),
            required_config: config.required,
            default_require: config.default_require,
            plugin: config.plugin.clone(),
            plugin_decision,
            activation,
        };
        if !mixin_present {
            scanned.coverage.registered_mixin_classes_missing += 1;
            registry.coverage_gaps.push(CoverageGap {
                artifact_id: Some(registered.artifact_id.clone()),
                scope: registered.config_path.clone(),
                kind: CoverageGapKind::MissingMixinClass,
                detail: format!(
                    "registered Mixin class '{}' is absent from the active Loader artifact unit",
                    registered.mixin_class
                ),
                count: 1,
            });
        }
        if let Some(PluginDecision::Conditional { detail } | PluginDecision::Unknown { detail }) =
            registered.plugin_decision.as_ref()
        {
            registry.coverage_gaps.push(CoverageGap {
                artifact_id: Some(registered.artifact_id.clone()),
                scope: format!("{}::{}", registered.config_path, registered.mixin_class),
                kind: CoverageGapKind::PluginDecision,
                detail: detail.clone(),
                count: 1,
            });
        }
        let index = registry.mixins.len();
        if matches!(
            registered.activation,
            MixinActivation::RegisteredForCurrentSide
                | MixinActivation::PluginAccepted
                | MixinActivation::PluginControlled
        ) {
            registry.active.insert(
                (
                    registered.artifact_id.clone(),
                    registered.mixin_class.clone(),
                ),
                index,
            );
        } else if registered.activation == MixinActivation::PluginRejected
            || (registered.activation == MixinActivation::Inactive
                && config_activation == ConfigActivation::Active)
        {
            registry.inactive_candidates.push(InactiveCandidate {
                artifact_id: registered.artifact_id.clone(),
                class: Some(registered.mixin_class.clone()),
                config_path: Some(registered.config_path.clone()),
                kind: if registered.activation == MixinActivation::PluginRejected {
                    InactiveCandidateKind::PluginRejected
                } else {
                    InactiveCandidateKind::SideMismatch
                },
                reason: if registered.activation == MixinActivation::PluginRejected {
                    "IMixinConfigPlugin.shouldApplyMixin statically rejected this Mixin".to_string()
                } else {
                    "Mixin entry does not apply to the current physical side".to_string()
                },
            });
        }
        registry.mixins.push(registered);
    }
    if config.plugin.is_some()
        && registry.mixins.iter().any(|mixin| {
            mixin.artifact_id == declaration.artifact_id && mixin.config_path == resource_path
        })
        && registry
            .mixins
            .iter()
            .filter(|mixin| {
                mixin.artifact_id == declaration.artifact_id && mixin.config_path == resource_path
            })
            .all(|mixin| mixin.activation == MixinActivation::PluginRejected)
    {
        registry.coverage_gaps.push(CoverageGap {
            artifact_id: Some(declaration.artifact_id),
            scope: resource_path,
            kind: CoverageGapKind::PluginDecision,
            detail: "all_mixins_rejected_by_plugin: every listed Mixin was statically proven false; the result was retained but flagged for consistency review"
                .to_string(),
            count: 1,
        });
    }
}

fn record_unregistered_mixins(scanned: &ScannedArtifacts, registry: &mut MixinRegistry) {
    let mut recorded = HashSet::new();
    for artifact in &scanned.artifacts {
        for class in &artifact.classes {
            if !class
                .annotations
                .iter()
                .any(|annotation| annotation.descriptor == MIXIN_ANNOTATION)
                || registry.mixins.iter().any(|registered| {
                    registered.artifact_id == artifact.id && registered.mixin_class == class.name
                })
                || !recorded.insert((artifact.id.clone(), class.name.clone()))
            {
                continue;
            }
            registry.inactive_candidates.push(InactiveCandidate {
                artifact_id: artifact.id.clone(),
                class: Some(class.name.clone()),
                config_path: None,
                kind: InactiveCandidateKind::UnregisteredConfig,
                reason: "class has @Mixin but is not named by an active Loader-registered config"
                    .to_string(),
            });
            registry.mixins.push(RegisteredMixin {
                artifact_id: artifact.id.clone(),
                config_path: "<unregistered>".to_string(),
                mixin_class: class.name.clone(),
                side: SideConstraint::Common,
                config_priority: DEFAULT_PRIORITY,
                class_priority: DEFAULT_PRIORITY,
                refmap: None,
                required_config: false,
                default_require: 0,
                plugin: None,
                plugin_decision: None,
                activation: MixinActivation::Unregistered,
            });
        }
    }
}

fn evaluate_plugin(
    scanned: &ScannedArtifacts,
    artifact_id: &str,
    plugin_class: &str,
    config_path: &str,
) -> PluginEvaluation {
    let normalized_plugin = normalize_class(plugin_class);
    let plugin = scanned
        .artifacts
        .iter()
        .find(|artifact| artifact.id == artifact_id)
        .and_then(|artifact| {
            artifact
                .classes
                .iter()
                .find(|class| class.name == normalized_plugin)
        });
    let Some(plugin) = plugin else {
        return PluginEvaluation {
            class_found: false,
            nested_class: false,
            dynamic_mixins: Vec::new(),
            coverage_gaps: vec![plugin_gap(
                artifact_id,
                config_path,
                CoverageGapKind::PluginDecision,
                "configured IMixinConfigPlugin class is not present in the active Loader artifact unit",
            )],
        };
    };
    let mut coverage_gaps = Vec::new();

    let dynamic_mixins = plugin
        .methods
        .iter()
        .find(|method| method.name == "getMixins" && method.descriptor == "()Ljava/util/List;")
        .map_or_else(Vec::new, |method| {
            recover_static_mixin_list(method).unwrap_or_else(|| {
                coverage_gaps.push(plugin_gap(
                    artifact_id,
                    config_path,
                    CoverageGapKind::PluginDynamicMixins,
                    "getMixins does not return a statically recoverable list",
                ));
                Vec::new()
            })
        });

    for method in plugin.methods.iter().filter(|method| {
        matches!(method.name.as_str(), "preApply" | "postApply")
            && method.instructions.iter().any(|instruction| {
                matches!(
                    &instruction.kind,
                    InstructionKind::FieldWrite(member)
                        if member.owner.starts_with("org/objectweb/asm/tree/")
                ) || matches!(
                    &instruction.kind,
                    InstructionKind::MethodCall(member)
                        if member.owner.starts_with("org/objectweb/asm/tree/")
                )
            })
    }) {
        coverage_gaps.push(plugin_gap(
            artifact_id,
            &format!("{config_path}::{}{}", method.name, method.descriptor),
            CoverageGapKind::PluginClassMutation,
            "plugin preApply/postApply directly manipulates ASM tree state",
        ));
    }

    PluginEvaluation {
        class_found: true,
        nested_class: plugin
            .definition_id
            .as_ref()
            .is_some_and(|definition| definition.entry_path.contains("!/")),
        dynamic_mixins,
        coverage_gaps,
    }
}

fn evaluate_plugin_for_mixin(
    scanned: &ScannedArtifacts,
    artifact_id: &str,
    plugin_class: &str,
    mixin_class: &str,
) -> PluginDecision {
    let plugin_class = normalize_class(plugin_class);
    let Some(artifact) = scanned
        .artifacts
        .iter()
        .find(|artifact| artifact.id == artifact_id)
    else {
        return PluginDecision::Unknown {
            detail: "the plugin artifact is absent from the active Loader artifact unit"
                .to_string(),
        };
    };
    let Some(plugin) = artifact
        .classes
        .iter()
        .find(|class| class.name == plugin_class)
    else {
        return PluginDecision::Unknown {
            detail: "the configured plugin class is absent from the active Loader artifact unit"
                .to_string(),
        };
    };
    let Some(method) = plugin.methods.iter().find(|method| {
        method.name == "shouldApplyMixin"
            && method.descriptor == "(Ljava/lang/String;Ljava/lang/String;)Z"
    }) else {
        return PluginDecision::Unknown {
            detail: "shouldApplyMixin is inherited or its implementation is unavailable"
                .to_string(),
        };
    };
    let Some(mixin) = artifact
        .classes
        .iter()
        .find(|class| class.name == mixin_class)
    else {
        return PluginDecision::Unknown {
            detail: "the listed Mixin class is absent from the active Loader artifact unit"
                .to_string(),
        };
    };
    let targets = mixin
        .annotations
        .iter()
        .find(|annotation| annotation.descriptor == MIXIN_ANNOTATION)
        .map(mixin_target_names)
        .unwrap_or_default();
    if targets.is_empty() {
        return PluginDecision::Unknown {
            detail: "the Mixin has no statically recoverable target for plugin evaluation"
                .to_string(),
        };
    }
    let decisions = targets
        .iter()
        .map(|target| evaluate_should_apply(method, target, mixin_class))
        .collect::<Vec<_>>();
    merge_plugin_decisions(&decisions)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AbstractValue {
    Boolean(bool),
    String(String),
    Unknown,
}

fn evaluate_should_apply(
    method: &crate::classfile::ParsedMethod,
    target_class: &str,
    mixin_class: &str,
) -> PluginDecision {
    let mut stack = Vec::<AbstractValue>::new();
    let mut returns = Vec::<Option<bool>>::new();
    let mut dynamic_unknown = false;
    let mut has_branch = false;
    for instruction in &method.instructions {
        match &instruction.kind {
            InstructionKind::IntegerConstant(0) => stack.push(AbstractValue::Boolean(false)),
            InstructionKind::IntegerConstant(1) => stack.push(AbstractValue::Boolean(true)),
            InstructionKind::StringConstant(value) => {
                stack.push(AbstractValue::String(value.clone()));
            }
            InstructionKind::Load(1) => {
                stack.push(AbstractValue::String(target_class.replace('/', ".")));
            }
            InstructionKind::Load(2) => {
                stack.push(AbstractValue::String(mixin_class.replace('/', ".")));
            }
            InstructionKind::MethodCall(call) => {
                let Some(value) = evaluate_known_plugin_call(call, &mut stack) else {
                    dynamic_unknown = true;
                    stack.push(AbstractValue::Unknown);
                    continue;
                };
                stack.push(value);
            }
            InstructionKind::Return if instruction.reference.opcode == 172 => {
                returns.push(match stack.pop() {
                    Some(AbstractValue::Boolean(value)) => Some(value),
                    _ => None,
                });
                stack.clear();
            }
            InstructionKind::Jump => {
                // The ClassFile reader intentionally does not retain control-flow
                // edges. A branch can therefore prove only that the result is
                // conditional when distinct constant returns are visible; it can
                // never prove a global true/false result.
                has_branch = true;
                stack.clear();
            }
            InstructionKind::Return
            | InstructionKind::FieldRead(_)
            | InstructionKind::FieldWrite(_)
            | InstructionKind::InvokeDynamic { .. }
            | InstructionKind::Store(_)
            | InstructionKind::Type(_)
            | InstructionKind::DecimalConstant(_)
            | InstructionKind::NullConstant
            | InstructionKind::Other => {
                dynamic_unknown = true;
            }
            InstructionKind::IntegerConstant(_) | InstructionKind::Load(_) => {
                stack.push(AbstractValue::Unknown);
            }
        }
    }
    if returns.is_empty() || returns.iter().any(Option::is_none) || dynamic_unknown {
        return PluginDecision::Unknown {
            detail: "shouldApplyMixin reads dynamic state, calls an unknown helper, or exceeds the conservative evaluator"
                .to_string(),
        };
    }
    let values = returns.into_iter().flatten().collect::<BTreeSet<_>>();
    if has_branch {
        return if values.len() > 1 {
            PluginDecision::Conditional {
                detail: "reachable plugin paths return different decisions".to_string(),
            }
        } else {
            PluginDecision::Unknown {
                detail: "shouldApplyMixin branches and the available bytecode view cannot prove every reachable path"
                    .to_string(),
            }
        };
    }
    if values == BTreeSet::from([true]) {
        PluginDecision::AlwaysApply
    } else if values == BTreeSet::from([false]) {
        PluginDecision::NeverApply
    } else {
        PluginDecision::Conditional {
            detail: "reachable plugin paths return different decisions".to_string(),
        }
    }
}

fn evaluate_known_plugin_call(
    call: &crate::model::MemberReference,
    stack: &mut Vec<AbstractValue>,
) -> Option<AbstractValue> {
    let binary_string_predicate = call.owner == "java/lang/String"
        && matches!(
            call.name.as_str(),
            "equals" | "startsWith" | "endsWith" | "contains"
        )
        && call.descriptor.ends_with(")Z");
    if !binary_string_predicate {
        return None;
    }
    let argument = stack.pop()?;
    let receiver = stack.pop()?;
    let (AbstractValue::String(receiver), AbstractValue::String(argument)) = (receiver, argument)
    else {
        return Some(AbstractValue::Unknown);
    };
    let value = match call.name.as_str() {
        "equals" => receiver == argument,
        "startsWith" => receiver.starts_with(&argument),
        "endsWith" => receiver.ends_with(&argument),
        "contains" => receiver.contains(&argument),
        _ => return None,
    };
    Some(AbstractValue::Boolean(value))
}

fn merge_plugin_decisions(decisions: &[PluginDecision]) -> PluginDecision {
    if decisions
        .iter()
        .any(|decision| matches!(decision, PluginDecision::Unknown { .. }))
    {
        return PluginDecision::Unknown {
            detail: "at least one target-specific plugin decision is unknown".to_string(),
        };
    }
    if decisions
        .iter()
        .any(|decision| matches!(decision, PluginDecision::Conditional { .. }))
    {
        return PluginDecision::Conditional {
            detail: "at least one target-specific plugin decision is conditional".to_string(),
        };
    }
    let accepted = decisions
        .iter()
        .any(|decision| matches!(decision, PluginDecision::AlwaysApply));
    let rejected = decisions
        .iter()
        .any(|decision| matches!(decision, PluginDecision::NeverApply));
    match (accepted, rejected) {
        (true, false) => PluginDecision::AlwaysApply,
        (false, true) => PluginDecision::NeverApply,
        _ => PluginDecision::Conditional {
            detail: "the plugin accepts some Mixin targets and rejects others".to_string(),
        },
    }
}

fn mixin_target_names(annotation: &crate::classfile::ParsedAnnotation) -> Vec<String> {
    annotation
        .value("value")
        .into_iter()
        .chain(annotation.value("targets"))
        .flat_map(crate::classfile::AnnotationValue::strings)
        .filter_map(|value| normalize_class_name(&value))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn recover_static_mixin_list(method: &crate::classfile::ParsedMethod) -> Option<Vec<String>> {
    if method
        .instructions
        .iter()
        .any(|instruction| matches!(instruction.kind, InstructionKind::NullConstant))
    {
        return Some(Vec::new());
    }
    let has_static_factory = method.instructions.iter().any(|instruction| {
        matches!(
            &instruction.kind,
            InstructionKind::MethodCall(call)
                if matches!(
                    (call.owner.as_str(), call.name.as_str()),
                    ("java/util/List", "of")
                        | ("java/util/Arrays", "asList")
                        | ("java/util/Collections", "singletonList")
                )
        )
    });
    if !has_static_factory {
        return None;
    }
    Some(
        method
            .instructions
            .iter()
            .filter_map(|instruction| match &instruction.kind {
                InstructionKind::StringConstant(value) => Some(value.clone()),
                _ => None,
            })
            .collect(),
    )
}

fn plugin_gap(artifact_id: &str, scope: &str, kind: CoverageGapKind, detail: &str) -> CoverageGap {
    CoverageGap {
        artifact_id: Some(artifact_id.to_string()),
        scope: scope.to_string(),
        kind,
        detail: detail.to_string(),
        count: 1,
    }
}

fn parse_config(bytes: &[u8]) -> Result<ParsedMixinConfig, ()> {
    let value = orbit_loader_json::from_slice::<Value>(bytes).map_err(|_| ())?;
    let object = value.as_object().ok_or(())?;
    let injectors = object.get("injectors").and_then(Value::as_object);
    let overwrites = object.get("overwrites").and_then(Value::as_object);
    Ok(ParsedMixinConfig {
        required: bool_field(object, "required")?.unwrap_or(false),
        min_version: string_field(object, "minVersion")?,
        compatibility_level: string_field(object, "compatibilityLevel")?,
        package: string_field(object, "package")?,
        plugin: string_field(object, "plugin")?,
        refmap: string_field(object, "refmap")?,
        priority: integer_field(object, "priority")?.map_or(DEFAULT_PRIORITY, |value| {
            i32::try_from(value).unwrap_or(DEFAULT_PRIORITY)
        }),
        mixin_priority: integer_field(object, "mixinPriority")?.map_or(DEFAULT_PRIORITY, |value| {
            i32::try_from(value).unwrap_or(DEFAULT_PRIORITY)
        }),
        mixins: string_array(object.get("mixins"))?,
        client: string_array(object.get("client"))?,
        server: string_array(object.get("server"))?,
        default_require: injectors
            .map(|injectors| integer_field(injectors, "defaultRequire"))
            .transpose()?
            .flatten()
            .map_or(0, |value| u32::try_from(value).unwrap_or(u32::MAX)),
        default_group: injectors
            .map(|injectors| string_field(injectors, "defaultGroup"))
            .transpose()?
            .flatten()
            .unwrap_or_else(|| "default".to_string()),
        overwrite_require_annotations: overwrites
            .map(|overwrites| bool_field(overwrites, "requireAnnotations"))
            .transpose()?
            .flatten()
            .unwrap_or(false),
    })
}

fn declaration(
    artifact: &ParsedArtifact,
    resource: &ResourceEntry,
    config_path: String,
    side: SideConstraint,
    source: RegistrationSource,
) -> ConfigDeclaration {
    ConfigDeclaration {
        artifact_id: artifact.id.clone(),
        metadata_path: resource.path.clone(),
        config_path,
        side,
        source,
        required_mods: Vec::new(),
        behavior_version: None,
        activation: ConfigActivation::Active,
    }
}

fn find_config_resource<'a>(
    artifacts: &'a [ParsedArtifact],
    artifact_id: &str,
    expected_path: &str,
) -> Option<&'a ResourceEntry> {
    artifacts
        .iter()
        .find(|artifact| artifact.id == artifact_id)
        .and_then(|artifact| {
            artifact
                .resources
                .iter()
                .find(|resource| resource.path.eq_ignore_ascii_case(expected_path))
                .or_else(|| {
                    let expected_leaf = leaf(expected_path);
                    let mut matches = artifact.resources.iter().filter(|resource| {
                        leaf(&resource.path).eq_ignore_ascii_case(expected_leaf)
                    });
                    let first = matches.next()?;
                    matches.next().is_none().then_some(first)
                })
        })
}

fn scoped_resource_path(owner_path: &str, resource: &str) -> String {
    let resource = resource.trim().trim_start_matches('/').replace('\\', "/");
    owner_path.rfind("!/").map_or(resource.clone(), |index| {
        format!("{}!/{resource}", &owner_path[..index])
    })
}

fn qualify_mixin_class(package: Option<&str>, name: &str) -> Option<String> {
    let name = normalize_class_name(name)?;
    let Some(package) = package.filter(|package| !package.is_empty()) else {
        return Some(name);
    };
    let package = normalize_class_name(package)?
        .trim_end_matches('/')
        .to_string();
    if name == package || name.starts_with(&format!("{package}/")) {
        Some(name)
    } else {
        Some(format!("{package}/{name}"))
    }
}

fn normalize_class(name: &str) -> String {
    normalize_class_name(name).unwrap_or_else(|| name.trim().replace('.', "/"))
}

pub(crate) fn normalize_class_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('[') {
        return None;
    }
    let name = name
        .strip_prefix('L')
        .and_then(|value| value.strip_suffix(';'))
        .filter(|value| !value.is_empty())
        .unwrap_or(name);
    Some(name.replace('.', "/"))
}

fn leaf(path: &str) -> &str {
    path.rsplit("!/").next().unwrap_or(path)
}

fn string_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn string_array(value: Option<&Value>) -> Result<Vec<String>, ()> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| value.as_str().map(str::to_string).ok_or(()))
            .collect(),
        _ => Err(()),
    }
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<String>, ()> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(()),
    }
}

fn integer_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<i64>, ()> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value.as_i64().map(Some).ok_or(()),
        _ => Err(()),
    }
}

fn bool_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<bool>, ()> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(()),
    }
}

fn manifest_attributes(manifest: &str) -> HashMap<String, String> {
    let mut unfolded = Vec::<String>::new();
    for line in manifest.replace("\r\n", "\n").lines() {
        if let Some(continuation) = line.strip_prefix(' ') {
            if let Some(previous) = unfolded.last_mut() {
                previous.push_str(continuation);
            }
        } else {
            unfolded.push(line.to_string());
        }
    }
    unfolded
        .into_iter()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::classfile::{
        AnnotationValue, ParsedAnnotation, ParsedClass, ParsedInstruction, ParsedMethod,
    };
    use crate::jar::{
        ClassDefinition, ClassUniverse, ParsedArtifact, ResourceEntry, ScannedArtifacts,
    };
    use crate::model::{
        AnalysisLimits, ArtifactKind, AuditEnvironment, AuditRequest, ClassDefinitionId, Coverage,
        InstructionReference, MemberKind, MemberReference, PhysicalSide,
    };

    use super::*;

    fn discover(
        scanned: &mut ScannedArtifacts,
        request: &crate::model::AuditRequest,
    ) -> MixinRegistry {
        let plan = match request.environment.loader {
            crate::model::LoaderFamily::Fabric => MixinDiscoveryPlan::FABRIC,
            crate::model::LoaderFamily::Quilt => MixinDiscoveryPlan::QUILT,
            crate::model::LoaderFamily::Forge => MixinDiscoveryPlan::FORGE,
            crate::model::LoaderFamily::NeoForge => MixinDiscoveryPlan::NEOFORGE,
        };
        super::discover(scanned, request, plan)
    }

    #[test]
    fn fabric_registration_filters_side_and_associates_each_refmap() {
        let mut scanned =
            scanned(
                vec![resource(
                "fabric.mod.json",
                br#"{"schemaVersion":1,"mixins":[
                    "common.json",
                    {"config":"client.json","environment":"client"},
                    {"config":"server.json","environment":"server"}
                ]}"#,
            ),
            resource(
                "common.json",
                br#"{"package":"example","refmap":"common.refmap.json","mixins":["Common"]}"#,
            ),
            resource(
                "client.json",
                br#"{"package":"example","refmap":"client.refmap.json","mixins":["Client"]}"#,
            ),
            resource(
                "server.json",
                br#"{"package":"example","refmap":"server.refmap.json","mixins":["Server"]}"#,
            )],
                vec![
                    mixin_class("example/Common"),
                    mixin_class("example/Client"),
                    mixin_class("example/Server"),
                    mixin_class("example/Unregistered"),
                ],
            );

        let registry = discover(
            &mut scanned,
            &request("fabric", PhysicalSide::Client, BTreeSet::new()),
        );

        assert_eq!(registry.configs.len(), 3);
        assert!(registry.active_mixin("mod", "example/Common").is_some());
        assert!(registry.active_mixin("mod", "example/Client").is_some());
        assert!(registry.active_mixin("mod", "example/Server").is_none());
        assert_eq!(
            registry
                .active_mixin("mod", "example/Client")
                .unwrap()
                .refmap
                .as_deref(),
            Some("client.refmap.json")
        );
        assert!(registry.inactive_candidates.iter().any(|candidate| {
            candidate.kind == InactiveCandidateKind::UnregisteredConfig
                && candidate.class.as_deref() == Some("example/Unregistered")
        }));
    }

    #[test]
    fn forge_manifest_and_static_registration_are_discovered_separately() {
        let mut registration_class = empty_class("example/Bootstrap");
        registration_class.methods.push(method(
            "boot",
            "()V",
            vec![
                string_instruction(0, "static.json"),
                call_instruction(
                    1,
                    "org/spongepowered/asm/mixin/Mixins",
                    "addConfiguration",
                    "(Ljava/lang/String;)V",
                ),
                return_instruction(2),
            ],
        ));
        let mut scanned = scanned(
            vec![
                resource(
                    "META-INF/MANIFEST.MF",
                    b"Manifest-Version: 1.0\r\nMixinConfigs: manifest.json\r\n\r\n",
                ),
                resource("manifest.json", br#"{"mixins":[]}"#),
                resource("static.json", br#"{"mixins":[]}"#),
            ],
            vec![registration_class],
        );

        let registry = discover(
            &mut scanned,
            &request("forge", PhysicalSide::Client, BTreeSet::new()),
        );

        assert!(registry.configs.iter().any(|config| {
            config.registration == RegistrationSource::ForgeManifest
                && config.config_path == "manifest.json"
        }));
        assert!(registry.configs.iter().any(|config| {
            config.registration == RegistrationSource::StaticCode
                && config.activation == ConfigActivation::Dynamic
        }));
    }

    #[test]
    fn quilt_registration_uses_its_own_metadata_and_side_constraint() {
        let mut scanned = scanned(
            vec![
                resource(
                    "quilt.mod.json",
                    br#"{"mixin":[
                        "common.json",
                        {"config":"server.json","environment":"dedicated_server"}
                    ]}"#,
                ),
                resource(
                    "common.json",
                    br#"{"package":"example","mixins":["Common"]}"#,
                ),
                resource(
                    "server.json",
                    br#"{"package":"example","mixins":["Server"]}"#,
                ),
            ],
            vec![mixin_class("example/Common"), mixin_class("example/Server")],
        );

        let registry = discover(
            &mut scanned,
            &request("quilt", PhysicalSide::Client, BTreeSet::new()),
        );

        assert!(
            registry
                .configs
                .iter()
                .all(|config| { config.registration == RegistrationSource::QuiltMetadata })
        );
        assert!(registry.active_mixin("mod", "example/Common").is_some());
        assert!(registry.active_mixin("mod", "example/Server").is_none());
    }

    #[test]
    fn neoforge_required_mods_control_registration_without_becoming_risk_evidence() {
        let mut missing_scanned = scanned(
            vec![
                resource(
                    "META-INF/neoforge.mods.toml",
                    br#"[[mixins]]
config = "required.json"
requiredMods = ["dependency"]
behaviorVersion = "1"
"#,
                ),
                resource(
                    "required.json",
                    br#"{"package":"example","mixins":["Required"]}"#,
                ),
            ],
            vec![mixin_class("example/Required")],
        );
        let missing = discover(
            &mut missing_scanned,
            &request("neoforge", PhysicalSide::Client, BTreeSet::new()),
        );
        assert!(missing.active_mixin("mod", "example/Required").is_none());
        assert!(matches!(
            missing.configs[0].activation,
            ConfigActivation::MissingRequiredMods { .. }
        ));

        let mut scanned = scanned(
            vec![
                resource(
                    "META-INF/neoforge.mods.toml",
                    br#"[[mixins]]
config = "required.json"
requiredMods = ["dependency"]
"#,
                ),
                resource(
                    "required.json",
                    br#"{"package":"example","mixins":["Required"]}"#,
                ),
            ],
            vec![mixin_class("example/Required")],
        );
        let active = discover(
            &mut scanned,
            &request(
                "neoforge",
                PhysicalSide::Client,
                BTreeSet::from(["dependency".to_string()]),
            ),
        );
        assert!(active.active_mixin("mod", "example/Required").is_some());
    }

    #[test]
    fn plugin_constant_decision_and_static_get_mixins_are_evaluated() {
        let mut plugin = empty_class("example/Plugin");
        plugin.methods.extend([
            method(
                "shouldApplyMixin",
                "(Ljava/lang/String;Ljava/lang/String;)Z",
                vec![integer_instruction(0, 1), return_instruction(1)],
            ),
            method(
                "getMixins",
                "()Ljava/util/List;",
                vec![
                    string_instruction(0, "Extra"),
                    call_instruction(
                        1,
                        "java/util/Collections",
                        "singletonList",
                        "(Ljava/lang/Object;)Ljava/util/List;",
                    ),
                    return_instruction(2),
                ],
            ),
        ]);
        let mut scanned = scanned(
            vec![
                resource(
                    "fabric.mod.json",
                    br#"{"schemaVersion":1,"mixins":["plugin.json"]}"#,
                ),
                resource(
                    "plugin.json",
                    br#"{"package":"example","plugin":"example.Plugin","mixins":["Declared"]}"#,
                ),
            ],
            vec![
                plugin,
                mixin_class("example/Declared"),
                mixin_class("example/Extra"),
            ],
        );

        let registry = discover(
            &mut scanned,
            &request("fabric", PhysicalSide::Client, BTreeSet::new()),
        );

        assert_eq!(
            registry
                .active_mixin("mod", "example/Declared")
                .unwrap()
                .activation,
            MixinActivation::PluginAccepted
        );
        assert!(registry.active_mixin("mod", "example/Extra").is_some());
        assert!(
            registry
                .coverage_gaps
                .iter()
                .all(|gap| gap.kind != CoverageGapKind::PluginDecision)
        );
    }

    #[test]
    fn plugin_constant_false_rejects_registered_mixins() {
        let mut plugin = empty_class("example/Plugin");
        plugin.methods.push(method(
            "shouldApplyMixin",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            vec![integer_instruction(0, 0), return_instruction(1)],
        ));
        let mut scanned = scanned(
            vec![
                resource(
                    "fabric.mod.json",
                    br#"{"schemaVersion":1,"mixins":["plugin.json"]}"#,
                ),
                resource(
                    "plugin.json",
                    br#"{"package":"example","plugin":"example.Plugin","mixins":["Rejected"]}"#,
                ),
            ],
            vec![plugin, mixin_class("example/Rejected")],
        );

        let registry = discover(
            &mut scanned,
            &request("fabric", PhysicalSide::Client, BTreeSet::new()),
        );

        assert!(registry.active_mixin("mod", "example/Rejected").is_none());
        assert!(registry.inactive_candidates.iter().any(|candidate| {
            candidate.kind == InactiveCandidateKind::PluginRejected
                && candidate.class.as_deref() == Some("example/Rejected")
        }));
    }

    #[test]
    fn class_loading_plugin_logic_remains_unknown_without_executing_the_jvm() {
        let mut plugin = empty_class("example/Plugin");
        plugin.methods.push(method(
            "shouldApplyMixin",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            vec![
                string_instruction(0, "optional.Dependency"),
                call_instruction(
                    1,
                    "java/lang/Class",
                    "forName",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                ),
                integer_instruction(2, 1),
                integer_instruction(3, 0),
                return_instruction(4),
            ],
        ));
        let resources = vec![
            resource(
                "fabric.mod.json",
                br#"{"schemaVersion":1,"mixins":["plugin.json"]}"#,
            ),
            resource(
                "plugin.json",
                br#"{"package":"example","plugin":"example.Plugin","mixins":["Optional"]}"#,
            ),
        ];
        let classes = vec![plugin.clone(), mixin_class("example/Optional")];
        let mut present = scanned(resources.clone(), classes.clone());
        present.universe.classes.insert(
            "optional/Dependency".to_string(),
            vec![definition("optional/Dependency")],
        );
        let accepted = discover(
            &mut present,
            &request("fabric", PhysicalSide::Client, BTreeSet::new()),
        );
        assert_eq!(
            accepted
                .active_mixin("mod", "example/Optional")
                .unwrap()
                .activation,
            MixinActivation::PluginControlled
        );

        let mut absent = scanned(resources, classes);
        let rejected = discover(
            &mut absent,
            &request("fabric", PhysicalSide::Client, BTreeSet::new()),
        );
        assert_eq!(
            rejected
                .active_mixin("mod", "example/Optional")
                .unwrap()
                .activation,
            MixinActivation::PluginControlled
        );
    }

    #[test]
    fn dynamic_plugin_decision_and_classnode_mutation_are_coverage_gaps() {
        let mut plugin = empty_class("example/Plugin");
        plugin.methods.extend([
            method(
                "shouldApplyMixin",
                "(Ljava/lang/String;Ljava/lang/String;)Z",
                vec![
                    instruction(0, InstructionKind::Other),
                    return_instruction(1),
                ],
            ),
            method(
                "preApply",
                "()V",
                vec![
                    call_instruction(0, "org/objectweb/asm/tree/InsnList", "clear", "()V"),
                    return_instruction(1),
                ],
            ),
        ]);
        let mut scanned = scanned(
            vec![
                resource(
                    "fabric.mod.json",
                    br#"{"schemaVersion":1,"mixins":["plugin.json"]}"#,
                ),
                resource(
                    "plugin.json",
                    br#"{"package":"example","plugin":"example.Plugin","mixins":["Controlled"]}"#,
                ),
            ],
            vec![plugin, mixin_class("example/Controlled")],
        );

        let registry = discover(
            &mut scanned,
            &request("fabric", PhysicalSide::Client, BTreeSet::new()),
        );

        assert!(
            registry
                .coverage_gaps
                .iter()
                .any(|gap| { gap.kind == CoverageGapKind::PluginDecision })
        );
        assert!(
            registry
                .coverage_gaps
                .iter()
                .any(|gap| { gap.kind == CoverageGapKind::PluginClassMutation })
        );
        assert_eq!(
            registry
                .active_mixin("mod", "example/Controlled")
                .unwrap()
                .activation,
            MixinActivation::PluginControlled
        );
    }

    #[test]
    fn nested_config_plugin_and_cross_member_refmap_share_one_loader_unit() {
        let mut plugin = empty_class("example/Plugin");
        plugin.definition_id = Some(ClassDefinitionId {
            loader_unit_id: "mod".to_string(),
            artifact_id: "mod".to_string(),
            entry_path: "META-INF/jars/plugin.jar!/example/Plugin.class".to_string(),
            original_name: "example/Plugin".to_string(),
            runtime_name: "example/Plugin".to_string(),
            content_hash: "plugin".to_string(),
        });
        plugin.methods.push(method(
            "shouldApplyMixin",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            vec![integer_instruction(0, 1), return_instruction(1)],
        ));
        let mut scanned = scanned(
            vec![
                resource(
                    "META-INF/jars/config.jar!/fabric.mod.json",
                    br#"{"schemaVersion":1,"mixins":["nested.mixins.json"]}"#,
                ),
                resource(
                    "META-INF/jars/config.jar!/nested.mixins.json",
                    br#"{"package":"example","plugin":"example.Plugin","refmap":"nested.refmap.json","mixins":["NestedMixin"]}"#,
                ),
                resource(
                    "META-INF/jars/refmap.jar!/nested.refmap.json",
                    br#"{"mappings":{}}"#,
                ),
            ],
            vec![plugin, mixin_class("example/NestedMixin")],
        );

        let registry = discover(
            &mut scanned,
            &request("fabric", PhysicalSide::Client, BTreeSet::new()),
        );
        let registered = registry.active_mixin("mod", "example/NestedMixin").unwrap();

        assert_eq!(registered.activation, MixinActivation::PluginAccepted);
        assert_eq!(
            registered.refmap.as_deref(),
            Some("META-INF/jars/refmap.jar!/nested.refmap.json")
        );
        assert_eq!(scanned.coverage.nested_plugin_classes_resolved, 1);
        assert_eq!(scanned.coverage.nested_plugin_classes_missing, 0);
    }

    #[test]
    fn relative_mixin_names_starting_with_l_are_not_descriptors() {
        for name in [
            "LocalPlayerMixin",
            "LivingEntityMixin",
            "LevelMixin",
            "LevelChunkMixin",
            "LoadingOverlayMixin",
            "LootTableMixin",
        ] {
            assert_eq!(normalize_class_name(name).as_deref(), Some(name));
            assert_eq!(
                qualify_mixin_class(Some("example.mixin"), name),
                Some(format!("example/mixin/{name}"))
            );
        }
        assert_eq!(
            normalize_class_name("Lnet/minecraft/Foo;").as_deref(),
            Some("net/minecraft/Foo")
        );
        assert_eq!(normalize_class_name("[[Lnet/minecraft/Foo;"), None);
    }

    #[test]
    fn branching_plugin_result_is_conditional_or_unknown_but_never_rejected() {
        let branching = method(
            "shouldApplyMixin",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            vec![
                instruction(0, InstructionKind::Load(1)),
                string_instruction(1, "game.Target"),
                call_instruction(2, "java/lang/String", "equals", "(Ljava/lang/Object;)Z"),
                instruction(3, InstructionKind::Jump),
                integer_instruction(4, 0),
                return_instruction(5),
                integer_instruction(6, 1),
                return_instruction(7),
            ],
        );

        assert!(matches!(
            evaluate_should_apply(&branching, "game/Target", "example/Mixin"),
            PluginDecision::Conditional { .. }
        ));

        let one_visible_result = method(
            "shouldApplyMixin",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            vec![
                instruction(0, InstructionKind::Jump),
                integer_instruction(1, 0),
                return_instruction(2),
            ],
        );
        assert!(matches!(
            evaluate_should_apply(&one_visible_result, "game/Target", "example/Mixin"),
            PluginDecision::Unknown { .. }
        ));
    }

    #[test]
    fn plugin_receives_the_current_target_and_mixin_names() {
        let compares_mixin_name = method(
            "shouldApplyMixin",
            "(Ljava/lang/String;Ljava/lang/String;)Z",
            vec![
                instruction(0, InstructionKind::Load(2)),
                string_instruction(1, "example.AcceptedMixin"),
                call_instruction(2, "java/lang/String", "equals", "(Ljava/lang/Object;)Z"),
                return_instruction(3),
            ],
        );

        assert_eq!(
            evaluate_should_apply(&compares_mixin_name, "game/Target", "example/AcceptedMixin"),
            PluginDecision::AlwaysApply
        );
        assert_eq!(
            evaluate_should_apply(&compares_mixin_name, "game/Target", "example/RejectedMixin"),
            PluginDecision::NeverApply
        );
    }

    #[test]
    fn unknown_physical_side_does_not_activate_both_side_specific_configs() {
        let mut scanned = scanned(
            vec![
                resource(
                    "fabric.mod.json",
                    br#"{"schemaVersion":1,"mixins":[
                        {"config":"client.json","environment":"client"},
                        {"config":"server.json","environment":"server"}
                    ]}"#,
                ),
                resource(
                    "client.json",
                    br#"{"package":"example","mixins":["Client"]}"#,
                ),
                resource(
                    "server.json",
                    br#"{"package":"example","mixins":["Server"]}"#,
                ),
            ],
            vec![mixin_class("example/Client"), mixin_class("example/Server")],
        );

        let registry = discover(
            &mut scanned,
            &request("fabric", PhysicalSide::Unknown, BTreeSet::new()),
        );

        assert!(registry.active.is_empty());
        assert_eq!(
            registry
                .coverage_gaps
                .iter()
                .filter(|gap| gap.kind == CoverageGapKind::PhysicalSideUnknown)
                .count(),
            2
        );
    }

    #[test]
    fn common_config_parser_keeps_all_execution_semantics() {
        let parsed = parse_config(
            br#"{
                "required": true,
                "minVersion": "0.8.7",
                "compatibilityLevel": "JAVA_21",
                "package": "example.mixin",
                "plugin": "example.Plugin",
                "refmap": "example.refmap.json",
                "priority": 1200,
                "mixinPriority": 1100,
                "mixins": ["Common"],
                "client": ["Client"],
                "server": ["Server"],
                "injectors": {"defaultRequire": 2, "defaultGroup": "orbit"},
                "overwrites": {"requireAnnotations": true}
            }"#,
        )
        .unwrap();

        assert!(parsed.required);
        assert_eq!(parsed.min_version.as_deref(), Some("0.8.7"));
        assert_eq!(parsed.compatibility_level.as_deref(), Some("JAVA_21"));
        assert_eq!(parsed.package.as_deref(), Some("example.mixin"));
        assert_eq!(parsed.plugin.as_deref(), Some("example.Plugin"));
        assert_eq!(parsed.refmap.as_deref(), Some("example.refmap.json"));
        assert_eq!(parsed.priority, 1200);
        assert_eq!(parsed.mixin_priority, 1100);
        assert_eq!(parsed.default_require, 2);
        assert_eq!(parsed.default_group, "orbit");
        assert!(parsed.overwrite_require_annotations);
    }

    #[test]
    fn common_config_parser_accepts_loader_compatible_string_controls() {
        let parsed =
            parse_config(b"{\"package\":\"example\nmixin\",\"mixins\":[\"Common\"]}").unwrap();

        assert_eq!(parsed.package.as_deref(), Some("example\nmixin"));
    }

    fn request(
        loader: &str,
        physical_side: PhysicalSide,
        active_mod_ids: BTreeSet<String>,
    ) -> AuditRequest {
        AuditRequest {
            environment: AuditEnvironment {
                minecraft_version: "test".to_string(),
                loader: match loader {
                    "fabric" => crate::model::LoaderFamily::Fabric,
                    "quilt" => crate::model::LoaderFamily::Quilt,
                    "forge" => crate::model::LoaderFamily::Forge,
                    "neoforge" => crate::model::LoaderFamily::NeoForge,
                    _ => panic!("unsupported test loader"),
                },
                loader_version: "test".to_string(),
                physical_side,
                java_feature: 17,
            },
            artifacts: Vec::new(),
            active_mod_ids,
            limits: AnalysisLimits::default(),
        }
    }

    fn scanned(resources: Vec<ResourceEntry>, classes: Vec<ParsedClass>) -> ScannedArtifacts {
        ScannedArtifacts {
            artifact_reports: Vec::new(),
            artifacts: vec![ParsedArtifact {
                id: "mod".to_string(),
                display_name: "mod".to_string(),
                kind: ArtifactKind::Mod,
                classes,
                refmaps: Vec::new(),
                resources,
            }],
            universe: ClassUniverse::default(),
            limits: AnalysisLimits::default(),
            coverage: Coverage::default(),
            warnings: Vec::new(),
            symbol_mappings: BTreeMap::new(),
        }
    }

    fn resource(path: &str, bytes: &[u8]) -> ResourceEntry {
        ResourceEntry {
            path: path.to_string(),
            bytes: bytes.to_vec(),
        }
    }

    fn mixin_class(name: &str) -> ParsedClass {
        let mut class = empty_class(name);
        class.annotations.push(ParsedAnnotation {
            descriptor: MIXIN_ANNOTATION.to_string(),
            values: BTreeMap::from([(
                "targets".to_string(),
                AnnotationValue::Array(vec![AnnotationValue::String("game.Target".to_string())]),
            )]),
        });
        class
    }

    fn empty_class(name: &str) -> ParsedClass {
        ParsedClass {
            definition_id: None,
            future_version_best_effort: false,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            annotations: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
        }
    }

    fn definition(name: &str) -> ClassDefinition {
        ClassDefinition {
            definition_id: None,
            artifact_id: "runtime".to_string(),
            is_mod: false,
            name: name.to_string(),
            super_name: Some("java/lang/Object".to_string()),
            interfaces: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            hard_references: Vec::new(),
        }
    }

    fn method(name: &str, descriptor: &str, instructions: Vec<ParsedInstruction>) -> ParsedMethod {
        ParsedMethod {
            name: name.to_string(),
            descriptor: descriptor.to_string(),
            is_static: false,
            is_public: true,
            is_synthetic: false,
            annotations: Vec::new(),
            max_locals: Some(3),
            instructions,
        }
    }

    fn instruction(id: u32, kind: InstructionKind) -> ParsedInstruction {
        let (member, constant, opcode) = match &kind {
            InstructionKind::MethodCall(member) => (Some(member.clone()), None, 184),
            InstructionKind::StringConstant(value) => (None, Some(value.clone()), 18),
            InstructionKind::IntegerConstant(value) => (None, Some(value.to_string()), 4),
            InstructionKind::Return => (None, None, 172),
            _ => (None, None, 0),
        };
        ParsedInstruction {
            reference: InstructionReference {
                identity: None,
                stable_id: id,
                original_offset: Some(id),
                opcode,
                local_slot: None,
                member,
                constant,
            },
            kind,
        }
    }

    fn string_instruction(id: u32, value: &str) -> ParsedInstruction {
        instruction(id, InstructionKind::StringConstant(value.to_string()))
    }

    fn integer_instruction(id: u32, value: i64) -> ParsedInstruction {
        instruction(id, InstructionKind::IntegerConstant(value))
    }

    fn return_instruction(id: u32) -> ParsedInstruction {
        instruction(id, InstructionKind::Return)
    }

    fn call_instruction(id: u32, owner: &str, name: &str, descriptor: &str) -> ParsedInstruction {
        instruction(
            id,
            InstructionKind::MethodCall(MemberReference {
                owner: owner.to_string(),
                name: name.to_string(),
                descriptor: descriptor.to_string(),
                kind: MemberKind::Method,
                is_static: Some(true),
            }),
        )
    }
}
