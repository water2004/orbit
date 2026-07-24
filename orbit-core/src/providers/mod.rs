//! 平台提供者抽象层。
//!
//! 定义 `ModProvider` trait 与统一的跨平台数据类型（`ResolvedMod`、`SearchResult` 等）。
//! 可用平台各自实现此 trait，`resolver` 模块仅依赖此 trait，不耦合具体 SDK。
//! 平台差异只保留在 provider 实现和各自的专属元数据中；安装、解析和锁文件编排
//! 统一消费这里的领域类型。

pub mod curseforge;
pub mod modrinth;
pub mod rate_limiter;

use crate::error::OrbitError;
use async_trait::async_trait;

/// 根据配置创建 provider 列表，按 `resolver.platforms` 顺序。
pub fn create_providers(
    platforms: &[String],
) -> Result<Vec<Box<dyn ModProvider>>, crate::error::OrbitError> {
    let config = crate::config::GlobalConfig::load()?;
    create_providers_with_auth(platforms, &config.auth)
}

pub fn create_providers_with_auth(
    platforms: &[String],
    auth: &crate::config::AuthConfig,
) -> Result<Vec<Box<dyn ModProvider>>, crate::error::OrbitError> {
    let ua = format!("orbit/{}", env!("CARGO_PKG_VERSION"));
    let mut providers: Vec<Box<dyn ModProvider>> = Vec::new();
    for name in platforms {
        match name.as_str() {
            "modrinth" => {
                providers.push(
                    Box::new(modrinth::ModrinthProvider::new(&ua, 3)?) as Box<dyn ModProvider>
                );
            }
            "curseforge" => {
                let api_key = auth
                    .curseforge_api_key
                    .as_deref()
                    .filter(|key| !key.trim().is_empty())
                    .ok_or_else(|| {
                        crate::error::OrbitError::Other(anyhow::anyhow!(
                            "CurseForge requires an API key; set \
                             ORBIT_CURSEFORGE_API_KEY or auth.curseforge_api_key in config.toml"
                        ))
                    })?;
                providers.push(
                    Box::new(curseforge::CurseForgeProvider::new(api_key, &ua, 3)?)
                        as Box<dyn ModProvider>,
                );
            }
            other => {
                return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                    "unsupported provider '{other}' in [resolver].platforms"
                )));
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

/// Create every provider that can safely participate in source identification
/// before a manifest exists. Modrinth is always available; authenticated
/// providers are added only when their credentials are configured.
pub fn create_identification_providers()
-> Result<Vec<Box<dyn ModProvider>>, crate::error::OrbitError> {
    let config = crate::config::GlobalConfig::load()?;
    let mut platforms = vec!["modrinth".to_string()];
    if config
        .auth
        .curseforge_api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        platforms.push("curseforge".to_string());
    }
    create_providers_with_auth(&platforms, &config.auth)
}

pub fn find_provider<'a>(
    providers: &'a [Box<dyn ModProvider>],
    name: &str,
) -> Option<&'a dyn ModProvider> {
    providers
        .iter()
        .find(|provider| provider.name() == name)
        .map(Box::as_ref)
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

/// CurseForge 平台专属字段
#[derive(Debug, Clone)]
pub struct CurseForgeResolvedInfo {
    pub project_id: u32,
    pub file_id: u32,
    /// CurseForge 的文件展示版本；JAR 自声明版本仍存放在 `ResolvedMod::version`。
    pub display_name: String,
    /// CurseForge 文件指纹，用于批量识别本地 JAR。
    pub fingerprint: u32,
}

/// 一个本地物理文件可用于各平台识别的摘要。
#[derive(Debug, Clone)]
pub struct ArtifactFingerprint {
    pub sha1: String,
    pub sha512: String,
    pub curseforge: u32,
}

