//! 平台提供者抽象层。
//!
//! 定义 `ModProvider` trait 与统一的跨平台数据类型。
//! 可用平台各自实现此 trait，`resolver` 模块仅依赖此 trait，不耦合具体 SDK。
//! 平台差异只保留在 provider 实现和各自的专属元数据中；安装、解析和锁文件编排
//! 统一消费这里的领域类型。

pub mod curseforge;
pub mod download;
pub mod modrinth;
pub mod rate_limiter;

use crate::error::OrbitError;
use async_trait::async_trait;
pub use download::ArtifactDownloadClient;

#[derive(Clone)]
pub(crate) struct ProviderHttpConfig {
    pub timeout: std::time::Duration,
    pub max_retries: u32,
    pub proxy: Option<String>,
    pub max_concurrency: usize,
    pub download_limiter: std::sync::Arc<tokio::sync::Semaphore>,
}

impl ProviderHttpConfig {
    fn from_global(config: &crate::config::GlobalConfig) -> Self {
        let max_concurrency = config.core.max_concurrent_downloads;
        Self {
            timeout: std::time::Duration::from_secs(config.network.timeout),
            max_retries: config.network.max_retries,
            proxy: config.network.proxy.clone(),
            max_concurrency,
            download_limiter: std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrency)),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        Self::from_global(&crate::config::GlobalConfig::default())
    }

    #[cfg(test)]
    pub(crate) fn test_with_max_concurrency(max_concurrency: usize) -> Self {
        let mut config = crate::config::GlobalConfig::default();
        config.core.max_concurrent_downloads = max_concurrency;
        Self::from_global(&config)
    }
}

/// 根据配置创建 provider 列表。顺序只影响无限定搜索的展示顺序。
pub fn create_providers(
    catalogs: &[String],
    config: &crate::config::GlobalConfig,
) -> Result<Vec<Box<dyn ModProvider>>, crate::error::OrbitError> {
    let ua = format!("orbit/{}", env!("CARGO_PKG_VERSION"));
    let http = ProviderHttpConfig::from_global(config);
    let mut providers: Vec<Box<dyn ModProvider>> = Vec::new();
    for name in catalogs {
        match name.as_str() {
            "modrinth" => {
                providers.push(Box::new(modrinth::ModrinthProvider::new(
                    &ua,
                    config.auth.modrinth_token.as_deref(),
                    &http,
                )?) as Box<dyn ModProvider>);
            }
            "curseforge" => {
                let api_key = config
                    .auth
                    .curseforge_api_key
                    .as_deref()
                    .filter(|key| !key.trim().is_empty())
                    .ok_or(crate::error::OrbitError::ProviderApiKeyRequired {
                        provider: "CurseForge",
                        environment_variable: "ORBIT_CURSEFORGE_API_KEY",
                        config_key: "auth.curseforge_api_key",
                    })?;
                providers.push(
                    Box::new(curseforge::CurseForgeProvider::new(api_key, &ua, &http)?)
                        as Box<dyn ModProvider>,
                );
            }
            other => {
                return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                    "unsupported provider '{other}' in [resolver].catalogs"
                )));
            }
        }
    }
    if providers.is_empty() {
        return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
            "no valid provider catalogs configured in [resolver].catalogs"
        )));
    }
    Ok(providers)
}

/// Create every provider that can safely participate in source identification
/// before a manifest exists. Modrinth is always available; authenticated
/// providers are added only when their credentials are configured.
pub fn create_identification_providers(
    config: &crate::config::GlobalConfig,
) -> Result<Vec<Box<dyn ModProvider>>, crate::error::OrbitError> {
    let mut catalogs = vec!["modrinth".to_string()];
    if config
        .auth
        .curseforge_api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        catalogs.push("curseforge".to_string());
    }
    create_providers(&catalogs, config)
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
}

/// CurseForge 平台专属字段
#[derive(Debug, Clone)]
pub struct CurseForgeResolvedInfo {
    pub project_id: u32,
    pub file_id: u32,
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

/// Provider 返回的可下载 artifact locator。
///
/// 这里故意不包含 mod ID、模组版本、依赖、运行环境或 provides。这些
/// package metadata 只能在下载后从 JAR loader metadata 中取得。
#[derive(Debug, Clone)]
pub struct RemoteArtifact {
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
    /// 下载 URL
    pub download_url: String,
    /// jar 文件名
    pub filename: String,
    /// 仅用于继续定位可能相关的下载项目，绝不进入依赖图。
    pub related_projects: Vec<RemoteProjectLocator>,
}

impl RemoteArtifact {
    pub fn package_remote(&self) -> Result<crate::manifest::PackageRemote, OrbitError> {
        if let Some(metadata) = &self.modrinth {
            return Ok(crate::manifest::PackageRemote::Modrinth {
                project_id: metadata.project_id.clone(),
            });
        }
        if let Some(metadata) = &self.curseforge {
            return Ok(crate::manifest::PackageRemote::Curseforge {
                project_id: metadata.project_id,
            });
        }
        Err(OrbitError::Other(anyhow::anyhow!(
            "{} artifact '{}' has no project identity",
            self.provider,
            self.filename
        )))
    }

