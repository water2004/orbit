//! 跨版本升级预检 (orbit check)。
//!
//! 检查当前已安装的模组集合是否已有目标 MC 版本的兼容版本。

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;

/// 兼容性检查结果
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub mod_name: String,
    pub current_version: String,
    pub compatible: bool,
    pub available_version: Option<String>,
}

/// 检查所有已安装模组在目标 MC 版本下的兼容性。
pub async fn check_compatibility(
    _lockfile: &OrbitLockfile,
    _target_mc_version: &str,
    _target_loader: &str,
) -> Result<Vec<CheckResult>, OrbitError> {
    // TODO: Phase 2
    Err(OrbitError::Other(anyhow::anyhow!("compatibility checker not yet implemented")))
}
