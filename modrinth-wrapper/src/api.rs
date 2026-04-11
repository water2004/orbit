use crate::{Client, error::Result, models::*};
use std::collections::HashMap;

impl Client {
    // Project Endpoints
    pub async fn get_project(&self, id_or_slug: &str) -> Result<Project> {
        let url = format!("{}/project/{}", self.base_url, id_or_slug);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let project = resp.json::<Project>().await?;
        Ok(project)
    }

    pub async fn get_projects(&self, ids: &[&str]) -> Result<Vec<Project>> {
        let url = format!("{}/projects", self.base_url);
        let ids_str = serde_json::to_string(ids)?;
        let url_with_query = format!("{}?ids={}", url, ids_str);
        let resp = self.http.get(&url_with_query).send().await?.error_for_status()?;
        let res = resp.json::<Vec<Project>>().await?;
        Ok(res)
    }

    pub async fn get_project_dependencies(&self, id_or_slug: &str) -> Result<crate::models::Dependencies> {
        let url = format!("{}/project/{}/dependencies", self.base_url, id_or_slug);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<crate::models::Dependencies>().await?;
        Ok(res)
    }

    pub async fn search_projects(&self, query: &str) -> Result<SearchResult> {
        let url = format!("{}/search?query={}", self.base_url, query);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<SearchResult>().await?;
        Ok(res)
    }

    // Version Endpoints
    pub async fn get_version(&self, project_id: &str, version_id_or_number: &str) -> Result<Version> {
        let url = format!("{}/project/{}/version/{}", self.base_url, project_id, version_id_or_number);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Version>().await?;
        Ok(res)
    }

    pub async fn get_latest_version_from_hash(&self, hash: &str, loaders: &[&str], game_versions: &[&str], algorithm: Option<&str>) -> Result<Version> {
        let algo = algorithm.unwrap_or("sha1");
        let url = format!("{}/version_file/{}/update?algorithm={}", self.base_url, hash, algo);
        let body = serde_json::json!({
            "loaders": loaders,
            "game_versions": game_versions
        });
        let resp = self.http.post(&url).json(&body).send().await?.error_for_status()?;
        let res = resp.json::<Version>().await?;
        Ok(res)
    }

    pub async fn get_latest_versions_from_hashes(&self, hashes: &[&str], loaders: &[&str], game_versions: &[&str], algorithm: Option<&str>) -> Result<HashMap<String, Version>> {
        let algo = algorithm.unwrap_or("sha1");
        let url = format!("{}/version_files/update?algorithm={}", self.base_url, algo);
        let body = serde_json::json!({
            "hashes": hashes,
            "loaders": loaders,
            "game_versions": game_versions
        });
        let resp = self.http.post(&url).json(&body).send().await?.error_for_status()?;
        let res = resp.json::<HashMap<String, Version>>().await?;
        Ok(res)
    }

    pub async fn list_versions(&self, project_id: &str) -> Result<Vec<Version>> {
        let url = format!("{}/project/{}/version", self.base_url, project_id);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Vec<Version>>().await?;
        Ok(res)
    }

    // Version File Endpoints
    pub async fn get_version_from_hash(&self, hash: &str) -> Result<Version> {
        let url = format!("{}/version_file/{}", self.base_url, hash);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Version>().await?;
        Ok(res)
    }

    pub async fn get_versions_from_hashes(&self, hashes: Vec<String>) -> Result<HashMap<String, Version>> {
        let url = format!("{}/version_files", self.base_url);
        let body = serde_json::json!({ "hashes": hashes });
        let resp = self.http.post(&url).json(&body).send().await?.error_for_status()?;
        let res = resp.json::<HashMap<String, Version>>().await?;
        Ok(res)
    }
}
