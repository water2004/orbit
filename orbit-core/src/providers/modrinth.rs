use async_trait::async_trait;
use modrinth_wrapper::api::SearchParams;
use modrinth_wrapper::{Client as MRClient, models as mr_models};
use std::collections::HashMap;

use super::rate_limiter::RateLimiter;
use super::{
    ArtifactDownloadClient, ArtifactFingerprint, CatalogDependency, ModInfo, ModProvider,
    ModrinthResolvedInfo, RemoteArtifact, RemoteProjectLocator, SearchResultItem, SideSupport,
};
use crate::error::OrbitError;

pub struct ModrinthProvider {
    client: MRClient,
    downloader: ArtifactDownloadClient,
    rate_limiter: RateLimiter,
}

impl ModrinthProvider {
    pub fn new(user_agent: &str, max_concurrency: usize) -> Result<Self, OrbitError> {
        let client = MRClient::new(user_agent).map_err(|e| OrbitError::Other(e.into()))?;
        Ok(Self {
            client,
            downloader: ArtifactDownloadClient::anonymous(user_agent)?,
            rate_limiter: RateLimiter::new(max_concurrency),
        })
    }

    /// 批量查询项目 ID → slug 映射（内部方法，不获取 rate_limiter permit，由调用方控制并发）
    async fn lookup_project_slugs(
        &self,
        ids: &[&str],
    ) -> Result<HashMap<String, String>, OrbitError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.client
            .get_projects(ids)
            .await
            .map(|projects| projects.into_iter().map(|p| (p.id, p.slug)).collect())
            .map_err(|error| OrbitError::Other(error.into()))
    }

    async fn lookup_project_dependencies(
        &self,
        project_id: &str,
    ) -> Result<Vec<CatalogDependency>, OrbitError> {
        let dependencies = self
            .client
            .get_project_dependencies(project_id)
            .await
            .map_err(|error| OrbitError::Other(error.into()))?;
        Ok(dependencies
            .projects
            .into_iter()
            .map(|project| CatalogDependency {
                slug: Some(project.slug),
                required: true,
                project_id: Some(project.id),
            })
            .collect())
    }

    async fn dependency_version_projects(
        &self,
        versions: &[mr_models::Version],
    ) -> Result<HashMap<String, String>, OrbitError> {
        let version_ids: Vec<&str> = versions
            .iter()
            .flat_map(|version| version.dependencies.as_deref().unwrap_or(&[]))
            .filter(|dependency| dependency.project_id.is_none())
            .filter_map(|dependency| dependency.version_id.as_deref())
            .collect();
        if version_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.client
            .get_versions_by_ids(&version_ids)
            .await
            .map(|versions| {
                versions
                    .into_iter()
                    .map(|version| (version.id, version.project_id))
                    .collect()
            })
            .map_err(|error| OrbitError::Other(error.into()))
    }
}

/// 将 Modrinth API 错误转为 OrbitError，404 → ModNotFound
fn map_api_error(e: modrinth_wrapper::ModrinthError, slug: &str) -> OrbitError {
    use modrinth_wrapper::ModrinthError;
    match &e {
        ModrinthError::Reqwest(req_err)
            if req_err.status() == Some(reqwest::StatusCode::NOT_FOUND) =>
        {
            OrbitError::ModNotFound(slug.to_string())
        }
        _ => OrbitError::Other(e.into()),
    }
}

fn map_side(side: &str) -> Option<SideSupport> {
    match side {
        "required" => Some(SideSupport::Required),
        "optional" => Some(SideSupport::Optional),
        "unsupported" => Some(SideSupport::Unsupported),
        _ => None,
    }
}

fn build_facets(mc_version: Option<&str>, loader: Option<&str>) -> Option<String> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    if let Some(mc) = mc_version {
        groups.push(vec![format!("versions:{mc}")]);
    }
    if let Some(l) = loader {
        groups.push(vec![format!("categories:{l}")]);
    }
    if groups.is_empty() {
        None
    } else {
        serde_json::to_string(&groups).ok()
    }
}

fn primary_jar(version: &mr_models::Version) -> Option<&mr_models::VersionFile> {
    version
        .files
        .iter()
        .find(|file| file.primary && file.filename.ends_with(".jar"))
        .or_else(|| {
            version
                .files
                .iter()
                .find(|file| file.filename.ends_with(".jar"))
        })
}

