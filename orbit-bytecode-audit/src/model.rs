use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const REPORT_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone)]
pub struct AuditRequest {
    pub environment: AuditEnvironment,
    pub artifacts: Vec<ArtifactInput>,
    pub limits: AnalysisLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEnvironment {
    pub minecraft_version: String,
    pub declared_loader: String,
    pub detected_loader: String,
    pub loader_version: String,
}

#[derive(Debug, Clone)]
pub struct ArtifactInput {
    pub id: String,
    pub display_name: String,
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub nested_jars: NestedJarPolicy,
}

#[derive(Debug, Clone)]
pub enum NestedJarPolicy {
    /// Nested archives are not part of this runtime artifact.
    None,
    /// Every nested archive belongs to the runtime artifact.
    All,
    /// Only loader/resolver-selected archive paths are active. Paths use
    /// `outer.jar!/inner.jar` for deeper nesting.
    Selected(BTreeSet<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Minecraft,
    Loader,
    Runtime,
    Mod,
}

#[derive(Debug, Clone)]
pub struct AnalysisLimits {
    pub max_entries_per_jar: usize,
    pub max_entry_bytes: u64,
    pub max_jar_uncompressed_bytes: u64,
    pub max_class_bytes: usize,
    pub max_constant_pool_entries: usize,
    pub max_classes: usize,
    pub max_methods_per_class: usize,
    pub max_instructions_per_method: usize,
    pub max_annotation_depth: usize,
    pub max_nested_jar_depth: usize,
    pub max_interpreter_states: usize,
    pub max_helper_depth: usize,
}

impl Default for AnalysisLimits {
    fn default() -> Self {
        Self {
            max_entries_per_jar: 100_000,
            max_entry_bytes: 64 * 1024 * 1024,
            max_jar_uncompressed_bytes: 2 * 1024 * 1024 * 1024,
            max_class_bytes: 32 * 1024 * 1024,
            max_constant_pool_entries: 65_535,
            max_classes: 500_000,
            max_methods_per_class: 65_535,
            max_instructions_per_method: 1_000_000,
            max_annotation_depth: 32,
            max_nested_jar_depth: 8,
            max_interpreter_states: 50_000,
            max_helper_depth: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderFamily {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    Ready,
    Unsupported,
    Incomplete,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Readiness {
    pub status: ReadinessStatus,
    pub loader: Option<LoaderFamily>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub schema_version: String,
    pub environment: AuditEnvironment,
    pub readiness: Readiness,
    pub artifacts: Vec<ArtifactReport>,
    pub risks: Vec<Risk>,
    pub coverage: Coverage,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReport {
    pub id: String,
    pub display_name: String,
    pub path: String,
    pub kind: ArtifactKind,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub jars_scanned: usize,
    pub jars_failed: usize,
    pub classes_discovered: usize,
    pub classes_parsed: usize,
    pub classes_failed: usize,
    pub methods_parsed: usize,
    pub methods_degraded: usize,
    pub mixins_discovered: usize,
    pub effects_instruction_precision: usize,
    pub effects_method_precision: usize,
    pub effects_class_precision: usize,
    pub transformers_discovered: usize,
    pub transformer_targets_recovered: usize,
    pub transformer_effects_recovered: usize,
    pub transformer_effects_partial: usize,
    pub transformer_effects_unknown: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_mechanisms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub budget_exhaustions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub artifact_id: Option<String>,
    pub scope: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    #[must_use]
    pub fn score(self) -> u8 {
        match self {
            Self::Low => 25,
            Self::Medium => 50,
            Self::High => 75,
            Self::Critical => 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
    Exact,
}

impl Confidence {
    #[must_use]
    pub fn score(self) -> u8 {
        match self {
            Self::Low => 35,
            Self::Medium => 60,
            Self::High => 80,
            Self::Exact => 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    Instruction,
    Pattern,
    Method,
    Class,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Definite,
    Conditional,
    Candidate,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    Mixin,
    MixinExtras,
    ModLauncherTransformer,
    JavaCoremod,
    BinaryShape,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemberReference {
    pub owner: String,
    pub name: String,
    pub descriptor: String,
    pub kind: MemberKind,
    pub is_static: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    Method,
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClassReference {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstructionReference {
    pub stable_id: u32,
    pub original_offset: Option<u32>,
    pub opcode: u8,
    pub member: Option<MemberReference>,
    pub constant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Target {
    pub class: String,
    pub member: Option<MemberReference>,
    pub instruction: Option<InstructionReference>,
}

impl Target {
    #[must_use]
    pub fn class(class: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            member: None,
            instruction: None,
        }
    }

    #[must_use]
    pub fn method(
        owner: impl Into<String>,
        name: impl Into<String>,
        descriptor: impl Into<String>,
    ) -> Self {
        let owner = owner.into();
        Self {
            class: owner.clone(),
            member: Some(MemberReference {
                owner,
                name: name.into(),
                descriptor: descriptor.into(),
                kind: MemberKind::Method,
                is_static: None,
            }),
            instruction: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeRequirement {
    pub kind: RequirementKind,
    pub target: Target,
    pub minimum_matches: Option<u32>,
    pub maximum_matches: Option<u32>,
    pub ordinal: Option<u32>,
    pub slice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    ClassExists,
    MemberExists,
    InstructionExists,
    SliceBoundary,
    Cardinality,
    LocalLayout,
    ControlFlow,
    HardReference,
    UnknownShape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutation {
    pub kind: MutationKind,
    pub target: Target,
    pub exclusive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    ReplaceMethodBody,
    AddMethod,
    RemoveMethod,
    AddField,
    RemoveField,
    InsertInstructions,
    RemoveInstruction,
    ReplaceInstruction,
    RedirectOperation,
    WrapOperation,
    ModifyArgument,
    ModifyLocal,
    ModifyConstant,
    ChangeAccess,
    ChangeSuperclass,
    ChangeInterfaces,
    ChangeControlFlow,
    ChangeLocalLayout,
    UnknownMethod,
    UnknownClass,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub artifact_id: String,
    pub class: String,
    pub method: Option<String>,
    pub annotation: Option<String>,
    pub instruction: Option<InstructionReference>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Effect {
    pub artifact_id: String,
    pub mechanism: Mechanism,
    pub target: Target,
    pub requirements: Vec<ShapeRequirement>,
    pub mutations: Vec<Mutation>,
    pub evidence: Vec<Evidence>,
    pub precision: Precision,
    pub confidence: Confidence,
    pub activation: Activation,
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderAnalysis {
    Commutes,
    BothApplyDifferentResult,
    LeftMustRunFirst,
    RightMustRunFirst,
    AnchorInvalidated,
    OrdinalChanged,
    CardinalityInvalidated,
    Exclusive,
    Structural,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risk {
    pub left_artifact: String,
    pub right_artifact: String,
    pub target: Target,
    pub rule: String,
    pub reason: String,
    pub left_mutations: Vec<MutationKind>,
    pub right_mutations: Vec<MutationKind>,
    pub evidence: Vec<Evidence>,
    pub order: OrderAnalysis,
    pub severity: Severity,
    pub confidence: Confidence,
    /// Heuristic ranking value in `0..=100`; this is not a probability.
    pub risk_index: u8,
    pub activation: Activation,
}