/// 平台解析后的统一模组信息
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    /// Provider 查询阶段的项目别名；下载后以 JAR 的 mod_id 建图。
    pub mod_id: String,
    /// Provider 展示版本；下载后以 JAR 自声明版本建图。
    pub version: String,
    /// 来源提供的 SHA-1；缺失时为空。
    pub sha1: String,
    /// 来源提供的 SHA-512；缺失时为空。
    pub sha512: String,
    /// slug
    pub slug: String,
    /// 来源平台名称（"modrinth"、"curseforge" 等）
    pub provider: String,
    /// Modrinth 专属字段
    pub modrinth: Option<ModrinthResolvedInfo>,
    /// CurseForge 专属字段
    pub curseforge: Option<CurseForgeResolvedInfo>,
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

impl ResolvedMod {
    pub fn project_id(&self) -> Option<String> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.project_id.clone())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.project_id.to_string())
            })
    }

    pub fn version_id(&self) -> Option<String> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.version_id.clone())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.file_id.to_string())
            })
    }

    pub fn platform_version(&self) -> Option<&str> {
        self.modrinth
            .as_ref()
            .map(|metadata| metadata.version_number.as_str())
            .or_else(|| {
                self.curseforge
                    .as_ref()
                    .map(|metadata| metadata.display_name.as_str())
            })
    }

    pub fn matches_artifact(&self, artifact: &ArtifactFingerprint) -> bool {
        (!self.sha512.is_empty() && self.sha512.eq_ignore_ascii_case(&artifact.sha512))
            || (!self.sha1.is_empty() && self.sha1.eq_ignore_ascii_case(&artifact.sha1))
            || self.curseforge.as_ref().is_some_and(|metadata| {
                metadata.fingerprint != 0 && metadata.fingerprint == artifact.curseforge
            })
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    /// 依赖在来源平台上的 slug，或 JAR 元数据中的 mod_id。
    pub slug: Option<String>,
    pub required: bool,
    /// 来源平台的 project ID，用于继续查询依赖候选。
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SideSupport {
    Required,
    Optional,
    Unsupported,
}

impl std::fmt::Display for SideSupport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::Unsupported => "unsupported",
        };
        formatter.write_str(value)
    }
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
    pub project_id: String,
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
/// 每个可用平台各自实现此 trait。
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

    /// 批量识别本地物理文件。各平台选择自己的摘要：
    /// Modrinth 使用 SHA-512，CurseForge 使用其文件指纹。
    async fn identify_artifacts(
        &self,
        _artifacts: &[ArtifactFingerprint],
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        Ok(Vec::new())
    }

    /// 获取模组的所有版本列表
    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError>;

    /// 批量获取多个 project 的版本列表（按 project_id）。
    /// 默认逐个调用 `get_versions`；有批量端点的 provider 可覆盖此实现。
    async fn get_versions_batch(
        &self,
        project_ids: &[String],
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        let mut results = Vec::new();
        for pid in project_ids {
            results.extend(self.get_versions(pid, mc_version, loader).await?);
        }
        Ok(results)
    }

    /// 获取平台分类列表
    async fn get_categories(&self) -> Result<Vec<String>, OrbitError>;

    /// 获取项目的完整依赖列表（含可读名称/slug）
    /// 默认返回空，各平台可覆盖实现
    async fn fetch_dependencies(
        &self,
        _project_id: &str,
    ) -> Result<Vec<ResolvedDependency>, OrbitError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    #[test]
    fn curseforge_requires_an_explicit_api_key() {
        let error = create_providers_with_auth(&["curseforge".to_string()], &AuthConfig::default())
            .err()
            .expect("missing key should fail");
        assert!(error.to_string().contains("ORBIT_CURSEFORGE_API_KEY"));
    }

    #[test]
    fn provider_order_is_preserved_when_curseforge_is_enabled() {
        let providers = create_providers_with_auth(
            &["modrinth".to_string(), "curseforge".to_string()],
            &AuthConfig {
                curseforge_api_key: Some("test-key".to_string()),
                modrinth_token: None,
            },
        )
        .unwrap();
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.name())
                .collect::<Vec<_>>(),
            vec!["modrinth", "curseforge"]
        );
    }
}
