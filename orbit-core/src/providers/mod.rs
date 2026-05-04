//! 平台提供者抽象层。
//!
//! 定义 `ModProvider` trait 与统一的跨平台数据类型（`ResolvedMod`、`SearchResult` 等）。
//! 每个平台（Modrinth、CurseForge）各自实现此 trait，`resolver` 模块仅依赖此 trait，
//! 不耦合任何具体平台的 SDK。

pub mod rate_limiter;
pub mod modrinth;
pub mod curseforge;

use async_trait::async_trait;
use crate::error::OrbitError;

/// 根据配置创建 provider 列表，按 `resolver.platforms` 顺序。
pub fn create_providers(platforms: &[String]) -> Result<Vec<Box<dyn ModProvider>>, crate::error::OrbitError> {
    let ua = format!("orbit/{}", env!("CARGO_PKG_VERSION"));
    let mut providers: Vec<Box<dyn ModProvider>> = Vec::new();
    for name in platforms {
        match name.as_str() {
            "modrinth" => {
                providers.push(Box::new(modrinth::ModrinthProvider::new(&ua, 3)?) as Box<dyn ModProvider>);
            }
            "curseforge" => {
                eprintln!("warning: CurseForge support is not yet implemented, skipping");
            }
            other => {
                eprintln!("warning: unknown platform '{other}', skipped");
            }
        }
    }
    if providers.is_empty() {
        return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
            "no valid platforms configured in [resolver].platforms"
        )));
    }
    Ok(providers)
}

/// 默认仅 Modrinth 的 provider 列表。
pub fn create_providers_default() -> Result<Vec<Box<dyn ModProvider>>, crate::error::OrbitError> {
    create_providers(&["modrinth".into()])
}

// ---------------------------------------------------------------------------
// 统一数据类型
// ---------------------------------------------------------------------------

/// Modrinth 平台专属字段
#[derive(Debug, Clone)]
pub struct ModrinthResolvedInfo {
    pub project_id: String,
    pub version_id: String,
    /// Modrinth 的 version_number（如 "mc26.1.2-0.8.10-fabric"）
    pub version_number: String,
}

/// 平台解析后的统一模组信息
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    /// fabric.mod.json 的 `id`（即 mod_id，PubGrub 用此作为 PackageId）
    pub mod_id: String,
    /// fabric.mod.json 的 `version`
    pub version: String,
    /// SHA-1
    pub sha1: String,
    /// SHA-512（Modrinth 原生提供，用于下载校验）
    pub sha512: String,
    /// slug
    pub slug: String,
    /// 来源平台名称（"modrinth"、"curseforge" 等）
    pub provider: String,
    /// Modrinth 专属字段
    pub modrinth: Option<ModrinthResolvedInfo>,
    /// 发布时间（ISO 8601），provider 版本排序用
    pub date_published: String,
    /// 下载 URL
    pub download_url: String,
    /// jar 文件名
    pub filename: String,
    /// 前置依赖
    pub dependencies: Vec<ResolvedDependency>,
    /// 平台元数据声明的 client_side
    pub client_side: Option<SideSupport>,
    /// 平台元数据声明的 server_side
    pub server_side: Option<SideSupport>,
}

#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    /// 依赖的 slug（Modrinth 解析后），或 mod_id
    pub slug: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SideSupport {
    Required,
    Optional,
    Unsupported,
}

/// 统一搜索返回结果
#[derive(Debug, Clone)]
pub struct SearchResultItem {
    pub mod_id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub latest_version: String,
    pub downloads: u64,
    pub mc_versions: Vec<String>,
    pub client_side: Option<SideSupport>,
    pub server_side: Option<SideSupport>,
    pub categories: Vec<String>,
}

/// orbit info 命令的完整输出结构
#[derive(Debug, Clone)]
pub struct ModInfo {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub authors: Vec<String>,
    pub latest_version: String,
    pub downloads: u64,
    pub license: Option<String>,
    pub client_side: Option<SideSupport>,
    pub server_side: Option<SideSupport>,
    pub categories: Vec<String>,
    pub recent_versions: Vec<ModVersionInfo>,
    pub dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug, Clone)]
pub struct ModVersionInfo {
    pub version: String,
    pub mc_versions: Vec<String>,
    pub loader: String,
    pub released_at: String,
}

// ---------------------------------------------------------------------------
// 平台提供者特质
// ---------------------------------------------------------------------------

/// 统一平台提供者接口。
///
/// 每个支持的平台（Modrinth、CurseForge）各自实现此 trait。
/// `resolver` 只需依赖此 trait，无需绑定具体 SDK。
#[async_trait]
pub trait ModProvider: Send + Sync {
    /// 提供者名称（如 "modrinth", "curseforge"）
    fn name(&self) -> &'static str;

    /// 搜索模组
    async fn search(
        &self,
        query: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResultItem>, OrbitError>;

    /// 获取模组详细信息（供 orbit info 使用）
    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError>;

    /// 解析模组：给定 slug 和版本约束，找到最匹配的版本
    async fn resolve(
        &self,
        slug: &str,
        version_constraint: &str,
        mc_version: &str,
        loader: &str,
    ) -> Result<ResolvedMod, OrbitError>;

    /// 根据哈希反查版本（供 orbit sync 识别手动拖入的 jar）。
    /// 注意：Modrinth 使用 SHA-512，CurseForge 使用 murmur2。调用方应传入对应平台的哈希值。
    async fn get_version_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ResolvedMod>, OrbitError>;

    /// 批量哈希反查（一次请求查所有 hash，避免 N+1 查询）
    async fn get_versions_by_hashes(
        &self,
        hashes: &[String],
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        // 默认回退：逐个调用 get_version_by_hash
        let mut results = Vec::new();
        for hash in hashes {
            if let Some(m) = self.get_version_by_hash(hash).await? {
                results.push(m);
            }
        }
        Ok(results)
    }

    /// 获取模组的所有版本列表
    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError>;

    /// 批量获取多个 project 的版本列表（按 project_id）。
    /// 默认逐个调用 `get_versions`，Modrinth 等 provider 覆盖为高效批量实现。
    async fn get_versions_batch(
        &self,
        project_ids: &[String],
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        let mut results = Vec::new();
        for pid in project_ids {
            if let Ok(versions) = self.get_versions(pid, mc_version, loader).await {
                results.extend(versions);
            }
        }
        Ok(results)
    }

    /// 获取平台分类列表
    async fn get_categories(&self) -> Result<Vec<String>, OrbitError>;

    /// 获取项目的完整依赖列表（含可读名称/slug）
    /// 默认返回空，各平台可覆盖实现
    async fn fetch_dependencies(&self, _project_id: &str) -> Result<Vec<ResolvedDependency>, OrbitError> {
        Ok(vec![])
    }
}