#[async_trait]
impl ModProvider for ModrinthProvider {
    fn name(&self) -> &'static str {
        "modrinth"
    }

    fn artifact_downloader(&self) -> &ArtifactDownloadClient {
        &self.downloader
    }

    async fn search(
        &self,
        query: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResultItem>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let facets = build_facets(mc_version, loader);
        let mut params = SearchParams::new(query).limit(limit as i64);
        if let Some(ref f) = facets {
            params = params.facets(f.clone());
        }
        let res: mr_models::SearchResult = self
            .client
            .search(params)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        Ok(res
            .hits
            .into_iter()
            .map(|hit| SearchResultItem {
                project_id: hit.project_id,
                slug: hit.slug,
                name: hit.title,
                description: hit.description,
                latest_version: hit.latest_version.unwrap_or_default(),
                downloads: hit.downloads as u64,
                mc_versions: hit.versions,
                client_side: map_side(&hit.client_side),
                server_side: map_side(&hit.server_side),
                categories: hit.categories.unwrap_or_default(),
            })
            .collect())
    }

    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let project: mr_models::Project = self
            .client
            .get_project(slug)
            .await
            .map_err(|e| map_api_error(e, slug))?;

        // Fetch recent versions for a richer display
        let recent: Vec<super::ModVersionInfo> = self
            .client
            .list_versions_with_params(
                &project.slug,
                modrinth_wrapper::api::ListVersionsParams::new().include_changelog(false),
            )
            .await
            .map_err(|error| OrbitError::Other(error.into()))?
            .into_iter()
            .take(5)
            .map(|v| super::ModVersionInfo {
                version: v.version_number,
                mc_versions: v.game_versions,
                loader: v.loaders.first().cloned().unwrap_or_default(),
                released_at: v.date_published,
            })
            .collect();

        let dependencies = self.lookup_project_dependencies(&project.id).await?;

        Ok(ModInfo {
            project_id: project.id,
            slug: project.slug.clone(),
            name: project.title,
            description: project.description,
            authors: vec![],
            latest_version: recent
                .first()
                .map(|v| v.version.clone())
                .unwrap_or_default(),
            downloads: project.downloads as u64,
            license: project.license.map(|l| l.id),
            client_side: map_side(&project.client_side),
            server_side: map_side(&project.server_side),
            categories: project.categories,
            recent_versions: recent,
            dependencies,
        })
    }

    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<RemoteArtifact>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let mut params = modrinth_wrapper::api::ListVersionsParams::new().include_changelog(false);
        if let Some(l) = loader {
            params = params.loaders(&[l]);
        }
        if let Some(mc) = mc_version {
            params = params.game_versions(&[mc]);
        }
        let versions = self
            .client
            .list_versions_with_params(slug, params)
            .await
            .map_err(|e| map_api_error(e, slug))?;
        let version_projects = self.dependency_version_projects(&versions).await?;

        let artifacts: Vec<_> = versions
            .iter()
            .filter_map(|v| {
                let file = primary_jar(v)?;
                let deps = v
                    .dependencies
                    .as_ref()
                    .map(|deps| {
                        deps.iter()
                            .filter_map(|d| {
                                let project_id = d.project_id.clone().or_else(|| {
                                    d.version_id
                                        .as_ref()
                                        .and_then(|id| version_projects.get(id).cloned())
                                });
                                project_id.map(|project_id| RemoteProjectLocator {
                                    slug: None,
                                    project_id: Some(project_id),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(RemoteArtifact {
                    sha1: file.hashes.sha1.clone(),
                    sha512: file.hashes.sha512.clone(),
                    slug: slug.to_string(),
                    provider: "modrinth".to_string(),
                    modrinth: Some(ModrinthResolvedInfo {
                        project_id: v.project_id.clone(),
                        version_id: v.id.clone(),
                    }),
                    curseforge: None,
                    download_url: file.url.clone(),
                    filename: file.filename.clone(),
                    related_projects: deps,
                })
            })
            .collect();
        if let Some(artifact) = artifacts
            .iter()
            .find(|artifact| artifact.sha512.is_empty() || artifact.download_url.is_empty())
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "Modrinth project '{}' returned JAR '{}' without a SHA-512 checksum or download URL",
                slug,
                artifact.filename
            )));
        }
        Ok(artifacts)
    }

    async fn identify_artifacts(
        &self,
        artifacts: &[ArtifactFingerprint],
    ) -> Result<Vec<RemoteArtifact>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        if artifacts.is_empty() {
            return Ok(vec![]);
        }
        let strs: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.sha512.as_str())
            .filter(|hash| !hash.is_empty())
            .collect();
        if strs.is_empty() {
            return Ok(vec![]);
        }
        let map = self
            .client
            .get_versions_from_hashes(&strs, Some("sha512"))
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;
        let matched_versions: Vec<_> = map.values().cloned().collect();
        let version_projects = self.dependency_version_projects(&matched_versions).await?;
        let all_ids: Vec<&str> = map.values().map(|v| v.project_id.as_str()).collect();
        let id_to_slug: HashMap<String, String> = self.lookup_project_slugs(&all_ids).await?;
        let requested_hashes: std::collections::HashSet<_> =
            strs.into_iter().map(str::to_ascii_lowercase).collect();
        Ok(map
            .into_values()
            .filter_map(|v| {
                let file = v.files.iter().find(|file| {
                    requested_hashes.contains(&file.hashes.sha512.to_ascii_lowercase())
                })?;
                let main_slug = id_to_slug
                    .get(&v.project_id)
                    .cloned()
                    .unwrap_or_else(|| v.project_id.clone());
                Some(RemoteArtifact {
                    sha1: file.hashes.sha1.clone(),
                    sha512: file.hashes.sha512.clone(),
                    slug: main_slug,
                    provider: "modrinth".to_string(),
                    modrinth: Some(ModrinthResolvedInfo {
                        project_id: v.project_id.clone(),
                        version_id: v.id.clone(),
                    }),
                    curseforge: None,
                    download_url: file.url.clone(),
                    filename: file.filename.clone(),
                    related_projects: v
                        .dependencies
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|dependency| {
                            let project_id = dependency.project_id.or_else(|| {
                                dependency
                                    .version_id
                                    .as_ref()
                                    .and_then(|id| version_projects.get(id).cloned())
                            });
                            project_id.map(|project_id| RemoteProjectLocator {
                                slug: None,
                                project_id: Some(project_id),
                            })
                        })
                        .collect(),
                })
            })
            .collect())
    }
}
