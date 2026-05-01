use crate::{Client, error::Result, models::*};
use std::collections::HashMap;

// ─────────────────────────────────────────────
// Search builder
// ─────────────────────────────────────────────

/// Builder for search query parameters.
///
/// # Example
/// ```no_run
/// # async fn demo() -> modrinth_wrapper::Result<()> {
/// use modrinth_wrapper::api::SearchParams;
/// let client = modrinth_wrapper::Client::new("my-app")?;
/// let result = client
///     .search(SearchParams::new("fabric api")
///         .index("downloads")
///         .limit(20))
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct SearchParams {
    query: String,
    facets: Option<String>,
    index: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

impl SearchParams {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            facets: None,
            index: None,
            offset: None,
            limit: None,
        }
    }

    /// The facets to filter by. Should be a JSON-encoded string of arrays.
    /// e.g. `[["categories:forge"],["versions:1.17.1"]]`
    pub fn facets(mut self, facets: impl Into<String>) -> Self {
        self.facets = Some(facets.into());
        self
    }

    /// The sorting method. Allowed values: relevance, downloads, follows, newest, updated.
    /// Default: relevance.
    pub fn index(mut self, index: impl Into<String>) -> Self {
        self.index = Some(index.into());
        self
    }

    /// The offset into the search (number of results to skip). Default: 0.
    pub fn offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// The number of results to return. Default: 10, max: 100.
    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }
}

// ─────────────────────────────────────────────
// ListVersions builder
// ─────────────────────────────────────────────

/// Builder for listing version query parameters.
///
/// # Example
/// ```no_run
/// # async fn demo() -> modrinth_wrapper::Result<()> {
/// use modrinth_wrapper::api::ListVersionsParams;
/// let client = modrinth_wrapper::Client::new("my-app")?;
/// let versions = client
///     .list_versions_with_params("fabric-api",
///         ListVersionsParams::new()
///             .loaders(&["fabric"])
///             .game_versions(&["1.20.1"]))
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct ListVersionsParams {
    loaders: Option<Vec<String>>,
    game_versions: Option<Vec<String>>,
    featured: Option<bool>,
    include_changelog: Option<bool>,
}

