//! 依赖解析引擎。
//!
//! 根据 orbit.toml 中的依赖声明和 orbit.lock 中的锁定版本，
//! 解析完整的依赖树：版本约束匹配、传递依赖展开、冲突检测。

use crate::error::OrbitError;
use crate::manifest::OrbitManifest;
use crate::lockfile::OrbitLockfile;
use crate::providers::ModProvider;

/// 解析后的依赖图
#[derive(Debug, Clone)]
pub struct ResolvedGraph {
    /// 需要安装的顶层依赖
    pub roots: Vec<ResolvedNode>,
    /// 全部节点（含传递依赖），去重
    pub all: Vec<ResolvedNode>,
}

#[derive(Debug, Clone)]
pub struct ResolvedNode {
    pub name: String,
    pub spec: crate::manifest::DependencySpec,
}

/// 解析依赖树。
///
/// `target` 参数用于 env 过滤：
/// - `"client"` — 仅安装 `env=client` + `env=both`
/// - `"server"` — 仅安装 `env=server` + `env=both`
/// - `"both"` — 安装全部
pub async fn resolve(
    _manifest: &OrbitManifest,
    _lockfile: Option<&OrbitLockfile>,
    _providers: &[Box<dyn ModProvider>],
    _target: &str,
) -> Result<ResolvedGraph, OrbitError> {
    // TODO: Phase 2 — 实现完整的依赖解析逻辑
    // 1. 遍历 [dependencies]
    // 2. 按 target 过滤 env
    // 3. 对每个依赖调用 provider.resolve()
    // 4. 递归展开传递依赖（除非被 exclude）
    // 5. 冲突检测
    todo!("dependency resolver not yet implemented")
}