    pub fn artifact_source(&self) -> Result<crate::lockfile::ArtifactSource, OrbitError> {
        if let Some(metadata) = &self.modrinth {
            return Ok(crate::lockfile::ArtifactSource::Modrinth {
                project_id: metadata.project_id.clone(),
                version_id: metadata.version_id.clone(),
                download_url: self.download_url.clone(),
            });
        }
        if let Some(metadata) = &self.curseforge {
            return Ok(crate::lockfile::ArtifactSource::Curseforge {
                project_id: metadata.project_id,
                file_id: metadata.file_id,
                download_url: self.download_url.clone(),
            });
        }
        Err(OrbitError::Other(anyhow::anyhow!(
            "{} artifact '{}' has no downloadable source",
            self.provider,
            self.filename
        )))
    }

    /// Human-readable provider label for a concrete artifact. Provider ids and
    /// the content-hash candidate identity stay outside presentation output.
    pub fn display_source(&self) -> String {
        if self.modrinth.is_some() {
            return "Modrinth".to_string();
        }
        if self.curseforge.is_some() {
            return "CurseForge".to_string();
        }
        self.provider.clone()
    }

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

    pub fn matches_artifact(&self, artifact: &ArtifactFingerprint) -> bool {
        (!self.sha512.is_empty() && self.sha512.eq_ignore_ascii_case(&artifact.sha512))
            || (!self.sha1.is_empty() && self.sha1.eq_ignore_ascii_case(&artifact.sha1))
            || self.curseforge.as_ref().is_some_and(|metadata| {
                metadata.fingerprint != 0 && metadata.fingerprint == artifact.curseforge
            })
    }
}

/// Provider 给出的远端项目定位提示。其身份和依赖语义都不可信；
/// 下载后的 JAR metadata 决定该 artifact 实际提供什么。
#[derive(Debug, Clone)]
pub struct RemoteProjectLocator {
    pub slug: Option<String>,
    pub project_id: Option<String>,
}

/// 仅供远端目录的 `info` 输出使用，不参与安装或求解。
#[derive(Debug, Clone)]
pub struct CatalogDependency {
    pub slug: Option<String>,
    pub required: bool,
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
    pub project_id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub latest_version: String,
    pub downloads: u64,
    pub mc_versions: Vec<String>,
    pub client_side: Option<SideSupport>,
    pub server_side: Option<SideSupport>,
    pub categories: Vec<String>,
    /// Provider-hosted project icon used only for presentation. It never
    /// participates in artifact identity or dependency resolution.
    pub icon_url: Option<String>,
    /// Provider-generated RGB accent color (`0xRRGGBB`).
    pub accent_color: Option<u32>,
}

/// Provider-hosted project image used by presentation clients.
#[derive(Debug, Clone)]
pub struct ProjectImage {
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
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
    pub icon_url: Option<String>,
    pub accent_color: Option<u32>,
    pub website_url: Option<String>,
    pub source_url: Option<String>,
    pub issues_url: Option<String>,
    pub wiki_url: Option<String>,
    pub gallery: Vec<ProjectImage>,
    pub recent_versions: Vec<ModVersionInfo>,
    pub dependencies: Vec<CatalogDependency>,
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

    /// Provider-owned artifact transport. Authentication stays in this runtime
    /// client and is never copied into resolved metadata or lockfiles.
    fn artifact_downloader(&self) -> &ArtifactDownloadClient;

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
    ) -> Result<Vec<RemoteArtifact>, OrbitError> {
        Ok(Vec::new())
    }

    /// 获取模组的所有版本列表
    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<RemoteArtifact>, OrbitError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GlobalConfig;

    #[test]
    fn curseforge_requires_an_explicit_api_key() {
        let error = create_providers(&["curseforge".to_string()], &GlobalConfig::default())
            .err()
            .expect("missing key should fail");
        assert!(matches!(
            error,
            OrbitError::ProviderApiKeyRequired {
                provider: "CurseForge",
                ..
            }
        ));
    }

    #[test]
    fn curseforge_rejects_a_whitespace_only_api_key() {
        let mut config = GlobalConfig::default();
        config.auth.curseforge_api_key = Some("   ".to_string());
        let error = create_providers(&["curseforge".to_string()], &config)
            .err()
            .expect("blank key should fail");
        assert!(matches!(
            error,
            OrbitError::ProviderApiKeyRequired {
                provider: "CurseForge",
                ..
            }
        ));
    }

    #[test]
    fn provider_order_is_preserved_when_curseforge_is_enabled() {
        let mut config = GlobalConfig::default();
        config.auth.curseforge_api_key = Some("test-key".to_string());
        let providers =
            create_providers(&["modrinth".to_string(), "curseforge".to_string()], &config).unwrap();
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.name())
                .collect::<Vec<_>>(),
            vec!["modrinth", "curseforge"]
        );
    }
}
