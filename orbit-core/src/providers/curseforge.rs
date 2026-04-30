use async_trait::async_trait;

use super::{
    ModInfo, ModProvider, ModVersionInfo, ResolvedDependency, ResolvedMod, SearchResultItem,
    SideSupport,
};
use crate::error::OrbitError;

pub struct CurseForgeProvider {
    api_key: String,
    // TODO Phase 2: 当 curseforge-wrapper 创建后，替换为 curseforge_wrapper::Client
    // client: curseforge_wrapper::Client,
}

impl CurseForgeProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl ModProvider for CurseForgeProvider {
    fn name(&self) -> &'static str {
        "curseforge"
    }

    async fn search(
        &self,
        _query: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<SearchResultItem>, OrbitError> {
        todo!("CurseForgeProvider::search")
    }

    async fn get_mod_info(&self, _slug: &str) -> Result<ModInfo, OrbitError> {
        todo!("CurseForgeProvider::get_mod_info")
    }

    async fn resolve(
        &self,
        _slug: &str,
        _version_constraint: &str,
        _mc_version: &str,
        _loader: &str,
    ) -> Result<ResolvedMod, OrbitError> {
        todo!("CurseForgeProvider::resolve")
    }

    async fn get_version_by_hash(&self, _hash: &str) -> Result<Option<ResolvedMod>, OrbitError> {
        todo!("CurseForgeProvider::get_version_by_hash")
    }

    async fn get_versions(
        &self,
        _slug: &str,
        _mc_version: Option<&str>,
        _loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        todo!("CurseForgeProvider::get_versions")
    }

    async fn get_categories(&self) -> Result<Vec<String>, OrbitError> {
        todo!("CurseForgeProvider::get_categories")
    }
}
