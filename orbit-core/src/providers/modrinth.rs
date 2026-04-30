use async_trait::async_trait;
use modrinth_wrapper::Client;

use super::{
    ModInfo, ModProvider, ModVersionInfo, ResolvedDependency, ResolvedMod, SearchResultItem,
    SideSupport,
};
use crate::error::OrbitError;

pub struct ModrinthProvider {
    client: Client,
}

impl ModrinthProvider {
    pub fn new(user_agent: &str) -> Result<Self, OrbitError> {
        let client = Client::new(user_agent)
            .map_err(|e| OrbitError::Other(e.into()))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl ModProvider for ModrinthProvider {
    fn name(&self) -> &'static str {
        "modrinth"
    }

    async fn search(
        &self,
        _query: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<SearchResultItem>, OrbitError> {
        // TODO: Phase 2 — 对接 modrinth_wrapper::Client::search_projects
        todo!("ModrinthProvider::search")
    }

    async fn get_mod_info(&self, _slug: &str) -> Result<ModInfo, OrbitError> {
        todo!("ModrinthProvider::get_mod_info")
    }

    async fn resolve(
        &self,
        _slug: &str,
        _version_constraint: &str,
        _mc_version: &str,
        _loader: &str,
    ) -> Result<ResolvedMod, OrbitError> {
        todo!("ModrinthProvider::resolve")
    }

    async fn get_version_by_hash(&self, _hash: &str) -> Result<Option<ResolvedMod>, OrbitError> {
        todo!("ModrinthProvider::get_version_by_hash")
    }

    async fn get_versions(
        &self,
        _slug: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        todo!("ModrinthProvider::get_versions")
    }

    async fn get_categories(&self) -> Result<Vec<String>, OrbitError> {
        todo!("ModrinthProvider::get_categories")
    }
}
