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

/// 下载进度回调：`(bytes_downloaded, total_bytes)`
pub type ProgressCallback = Box<dyn Fn(u64, u64) + Send + Sync>;

// ---------------------------------------------------------------------------
// 统一数据类型
// ---------------------------------------------------------------------------

/// 平台解析后的统一模组信息
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    /// orbit.toml [dependencies] 中的键名
    pub name: String,
    /// 平台内唯一 ID
    pub mod_id: String,
    /// 实际安装的版本号
    pub version: String,
    /// 下载 URL
    pub download_url: String,
    /// jar 文件名
    pub filename: String,
    /// SHA-256 校验值
    pub sha256: String,
    /// 前置依赖
    pub dependencies: Vec<ResolvedDependency>,
    /// 平台元数据声明的 client_side
    pub client_side: Option<SideSupport>,
    /// 平台元数据声明的 server_side
    pub server_side: Option<SideSupport>,
}

#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    pub name: String,
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

    /// 根据 SHA-256 哈希反查版本（供 orbit sync 识别手动拖入的 jar）
    async fn get_version_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ResolvedMod>, OrbitError>;

    /// 获取模组的所有版本列表
    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError>;

    /// 获取平台分类列表
    async fn get_categories(&self) -> Result<Vec<String>, OrbitError>;

    /// 获取项目的完整依赖列表（含可读名称/slug）
    /// 默认返回空，各平台可覆盖实现
    async fn fetch_dependencies(&self, _project_id: &str) -> Result<Vec<ResolvedDependency>, OrbitError> {
        Ok(vec![])
    }
}
