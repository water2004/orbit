//! Orbit Core — 业务逻辑层
//!
//! 定义 orbit.toml / orbit.lock 的数据结构、依赖解析引擎、
//! 平台提供者抽象及实现、JAR 文件解析等核心逻辑。
//!
//! 此 crate 不包含任何 CLI 或 UI 代码。

pub mod config;
mod dependency_environment;
mod detection;
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
pub mod version_repository;
pub mod version_string;
pub mod versions;

// 业务逻辑模块
pub mod archive;
pub mod audit;
pub mod installer;
pub mod jar_cache;
mod launcher;
pub mod loader;
pub mod migration;
pub mod outdated;
pub mod package_constraint;
pub mod package_versions;
mod platform;
mod platform_detection;
pub mod purge;
pub mod remote;
pub mod resolver;
mod source_store;
pub mod sync;
pub mod workspace;

pub use archive::{
    ExportReport, ImportMergeStrategy, ImportReport, PortableInstance, consume_portable_instance,
    export_instance, extract_portable_instance, import_archive, import_manifest, import_mrpack,
};
pub use audit::{audit_instance, audit_instance_with_progress};
pub use config::{
    ColorMode, ConfigKey, ConfigValue, GlobalConfig, InstanceEntry, InstancesRegistry,
    LanguagePreference, ProgressBarMode, clear_default_instance, persist_config_field,
    register_existing_instance, register_instance, remove_instance, set_default_instance,
};
pub use dependency_environment::{PackageEnvironmentReport, set_package_environment};
pub use error::OrbitError;
pub use installer::{
    InstallIntent, InstallInteraction, InstallOptions, InstallPrompt, InstallReport, InstallTarget,
    InstalledMod, InstanceInstallOptions, InstanceInstallReport, ListOutput, ListedPackage,
    PackageSelection, PackageSelector, RemoveReport, RemovedPackage, fix_instance,
    install_instance, install_local_file_to_instance, install_to_instance, list_installed,
    list_installed_for_target, list_packages, materialize_listed_package_icon,
    remove_from_instance, upgrade_all_in_instance,
};
pub use jar_cache::{CachePruneSummary, CacheSummary, JarCache, clean_cache, inspect_cache};
pub use loader::LoaderKind;
pub use lockfile::{ArtifactSource, BundledMod, LockMeta, OrbitLockfile, PackageEntry};
pub use manifest::{OrbitManifest, PackageRemote, PackageSpec, PlatformArtifact, PlatformSnapshot};
pub use metadata::mojang::McVersion;
pub use migration::{
    MigrationExportReport, MigrationFallbackConfirmation, MigrationFallbackPrompt,
    MigrationInteraction, MigrationOptions, MigrationPlan, export_migration, plan_migration,
    plan_migration_from_portable,
};
pub use orbit_bytecode_audit as audit_model;
pub use orbit_bytecode_audit::{
    Activation as AuditActivation, ArtifactKind as AuditArtifactKind, AuditReport,
    Confidence as AuditConfidence, Coverage as AuditCoverage, LoaderFamily as AuditLoaderFamily,
    Mechanism as AuditMechanism, MutationKind as AuditMutationKind,
    OrderAnalysis as AuditOrderAnalysis, Precision as AuditPrecision, Readiness as AuditReadiness,
    ReadinessStatus as AuditReadinessStatus, Risk as AuditRisk, Severity as AuditSeverity,
    SymbolNamespace as AuditSymbolNamespace, UnaryCompatibilityRisk as AuditUnaryRisk,
    WarningKind as AuditWarningKind,
};
pub use orbit_bytecode_audit::{AuditProgressEvent, AuditProgressReporter, AuditProgressStage};
pub use outdated::{
    OutdatedInteraction, OutdatedMod, check_all_outdated, check_all_outdated_with_progress,
    check_outdated_with_interaction,
};
pub use package_constraint::{
    PackageConstraintApplyOptions, PackageConstraintApplyReport, PackageConstraintState,
    PackageVersionPolicy, VersionComparison, apply_package_constraint, package_constraint,
};
pub use package_versions::{PackageVersionCandidate, PackageVersionsReport, list_package_versions};
pub use progress::{
    ArtifactProgressState, ProgressEvent, ProgressReporter, ResolutionActivity, ResolutionWork,
};
pub use providers::ModProvider;
pub use purge::{CandidateConfig, find_config_candidates, remove_config_candidates};
pub use remote::{RemoteReport, add_package_remote, list_package_remotes, remove_package_remote};
pub use resolver::types::{PackageChange, PackageChangeKind, ResolutionReport, ResolutionSelector};
pub use runtime::{
    NativeRuntimeEnvironment, PathLayout, RuntimeContext, RuntimeEnvironment, RuntimePathOptions,
    RuntimePaths, compiled_default_layout,
};
pub use sync::{PlatformChange, SyncReport, sync_instance};
pub use version_repository::{CandidateStorage, VersionRepository};
pub use version_string::{
    DEFAULT_NEW_PACKAGE_STRING, VersionStringInitialSet, VersionStringOperation,
    VersionStringPredicate, VersionStringRule,
};
pub use workspace::{Lockfile, ManifestFile};
