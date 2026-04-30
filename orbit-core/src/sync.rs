//! 双向同步算法。
//!
//! 扫描 mods/ 目录，与 orbit.toml + orbit.lock 三方比对，
//! 识别差异并更新声明文件和锁文件（不产生网络下载）。

use crate::error::OrbitError;
use crate::manifest::OrbitManifest;
use crate::lockfile::OrbitLockfile;
use crate::providers::ModProvider;

/// 同步报告
#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub added: Vec<String>,        // 手动拖入的新 jar
    pub changed: Vec<String>,      // SHA-256 变化的 jar
    pub missing: Vec<String>,      // toml 声明了但文件缺失
    pub unlocked: Vec<String>,     // toml 有但 lock 无
}

/// 执行双向同步。
///
/// 五态比对模型：
/// - NEW: mods/ 有，toml 无 → 尝试 hash 识别，添加到 toml + lock
/// - MISSING: toml 有，mods/ 无 → 记录到 report
/// - CHANGED: toml 有，mods/ 有，SHA-256 ≠ lock → 更新 lock
/// - UNLOCKED: toml 有，lock 无 → 记录到 report
/// - OK: 三方一致 → 无操作
pub async fn sync(
    _manifest: &mut OrbitManifest,
    _lockfile: &mut OrbitLockfile,
    _providers: &[Box<dyn ModProvider>],
) -> Result<SyncReport, OrbitError> {
    // TODO: Phase 2 — 实现完整的同步逻辑
    todo!("sync algorithm not yet implemented")
}
