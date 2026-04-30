//! Orbit Core — 业务逻辑层
//!
//! 定义 orbit.toml / orbit.lock 的数据结构、依赖解析引擎、
//! 平台提供者抽象及实现、JAR 文件解析等核心逻辑。
//!
//! 此 crate 不包含任何 CLI 或 UI 代码。

pub mod error;
pub mod manifest;
pub mod lockfile;
pub mod jar;
pub mod providers;

// 重新导出常用类型
pub use error::OrbitError;
pub use manifest::OrbitManifest;
pub use lockfile::{OrbitLockfile, LockEntry, LockMeta};
pub use jar::FabricModInfo;
pub use providers::ModProvider;
