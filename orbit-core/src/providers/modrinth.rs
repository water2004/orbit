use std::collections::HashMap;
use async_trait::async_trait;
use modrinth_wrapper::{Client as MRClient, models as mr_models};
use modrinth_wrapper::api::SearchParams;

use super::{ModInfo, ModProvider, ModrinthResolvedInfo, ResolvedDependency, ResolvedMod, SearchResultItem, SideSupport};
use super::rate_limiter::RateLimiter;
use crate::error::OrbitError;

pub struct ModrinthProvider {
    client: MRClient,
    rate_limiter: RateLimiter,
}

impl ModrinthProvider {
    pub fn new(user_agent: &str, max_concurrency: usize) -> Result<Self, OrbitError> {
        let client = MRClient::new(user_agent)
            .map_err(|e| OrbitError::Other(e.into()))?;
        Ok(Self {
            client,
            rate_limiter: RateLimiter::new(max_concurrency),
        })
    }

    /// 批量查询项目 ID → slug 映射（内部方法，不获取 rate_limiter permit，由调用方控制并发）
    async fn lookup_project_slugs(&self, ids: &[&str]) -> HashMap<String, String> {
        if ids.is_empty() {
            return HashMap::new();
        }
        match self.client.get_projects(ids).await {
            Ok(projects) => projects.into_iter().map(|p| (p.id, p.slug)).collect(),
            Err(_) => HashMap::new(),
        }
    }

}

