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
pub mod providers;
pub mod versions;

// 业务逻辑模块（逐步实现中）
pub mod checker;
pub mod installer;
pub mod jar_cache;
pub mod outdated;
pub mod purge;
pub mod resolver;
pub mod sync;
pub mod workspace;

pub use checker::{CheckResult, check_compatibility};
pub use config::{
    GlobalConfig, InstanceEntry, InstancesRegistry, config_path, orbit_data_dir, register_instance,
    remove_instance, set_default_instance,
};
pub use detection::LoaderDetectionService;
pub use error::OrbitError;
pub use installer::{
    InstallOptions, InstallPrompt, InstallReport, InstalledMod, ListOutput, ListedPackage,
    RemoveReport, RestoreOptions, RestoreReport, install_to_instance, list_dependencies,
    list_installed, remove_from_instance, restore_instance, upgrade_all_in_instance,
};
pub use jar_cache::{CacheSummary, clean_cache, inspect_cache};
pub use lockfile::{
    FileInfo, ImplantedMod, LockDependency, LockMeta, ModrinthInfo, OrbitLockfile, PackageEntry,
};
pub use manifest::OrbitManifest;
pub use metadata::{ModLoader, mojang::McVersion};
pub use outdated::{OutdatedMod, check_all_outdated};
pub use providers::ModProvider;
pub use purge::{CandidateConfig, find_config_candidates, remove_config_candidates};
pub use sync::{SyncReport, sync_instance};
pub use workspace::{Lockfile, ManifestFile};
