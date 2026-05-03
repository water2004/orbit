use async_trait::async_trait;
use modrinth_wrapper::{Client as MRClient, models as mr_models};
use modrinth_wrapper::api::SearchParams;
use std::path::{Path, PathBuf};

use super::{ModInfo, ModProvider, ProgressCallback, ResolvedDependency, ResolvedMod, SearchResultItem, SideSupport};
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

    /// 下载模组 JAR 到指定目录。
    /// 先写入 .tmp → SHA-256 校验 → rename 为正式文件名。
    pub async fn download_to(
        &self,
        url: &str,
        expected_sha256: &str,
        dest_dir: &Path,
        filename: &str,
        on_progress: Option<&ProgressCallback>,
    ) -> Result<PathBuf, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;

        let tmp_path = dest_dir.join(format!(".{filename}.tmp"));
        let final_path = dest_dir.join(filename);

        let bytes = reqwest::get(url).await?.bytes().await?;
        let total = bytes.len() as u64;
        if let Some(cb) = on_progress {
            cb(total, total);
        }

        let actual = crate::jar::sha256_digest(&bytes);
        if actual != expected_sha256 {
            std::fs::remove_file(&tmp_path).ok();
            return Err(OrbitError::ChecksumMismatch {
                name: filename.to_string(),
                expected: expected_sha256.to_string(),
                actual,
            });
        }

        std::fs::write(&tmp_path, &bytes)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(final_path)
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
        let _permit = self.rate_limiter.acquire().await;
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
        let _permit = self.rate_limiter.acquire().await;
        let project: mr_models::Project = self
            .client
            .get_project(slug)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        Ok(ModInfo {
            slug: project.slug.clone(),
            name: project.title,
            description: project.description,
            authors: vec![],
            latest_version: project.versions.as_deref().and_then(|v| v.last()).cloned().unwrap_or_default(),
            downloads: project.downloads as u64,
            license: project.license.map(|l| l.id),
            client_side: map_side(&project.client_side),
            server_side: map_side(&project.server_side),
            categories: project.categories,
            recent_versions: vec![],
            dependencies: vec![],
        })
    }

    async fn resolve(
        &self,
        slug: &str,
        version_constraint: &str,
        mc_version: &str,
        loader: &str,
    ) -> Result<ResolvedMod, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;
        let versions = self
            .client
            .list_versions(slug)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        let candidate = versions
            .iter()
            .filter(|v| v.game_versions.iter().any(|g| g == mc_version))
            .filter(|v| v.loaders.iter().any(|l| l == loader))
            .max_by_key(|v| v.date_published.clone());

        match candidate {
            Some(v) => {
                let file = v.files.first().ok_or_else(|| OrbitError::ModNotFound(slug.to_string()))?;
                let deps = v.dependencies.as_ref().map(|deps| deps.iter().map(|d| ResolvedDependency {
                    name: d.project_id.clone().unwrap_or_default(),
                    slug: d.project_id.clone(),
                    required: d.dependency_type == "required",
                }).collect()).unwrap_or_default();
                Ok(ResolvedMod {
                    name: slug.to_string(),
                    mod_id: v.project_id.clone(),
                    version: v.version_number.clone(),
                    download_url: file.url.clone(),
                    filename: file.filename.clone(),
                    sha256: file.hashes.sha512.clone(),
                    dependencies: deps,
                    client_side: None,
                    server_side: None,
                })
            }
            None => Err(OrbitError::VersionMismatch {
                mod_name: slug.to_string(),
                constraint: version_constraint.to_string(),
            }),
        }
    }

    async fn get_versions(
        &self,
        slug: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;
        eprintln!("    [modrinth] get_versions(slug={slug})");
        let versions = self
            .client
            .list_versions(slug)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;
        eprintln!("    [modrinth]   → {} versions", versions.len());

        Ok(versions.iter().map(|v| {
            let file = v.files.first();
            let deps = v.dependencies.as_ref().map(|deps| deps.iter().map(|d| ResolvedDependency {
                name: d.project_id.clone().unwrap_or_default(),
                slug: d.project_id.clone(),
                required: d.dependency_type == "required",
            }).collect()).unwrap_or_default();
            ResolvedMod {
                name: slug.to_string(),
                mod_id: v.project_id.clone(),
                version: v.version_number.clone(),
                download_url: file.map(|f| f.url.clone()).unwrap_or_default(),
                filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
                sha256: file.map(|f| f.hashes.sha512.clone()).unwrap_or_default(),
                dependencies: deps,
                client_side: None,
                server_side: None,
            }
        }).collect())
    }

    async fn get_categories(&self) -> Result<Vec<String>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;
        Ok(vec![])
    }

    async fn fetch_dependencies(&self, project_id: &str) -> Result<Vec<ResolvedDependency>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;
        eprintln!("    [modrinth] fetch_dependencies({project_id})");
        let deps = self.client.get_project_dependencies(project_id).await
            .map_err(|e| OrbitError::Other(e.into()))?;
        eprintln!("    [modrinth]   → {} projects, {} versions", deps.projects.len(), deps.versions.len());
        for p in &deps.projects {
            eprintln!("    [modrinth]     project: id={} slug={} title={}", p.id, p.slug, p.title);
        }
        Ok(deps.projects.into_iter().map(|p| ResolvedDependency {
            name: p.title.clone(),
            slug: Some(p.slug),
            required: true,
        }).collect())
    }

    async fn get_version_by_hash(&self, hash: &str) -> Result<Option<ResolvedMod>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;
        eprintln!("    [modrinth] get_version_by_hash(sha512={:.16}...)", &hash[..16]);
        match self.client.get_version_from_hash(hash, Some("sha512"), None).await {
            Ok(v) => {
                let ver = v.version_number.clone();
                eprintln!("    [modrinth]   → id={} project_id={} version={ver}", v.id, v.project_id);
                if let Some(ref raw_deps) = v.dependencies {
                    for d in raw_deps {
                        eprintln!("    [modrinth]     dep: type={} project_id={:?} version_id={:?}", d.dependency_type, d.project_id, d.version_id);
                    }
                }
                let file = v.files.first();
                let deps = v.dependencies.unwrap_or_default().into_iter().map(|d| ResolvedDependency {
                    name: d.project_id.clone().unwrap_or_default(),
                    slug: d.project_id.clone(),
                    required: d.dependency_type == "required",
                }).collect();
                Ok(Some(ResolvedMod {
                    name: v.project_id.clone(),
                    mod_id: v.project_id.clone(),
                    version: ver,
                    download_url: file.map(|f| f.url.clone()).unwrap_or_default(),
                    filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
                    sha256: file.map(|f| f.hashes.sha512.clone()).unwrap_or_default(),
                    dependencies: deps,
                    client_side: None,
                    server_side: None,
                }))
            }
            Err(e) => {
                eprintln!("    [modrinth]   → not found ({e})");
                Ok(None)
            }
        }
    }
}