/// 将 Modrinth API 错误转为 OrbitError，404 → ModNotFound
fn map_api_error(e: modrinth_wrapper::ModrinthError, slug: &str) -> OrbitError {
    use modrinth_wrapper::ModrinthError;
    match &e {
        ModrinthError::Reqwest(req_err) if req_err.status() == Some(reqwest::StatusCode::NOT_FOUND) => {
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

#[async_trait]
impl ModProvider for ModrinthProvider {
    fn name(&self) -> &'static str {
        "modrinth"
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

        Ok(res.hits.into_iter().map(|hit| SearchResultItem {
            mod_id: hit.project_id,
            slug: hit.slug,
            name: hit.title,
            description: hit.description,
            latest_version: hit.latest_version.unwrap_or_default(),
            downloads: hit.downloads as u64,
            mc_versions: hit.versions,
            client_side: map_side(&hit.client_side),
            server_side: map_side(&hit.server_side),
            categories: hit.categories.unwrap_or_default(),
        }).collect())
    }

    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let project: mr_models::Project = self
            .client
            .get_project(slug)
            .await
            .map_err(|e| map_api_error(e, slug))?;

        // Fetch recent versions for a richer display
        let recent: Vec<super::ModVersionInfo> = self.client.list_versions_with_params(&project.slug,
            modrinth_wrapper::api::ListVersionsParams::new().include_changelog(false))
            .await
            .map(|versions| versions.into_iter().take(5).map(|v| super::ModVersionInfo {
                version: v.version_number,
                mc_versions: v.game_versions,
                loader: v.loaders.first().cloned().unwrap_or_default(),
                released_at: v.date_published,
            }).collect())
            .unwrap_or_default();

        Ok(ModInfo {
            slug: project.slug.clone(),
            name: project.title,
            description: project.description,
            authors: vec![],
            latest_version: recent.first().map(|v| v.version.clone()).unwrap_or_default(),
            downloads: project.downloads as u64,
            license: project.license.map(|l| l.id),
            client_side: map_side(&project.client_side),
            server_side: map_side(&project.server_side),
            categories: project.categories,
            recent_versions: recent,
            dependencies: vec![], // 需额外调用 get_project_dependencies
        })
    }

    async fn resolve(
        &self,
        slug: &str,
        version_constraint: &str,
        mc_version: &str,
        loader: &str,
    ) -> Result<ResolvedMod, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let versions = self
            .client
            .list_versions_with_params(slug,
                modrinth_wrapper::api::ListVersionsParams::new()
                    .loaders(&[loader])
                    .game_versions(&[mc_version])
                    .include_changelog(false))
            .await
            .map_err(|e| map_api_error(e, slug))?;

        let candidate = versions
            .iter()
            .filter(|v| version_constraint == "*" || version_constraint.is_empty()
                || crate::versions::fabric::SemanticVersion::parse(&v.version_number, true)
                    .map(|sv| crate::versions::fabric::satisfies(&sv, version_constraint))
                    .unwrap_or(false))
            .max_by_key(|v| v.date_published.clone());

        match candidate {
            Some(v) => {
                let file = v.files.first().ok_or_else(|| OrbitError::ModNotFound(slug.to_string()))?;

                // Resolve dependency project_ids → slugs via batch lookup
                let dep_ids: Vec<&str> = v.dependencies.as_ref()
                    .map(|deps| deps.iter().filter_map(|d| d.project_id.as_deref()).collect())
                    .unwrap_or_default();
                let id_to_slug: HashMap<String, String> = self.lookup_project_slugs(&dep_ids).await;

                let deps: Vec<ResolvedDependency> = v.dependencies.as_ref()
                    .map(|deps| deps.iter().map(|d| {
                        let pid = d.project_id.clone().unwrap_or_default();
                        let resolved_slug = id_to_slug.get(&pid).cloned();
                        ResolvedDependency {
                            slug: resolved_slug,
                            required: d.dependency_type == "required",
                        }
                    }).collect())
                    .unwrap_or_default();

                Ok(ResolvedMod {
                    mod_id: slug.to_string(),
                    version: v.version_number.clone(),
                    sha1: file.hashes.sha1.clone(),
                    sha512: file.hashes.sha512.clone(),
                    slug: slug.to_string(),
                    provider: "modrinth".to_string(),
                    modrinth: Some(ModrinthResolvedInfo {
                        project_id: v.project_id.clone(),
                        version_id: v.id.clone(),
                        version_number: v.version_number.clone(),
                    }),
                    date_published: v.date_published.clone(),
                    download_url: file.url.clone(),
                    filename: file.filename.clone(),
                    dependencies: deps,
                    client_side: None,
                    server_side: None,
                })
            }
            None => Err(OrbitError::ModNotFound(slug.to_string())),
        }
    }

    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let mut params = modrinth_wrapper::api::ListVersionsParams::new().include_changelog(false);
        if let Some(l) = loader { params = params.loaders(&[l]); }
        if let Some(mc) = mc_version { params = params.game_versions(&[mc]); }
        let versions = self
            .client
            .list_versions_with_params(slug, params)
            .await
            .map_err(|e| map_api_error(e, slug))?;

        // 收集所有依赖的 project_id → 批量查 slug
        let all_dep_ids: Vec<&str> = versions.iter()
            .flat_map(|v| v.dependencies.as_ref().map(|d| d.as_slice()).unwrap_or(&[]))
            .filter_map(|d| d.project_id.as_deref())
            .collect();
        let id_to_slug: HashMap<String, String> = self.lookup_project_slugs(&all_dep_ids).await;

        Ok(versions.iter().map(|v| {
            let file = v.files.first();
            let deps = v.dependencies.as_ref().map(|deps| deps.iter().map(|d| {
                let pid = d.project_id.clone().unwrap_or_default();
                let resolved_slug = id_to_slug.get(&pid).cloned();
                ResolvedDependency {
                    slug: resolved_slug,
                    required: d.dependency_type == "required",
                }
            }).collect()).unwrap_or_default();
            ResolvedMod {
                mod_id: slug.to_string(),
                version: v.version_number.clone(),
                sha1: file.map(|f| f.hashes.sha1.clone()).unwrap_or_default(),
                sha512: file.map(|f| f.hashes.sha512.clone()).unwrap_or_default(),
                slug: slug.to_string(),
                provider: "modrinth".to_string(),
                modrinth: Some(ModrinthResolvedInfo {
                    project_id: v.project_id.clone(),
                    version_id: v.id.clone(),
                    version_number: v.version_number.clone(),
                }),
                date_published: v.date_published.clone(),
                download_url: file.map(|f| f.url.clone()).unwrap_or_default(),
                filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
                dependencies: deps,
                client_side: None,
                server_side: None,
            }
        }).collect())
    }

    async fn get_versions_batch(
        &self,
        project_ids: &[String],
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        // 逐个调 get_versions（内部已按 mc_version + loader 过滤，每次返回版本数很少）
        let mut results = Vec::new();
        for pid in project_ids {
            if let Ok(versions) = self.get_versions(pid, mc_version, loader).await {
                results.extend(versions);
            }
        }
        Ok(results)
    }

    async fn get_categories(&self) -> Result<Vec<String>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        Ok(vec![])
    }

    async fn fetch_dependencies(&self, project_id: &str) -> Result<Vec<ResolvedDependency>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let deps = self.client.get_project_dependencies(project_id).await
            .map_err(|e| OrbitError::Other(e.into()))?;
        Ok(deps.projects.into_iter().map(|p| ResolvedDependency {
            slug: Some(p.slug),
            required: true,
        }).collect())
    }

    async fn get_versions_by_hashes(&self, hashes: &[String]) -> Result<Vec<ResolvedMod>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        if hashes.is_empty() { return Ok(vec![]); }
        let strs: Vec<&str> = hashes.iter().map(|s| s.as_str()).collect();
        let map = self.client.get_versions_from_hashes(&strs, Some("sha512")).await
            .map_err(|e| OrbitError::Other(e.into()))?;
        // 批量查 project_id → slug（包含主项目 + 依赖项目）
        let mut all_ids: Vec<&str> = map.values().map(|v| v.project_id.as_str()).collect();
        let dep_ids: Vec<&str> = map.values()
            .flat_map(|v| v.dependencies.as_ref().map(|d| d.as_slice()).unwrap_or(&[]))
            .filter_map(|d| d.project_id.as_deref())
            .collect();
        all_ids.extend(dep_ids);
        let id_to_slug: HashMap<String, String> = self.lookup_project_slugs(&all_ids).await;
        Ok(map.into_values().map(|v| {
            let file = v.files.first();
            let main_slug = id_to_slug.get(&v.project_id).cloned().unwrap_or_else(|| v.project_id.clone());
            ResolvedMod {
                mod_id: main_slug.clone(),
                version: v.version_number.clone(),
                sha1: file.map(|f| f.hashes.sha1.clone()).unwrap_or_default(),
                sha512: file.map(|f| f.hashes.sha512.clone()).unwrap_or_default(),
                slug: main_slug,
                provider: "modrinth".to_string(),
                modrinth: Some(ModrinthResolvedInfo {
                    project_id: v.project_id.clone(),
                    version_id: v.id.clone(),
                    version_number: v.version_number.clone(),
                }),
                date_published: v.date_published.clone(),
                download_url: file.map(|f| f.url.clone()).unwrap_or_default(),
                filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
                dependencies: v.dependencies.unwrap_or_default().into_iter().map(|d| {
                    let pid = d.project_id.clone().unwrap_or_default();
                    let resolved_slug = id_to_slug.get(&pid).cloned();
                    ResolvedDependency {
                        slug: resolved_slug,
                        required: d.dependency_type == "required",
                    }
                }).collect(),
                client_side: None, server_side: None,
            }
        }).collect())
    }

    async fn get_version_by_hash(&self, hash: &str) -> Result<Option<ResolvedMod>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        match self.client.get_version_from_hash(hash, Some("sha512"), None).await {
            Ok(v) => {
                let ver = v.version_number.clone();
                let file = v.files.first();
                let dep_ids: Vec<&str> = v.dependencies.as_ref().map(|d| d.iter().filter_map(|x| x.project_id.as_deref()).collect()).unwrap_or_default();
                let mut all_ids = dep_ids.clone();
                all_ids.push(&v.project_id);
                let id_to_slug: HashMap<String, String> = self.lookup_project_slugs(&all_ids).await;
                let main_slug = id_to_slug.get(&v.project_id).cloned().unwrap_or_else(|| v.project_id.clone());
                let deps = v.dependencies.unwrap_or_default().into_iter().map(|d| {
                    let pid = d.project_id.clone().unwrap_or_default();
                    let resolved_slug = id_to_slug.get(&pid).cloned();
                    ResolvedDependency {
                        slug: resolved_slug,
                        required: d.dependency_type == "required",
                    }
                }).collect();
                Ok(Some(ResolvedMod {
                    mod_id: main_slug.clone(),
                    version: ver,
                    sha1: file.map(|f| f.hashes.sha1.clone()).unwrap_or_default(),
                    sha512: file.map(|f| f.hashes.sha512.clone()).unwrap_or_default(),
                    slug: main_slug,
                    provider: "modrinth".to_string(),
                    modrinth: Some(ModrinthResolvedInfo {
                        project_id: v.project_id.clone(),
                        version_id: v.id.clone(),
                        version_number: v.version_number.clone(),
                    }),
                    date_published: v.date_published.clone(),
                    download_url: file.map(|f| f.url.clone()).unwrap_or_default(),
                    filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
                    dependencies: deps,
                    client_side: None,
                    server_side: None,
                }))
            }
            Err(_) => Ok(None),
        }
    }
}
