//! 模组下载与磁盘写入。
//!
//! 支持并发下载（tokio::task::JoinSet），
//! 下载完成后校验 SHA-256 并更新 orbit.lock。

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;

/// 安装报告
#[derive(Debug, Clone, Default)]
pub struct InstallReport {
    pub installed: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// 根据已解析的依赖图，并发下载并写入 mods/ 目录。
///
/// `concurrency` 控制最大并发下载数（默认 8）。
pub async fn install_all(
    _lockfile: &mut OrbitLockfile,
    _concurrency: usize,
) -> Result<InstallReport, OrbitError> {
    // TODO: Phase 2
    todo!("installer not yet implemented")
}
