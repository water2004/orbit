//! Orbit Core — 业务逻辑层
//!
//! 定义 orbit.toml / orbit.lock 的数据结构、依赖解析引擎、
//! 平台提供者抽象及实现、JAR 文件解析等核心逻辑。
//!
//! 此 crate 不包含任何 CLI 或 UI 代码。

pub mod config;
pub mod detection;
pub mod error;
pub mod manifest;
pub mod lockfile;
pub mod jar;
pub mod metadata;
pub mod providers;

// 业务逻辑模块（逐步实现中）
pub mod resolver;
pub mod sync;
pub mod installer;
pub mod checker;
pub mod purge;

pub use config::{GlobalConfig, InstancesRegistry, InstanceEntry, orbit_data_dir, config_path};
pub use detection::LoaderDetectionService;
pub use error::OrbitError;
pub use manifest::OrbitManifest;
pub use lockfile::{OrbitLockfile, LockEntry, LockMeta};
pub use metadata::{ModLoader, mojang::McVersion};
pub use jar::FabricModInfo;
pub use providers::ModProvider;
