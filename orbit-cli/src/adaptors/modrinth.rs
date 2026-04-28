use super::{ModProvider, ModSource, UnifiedMod, ModVersion, ModFile};
use modrinth_wrapper::{Client as MRClient, models as mr_models};
use modrinth_wrapper::models::ProjectInfo;
use std::collections::HashMap;

// Convert wrapper `Version` -> local `ModVersion`
fn convert_version(v: mr_models::Version) -> ModVersion {
    let files = v.files.into_iter().map(|f| {
        let mut hashes = HashMap::new();
        hashes.insert("sha1".to_string(), f.hashes.sha1);
        hashes.insert("sha512".to_string(), f.hashes.sha512);
        ModFile {
            filename: f.filename,
            url: f.url,
            hashes,
        }
    }).collect();

    ModVersion {
        id: v.id,
        project_id: v.project_id,
        version_number: v.version_number.unwrap_or_default(),
        date_published: v.date_published,
        files,
    }
}

pub struct ModrinthProvider {
    client: MRClient,
}

impl ModrinthProvider {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // wrapper Client::new is synchronous and returns its own Result type
        let client = MRClient::new("orbit").map_err(|e| Box::<dyn std::error::Error>::from(e))?;
        Ok(Self { client })
    }
}

impl ModProvider for ModrinthProvider {
    async fn search(
        &self,
        query: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
        _category: Option<&str>,
        offset: usize,
        limit: usize,
        _sort_type: super::SortType,
        _sort_order: super::SortOrder,
    ) -> Result<super::SearchResult, Box<dyn std::error::Error>> {
        // wrapper provides `search_projects` returning `SearchResult`
        let q = query.to_string();
        let res: mr_models::SearchResult = self.client.search_projects(&q).await?;

        let results = res.hits.into_iter().map(|h| UnifiedMod {
            id: h.get_id().to_string(),
            name: h.get_title().unwrap_or_default().to_string(),
            summary: h.get_description().unwrap_or_default().to_string(),
            source: ModSource::Modrinth,
        }).collect();

        Ok(super::SearchResult {
            results,
            total_count: res.total_hits as usize,
            offset,
            limit,
        })
    }

    async fn get_mod(&self, id: &str) -> Result<Option<UnifiedMod>, Box<dyn std::error::Error>> {
        let id_str = id.to_string();
        let parsed: mr_models::Project = match self.client.get_project(&id_str).await {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };

        Ok(Some(UnifiedMod {
            id: parsed.id,
            name: parsed.title.unwrap_or_default(),
            summary: parsed.description.unwrap_or_default(),
            source: ModSource::Modrinth,
        }))
    }

    async fn get_version_by_hash(
        &self,
        hash: &str,
        _algorithm: &str,
    ) -> Result<Option<ModVersion>, Box<dyn std::error::Error>> {
        // wrapper provides get_version_from_hash
        match self.client.get_version_from_hash(hash).await {
            Ok(v) => Ok(Some(convert_version(v))),
            Err(_) => Ok(None),
        }
    }

    async fn get_versions(
        &self,
        mod_id: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
    ) -> Result<Vec<ModVersion>, Box<dyn std::error::Error>> {
        let parsed: Vec<mr_models::Version> = match self.client.list_versions(mod_id).await {
            Ok(vs) => vs,
            Err(_) => return Ok(vec![]),
        };

        Ok(parsed.into_iter().map(convert_version).collect())
    }

    async fn get_categories(&self) -> Result<Vec<super::Category>, Box<dyn std::error::Error>> {
        Ok(vec![])
    }

    async fn resolve_dependency(&self, _id: &str) -> Result<Option<UnifiedMod>, Box<dyn std::error::Error>> {
        self.get_mod(_id).await
    }
}
