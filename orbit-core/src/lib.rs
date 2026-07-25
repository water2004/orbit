//! Orbit Core — 业务逻辑层
//!
//! 定义 orbit.toml / orbit.lock 的数据结构、依赖解析引擎、
//! 平台提供者抽象及实现、JAR 文件解析等核心逻辑。
//!
//! 此 crate 不包含任何 CLI 或 UI 代码。

pub mod config;
pub mod detection;
pub mod error;
pub mod identification;
pub mod init;
pub mod jar;
pub mod lockfile;
pub mod manifest;
pub mod metadata;
pub mod progress;
pub mod providers;
pub mod runtime;
pub mod versions;

// 业务逻辑模块
pub mod archive;
pub mod audit;
pub mod checker;
pub mod installer;
pub mod jar_cache;
mod launcher;
pub mod outdated;
mod package_reconciliation;
mod platform;
pub mod purge;
pub mod resolver;
pub mod sync;
pub mod workspace;

pub use archive::{
    ExportReport, ImportMergeStrategy, ImportReport, export_instance, import_archive,
    import_manifest, import_mrpack,
};
pub use audit::{audit_instance, audit_instance_with_progress};
pub use checker::{CheckResult, check_compatibility, check_compatibility_with_progress};
pub use config::{
    GlobalConfig, InstanceEntry, InstancesRegistry, register_instance, remove_instance,
    set_default_instance,
};
pub use detection::LoaderDetectionService;
pub use error::OrbitError;
pub use installer::{
    InstallIntent, InstallInteraction, InstallOptions, InstallPrompt, InstallReport, InstalledMod,
    ListOutput, ListedPackage, PackageSelector, RemoveReport, RemovedPackage, RestoreOptions,
    RestoreReport, install_local_file_to_instance, install_to_instance, list_dependencies,
    list_installed, list_installed_for_target, remove_from_instance, restore_instance,
    upgrade_all_in_instance,
};
pub use jar_cache::{CacheSummary, JarCache, clean_cache, inspect_cache};
pub use lockfile::{
    BundledMod, CurseForgeInfo, FileInfo, LockMeta, ModrinthInfo, OrbitLockfile, PackageEntry,
};
pub use manifest::{OrbitManifest, PlatformArtifact, PlatformArtifacts};
pub use metadata::{ModLoader, mojang::McVersion};
pub use orbit_bytecode_audit as audit_model;
pub use orbit_bytecode_audit::{
    Activation as AuditActivation, ArtifactKind as AuditArtifactKind, AuditReport,
    Confidence as AuditConfidence, Coverage as AuditCoverage, LoaderFamily as AuditLoaderFamily,
    Mechanism as AuditMechanism, MutationKind as AuditMutationKind,
    OrderAnalysis as AuditOrderAnalysis, Precision as AuditPrecision, Readiness as AuditReadiness,
    ReadinessStatus as AuditReadinessStatus, Risk as AuditRisk, Severity as AuditSeverity,
    WarningKind as AuditWarningKind,
};
pub use orbit_bytecode_audit::{AuditProgressEvent, AuditProgressReporter, AuditProgressStage};
pub use outdated::{
    OutdatedInteraction, OutdatedMod, check_all_outdated, check_all_outdated_with_progress,
    check_outdated_with_interaction,
};
pub use progress::{
    ArtifactProgressState, ProgressEvent, ProgressReporter, ResolutionActivity, ResolutionWork,
};
pub use providers::ModProvider;
pub use purge::{CandidateConfig, find_config_candidates, remove_config_candidates};
pub use resolver::types::{PackageChange, PackageChangeKind, ResolutionReport, ResolutionSelector};
pub use runtime::{
    NativeRuntimeEnvironment, PathLayout, RuntimeContext, RuntimeEnvironment, RuntimePathOptions,
    RuntimePaths, compiled_default_layout,
};
pub use sync::{PlatformChange, SyncReport, sync_instance};
pub use workspace::{Lockfile, ManifestFile};
