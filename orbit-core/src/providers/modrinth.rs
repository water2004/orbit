use async_trait::async_trait;
use modrinth_wrapper::{Client as MRClient, models as mr_models};

use super::{ModInfo, ModProvider, ResolvedDependency, ResolvedMod, SearchResultItem, SideSupport};
use crate::error::OrbitError;

pub struct ModrinthProvider {
    client: MRClient,
}

impl ModrinthProvider {
    pub fn new(user_agent: &str) -> Result<Self, OrbitError> {
        let client = MRClient::new(user_agent)
            .map_err(|e| OrbitError::Other(e.into()))?;
        Ok(Self { client })
    }
}

fn map_side(side: Option<&str>) -> Option<SideSupport> {
    match side {
        Some("required") => Some(SideSupport::Required),
        Some("optional") => Some(SideSupport::Optional),
        Some("unsupported") => Some(SideSupport::Unsupported),
        _ => None,
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
        _mc_version: Option<&str>,
        _loader: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResultItem>, OrbitError> {
        let res: mr_models::SearchResult = self
            .client
            .search_projects(query)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        Ok(res.hits.into_iter().take(limit).map(|hit| SearchResultItem {
            mod_id: hit.project_id,
            slug: hit.slug,
            name: hit.title,
            description: hit.description,
            latest_version: hit.latest_version.unwrap_or_default(),
            downloads: hit.downloads as u64,
            mc_versions: hit.versions,
            client_side: map_side(hit.client_side.as_deref()),
            server_side: map_side(hit.server_side.as_deref()),
            categories: hit.categories,
        }).collect())
    }

    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError> {
        let project: mr_models::Project = self
            .client
            .get_project(slug)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        Ok(ModInfo {
            slug: project.slug.unwrap_or_else(|| project.id.clone()),
            name: project.title.unwrap_or_default(),
            description: project.description.unwrap_or_default(),
            authors: vec![],
            latest_version: project.latest_version.unwrap_or_default(),
            downloads: project.downloads as u64,
            license: project.license.map(|l| l.id),
            client_side: map_side(project.client_side.as_deref()),
            server_side: map_side(project.server_side.as_deref()),
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
        let versions = self
            .client
            .list_versions(slug)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        let candidate = versions
            .iter()
            .filter(|v| v.game_versions.iter().any(|gv| gv == mc_version))
            .filter(|v| v.loaders.iter().any(|l| l == loader))
            .max_by_key(|v| v.date_published.clone());

        match candidate {
            Some(v) => {
                let file = v.files.first().ok_or_else(|| OrbitError::ModNotFound(slug.to_string()))?;
                Ok(ResolvedMod {
                    name: slug.to_string(),
                    mod_id: v.project_id.clone(),
                    version: v.version_number.clone().unwrap_or_default(),
                    download_url: file.url.clone(),
                    filename: file.filename.clone(),
                    sha256: file.hashes.sha512.clone(),
                    dependencies: v.dependencies.iter().map(|d| ResolvedDependency {
                        name: d.project_id.clone().unwrap_or_default(),
                        slug: d.project_id.clone(),
                        required: d.dependency_type.as_deref() == Some("required"),
                    }).collect(),
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

    async fn get_version_by_hash(&self, hash: &str) -> Result<Option<ResolvedMod>, OrbitError> {
        match self.client.get_version_from_hash(hash).await {
            Ok(v) => {
                let file = v.files.first();
                Ok(Some(ResolvedMod {
                    name: v.project_id.clone(),
                    mod_id: v.project_id.clone(),
                    version: v.version_number.clone().unwrap_or_default(),
                    download_url: file.map(|f| f.url.clone()).unwrap_or_default(),
                    filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
                    sha256: file.map(|f| f.hashes.sha512.clone()).unwrap_or_default(),
                    dependencies: vec![],
                    client_side: None,
                    server_side: None,
                }))
            }
            Err(_) => Ok(None),
        }
    }

    async fn get_versions(
        &self,
        slug: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        let versions = self
            .client
            .list_versions(slug)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        Ok(versions.iter().map(|v| {
            let file = v.files.first();
            ResolvedMod {
                name: slug.to_string(),
                mod_id: v.project_id.clone(),
                version: v.version_number.clone().unwrap_or_default(),
                download_url: file.map(|f| f.url.clone()).unwrap_or_default(),
                filename: file.map(|f| f.filename.clone()).unwrap_or_default(),
                sha256: file.map(|f| f.hashes.sha512.clone()).unwrap_or_default(),
                dependencies: vec![],
                client_side: None,
                server_side: None,
            }
        }).collect())
    }

    async fn get_categories(&self) -> Result<Vec<String>, OrbitError> {
        Ok(vec![])
    }
}