impl ListVersionsParams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by loaders (e.g. ["fabric", "forge"]).
    pub fn loaders(mut self, loaders: &[&str]) -> Self {
        self.loaders = Some(loaders.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Filter by game versions (e.g. ["1.20.1"]).
    pub fn game_versions(mut self, game_versions: &[&str]) -> Self {
        self.game_versions = Some(game_versions.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Filter for featured or non-featured versions only.
    pub fn featured(mut self, featured: bool) -> Self {
        self.featured = Some(featured);
        self
    }

    /// Whether to include the changelog in the response. Default: true.
    /// It is highly recommended to set this to false in most cases.
    pub fn include_changelog(mut self, include: bool) -> Self {
        self.include_changelog = Some(include);
        self
    }
}

/// Helper: build a URL with query parameters using `url::Url`.
fn build_url(base: &str, path: &str, params: &[(&str, String)]) -> String {
    let mut url = url::Url::parse(&format!("{}{}", base, path)).expect("valid base URL");
    for (k, v) in params {
        url.query_pairs_mut().append_pair(k, v);
    }
    url.to_string()
}

impl Client {
    // ─────────────────────────────────────────
    // Project Endpoints
    // ─────────────────────────────────────────

    /// Get a project by ID or slug.
    /// `GET /project/{id|slug}`
    pub async fn get_project(&self, id_or_slug: &str) -> Result<Project> {
        let url = format!("{}/project/{}", self.base_url, id_or_slug);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let project = resp.json::<Project>().await?;
        Ok(project)
    }

    /// Get multiple projects by their IDs.
    /// `GET /projects?ids=[...]`
    pub async fn get_projects(&self, ids: &[&str]) -> Result<Vec<Project>> {
        let ids_json = serde_json::to_string(ids)?;
        let url = build_url(&self.base_url, "/projects", &[("ids", ids_json)]);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Vec<Project>>().await?;
        Ok(res)
    }

    /// Get all of a project's dependencies.
    /// `GET /project/{id|slug}/dependencies`
    pub async fn get_project_dependencies(&self, id_or_slug: &str) -> Result<ProjectDependencyList> {
        let url = format!("{}/project/{}/dependencies", self.base_url, id_or_slug);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<ProjectDependencyList>().await?;
        Ok(res)
    }

    /// Search projects with a simple query string.
    /// `GET /search?query=...`
    pub async fn search_projects(&self, query: &str) -> Result<SearchResult> {
        self.search(SearchParams::new(query)).await
    }

    /// Search projects with full parameter control via [`SearchParams`] builder.
    /// `GET /search?query=...&facets=...&index=...&offset=...&limit=...`
    pub async fn search(&self, params: SearchParams) -> Result<SearchResult> {
        let mut query_params: Vec<(&str, String)> = vec![("query", params.query.clone())];
        if let Some(ref facets) = params.facets {
            query_params.push(("facets", facets.clone()));
        }
        if let Some(ref index) = params.index {
            query_params.push(("index", index.clone()));
        }
        if let Some(offset) = params.offset {
            query_params.push(("offset", offset.to_string()));
        }
        if let Some(limit) = params.limit {
            query_params.push(("limit", limit.to_string()));
        }
        let url = build_url(&self.base_url, "/search", &query_params);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<SearchResult>().await?;
        Ok(res)
    }

    // ─────────────────────────────────────────
    // Version Endpoints
    // ─────────────────────────────────────────

    /// Get a version by its ID.
    /// `GET /version/{id}`
    pub async fn get_version_by_id(&self, version_id: &str) -> Result<Version> {
        let url = format!("{}/version/{}", self.base_url, version_id);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Version>().await?;
        Ok(res)
    }

    /// Get a version given a project ID/slug and a version ID or number.
    /// `GET /project/{id|slug}/version/{id|number}`
    pub async fn get_version(&self, project_id: &str, version_id_or_number: &str) -> Result<Version> {
        let url = format!("{}/project/{}/version/{}", self.base_url, project_id, version_id_or_number);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Version>().await?;
        Ok(res)
    }

    /// Get multiple versions by their IDs.
    /// `GET /versions?ids=[...]`
    pub async fn get_versions_by_ids(&self, ids: &[&str]) -> Result<Vec<Version>> {
        let ids_json = serde_json::to_string(ids)?;
        let url = build_url(&self.base_url, "/versions", &[("ids", ids_json)]);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Vec<Version>>().await?;
        Ok(res)
    }

    /// List all versions of a project (no filtering).
    /// `GET /project/{id|slug}/version`
    pub async fn list_versions(&self, project_id: &str) -> Result<Vec<Version>> {
        let url = format!("{}/project/{}/version", self.base_url, project_id);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Vec<Version>>().await?;
        Ok(res)
    }

    /// List versions of a project with filtering via [`ListVersionsParams`] builder.
    /// `GET /project/{id|slug}/version?loaders=...&game_versions=...&featured=...&include_changelog=...`
    pub async fn list_versions_with_params(&self, project_id: &str, params: ListVersionsParams) -> Result<Vec<Version>> {
        let mut query_params: Vec<(&str, String)> = Vec::new();
        if let Some(ref loaders) = params.loaders {
            query_params.push(("loaders", serde_json::to_string(loaders).unwrap_or_default()));
        }
        if let Some(ref gv) = params.game_versions {
            query_params.push(("game_versions", serde_json::to_string(gv).unwrap_or_default()));
        }
        if let Some(featured) = params.featured {
            query_params.push(("featured", featured.to_string()));
        }
        if let Some(include) = params.include_changelog {
            query_params.push(("include_changelog", include.to_string()));
        }
        let path = format!("/project/{}/version", project_id);
        let url = build_url(&self.base_url, &path, &query_params);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Vec<Version>>().await?;
        Ok(res)
    }

    // ─────────────────────────────────────────
    // Version File Endpoints
    // ─────────────────────────────────────────

    /// Get a version from a file hash.
    /// `GET /version_file/{hash}?algorithm=...`
    ///
    /// If `algorithm` is `None`, defaults to `sha1`.
    /// If `multiple` is `true`, returns the version even if multiple files share the same hash.
    pub async fn get_version_from_hash(&self, hash: &str, algorithm: Option<&str>, multiple: Option<bool>) -> Result<Version> {
        let algo = algorithm.unwrap_or("sha1");
        let mut query_params: Vec<(&str, String)> = vec![("algorithm", algo.to_string())];
        if let Some(true) = multiple {
            query_params.push(("multiple", "true".to_string()));
        }
        let path = format!("/version_file/{}", hash);
        let url = build_url(&self.base_url, &path, &query_params);
        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let res = resp.json::<Version>().await?;
        Ok(res)
    }

    /// Get versions from multiple file hashes.
    /// `POST /version_files`
    pub async fn get_versions_from_hashes(&self, hashes: &[&str], algorithm: Option<&str>) -> Result<HashMap<String, Version>> {
        let algo = algorithm.unwrap_or("sha1");
        let url = format!("{}/version_files", self.base_url);
        let body = serde_json::json!({
            "hashes": hashes,
            "algorithm": algo
        });
        let resp = self.http.post(&url).json(&body).send().await?.error_for_status()?;
        let res = resp.json::<HashMap<String, Version>>().await?;
        Ok(res)
    }

    /// Get the latest version of a project from a file hash, loader(s), and game version(s).
    /// `POST /version_file/{hash}/update?algorithm=...`
    pub async fn get_latest_version_from_hash(&self, hash: &str, loaders: &[&str], game_versions: &[&str], algorithm: Option<&str>) -> Result<Version> {
        let algo = algorithm.unwrap_or("sha1");
        let path = format!("/version_file/{}/update", hash);
        let url = build_url(&self.base_url, &path, &[("algorithm", algo.to_string())]);
        let body = serde_json::json!({
            "loaders": loaders,
            "game_versions": game_versions
        });
        let resp = self.http.post(&url).json(&body).send().await?.error_for_status()?;
        let res = resp.json::<Version>().await?;
        Ok(res)
    }

    /// Get the latest versions of multiple projects from file hashes, loader(s), and game version(s).
    /// `POST /version_files/update`
    pub async fn get_latest_versions_from_hashes(&self, hashes: &[&str], loaders: &[&str], game_versions: &[&str], algorithm: Option<&str>) -> Result<HashMap<String, Version>> {
        let algo = algorithm.unwrap_or("sha1");
        let url = format!("{}/version_files/update", self.base_url);
        let body = serde_json::json!({
            "hashes": hashes,
            "algorithm": algo,
            "loaders": loaders,
            "game_versions": game_versions
        });
        let resp = self.http.post(&url).json(&body).send().await?.error_for_status()?;
        let res = resp.json::<HashMap<String, Version>>().await?;
        Ok(res)
    }
}
