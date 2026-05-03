//! 深度清理启发式搜索。
//!
//! 按模组名称/slug 匹配 config/ 目录下的候选配置文件。

use crate::error::OrbitError;

/// 候选配置文件
#[derive(Debug, Clone)]
pub struct CandidateConfig {
    pub path: String,
    pub reason: String,
}

/// 启发式搜索 config/ 目录中与指定模组相关的配置文件。
pub fn find_config_candidates(
    _mod_name: &str,
    _mod_slug: Option<&str>,
    _config_dir: &std::path::Path,
) -> Result<Vec<CandidateConfig>, OrbitError> {
    // TODO: Phase 2
    // 1. 按名称模糊匹配（大小写不敏感、连字符/下划线模糊匹配）
    // 2. 按 slug 匹配
    Err(OrbitError::Other(anyhow::anyhow!("purge config scanner not yet implemented")))
}
