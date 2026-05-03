use async_trait::async_trait;

use super::{ModInfo, ModProvider, ResolvedMod, SearchResultItem};
use crate::error::OrbitError;

pub struct CurseForgeProvider {
    _api_key: String,
}

impl CurseForgeProvider {
    pub fn new(api_key: &str) -> Self {
        Self { _api_key: api_key.to_string() }
    }
}

fn cf_not_ready() -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(
        "CurseForge support is not yet implemented. Remove 'curseforge' from [resolver].platforms."
    ))
}

#[async_trait]
impl ModProvider for CurseForgeProvider {
    fn name(&self) -> &'static str { "curseforge" }

    async fn search(&self, _q: &str, _mc: Option<&str>, _l: Option<&str>, _n: usize)
        -> Result<Vec<SearchResultItem>, OrbitError> { Err(cf_not_ready()) }

    async fn get_mod_info(&self, _s: &str) -> Result<ModInfo, OrbitError> { Err(cf_not_ready()) }

    async fn resolve(&self, _s: &str, _c: &str, _mc: &str, _l: &str)
        -> Result<ResolvedMod, OrbitError> { Err(cf_not_ready()) }

    async fn get_version_by_hash(&self, _h: &str) -> Result<Option<ResolvedMod>, OrbitError> { Err(cf_not_ready()) }

    async fn get_versions(&self, _s: &str, _mc: Option<&str>, _l: Option<&str>)
        -> Result<Vec<ResolvedMod>, OrbitError> { Err(cf_not_ready()) }

    async fn get_categories(&self) -> Result<Vec<String>, OrbitError> { Err(cf_not_ready()) }
}
