use reqwest::StatusCode;
use serde::de::DeserializeOwned;

use super::models::{
    ApiResponse, Category, File, FingerprintMatches, FingerprintsRequest, Game, GetFilesParams,
    GetModsRequest, Mod, PagedResponse, SearchModsParams,
};

const DEFAULT_BASE_URL: &str = "https://api.curseforge.com/v1/";
const PAGE_SIZE: u32 = 50;
pub(super) const MAX_RESULTS: u32 = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("invalid CurseForge API key header: {0}")]
    InvalidApiKey(#[source] reqwest::header::InvalidHeaderValue),
    #[error("failed to build CurseForge HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("CurseForge request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("CurseForge API returned HTTP {status}: {message}")]
    Status { status: StatusCode, message: String },
}

impl ApiError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::InvalidApiKey(_) | Self::Client(_) | Self::Request(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
}

impl Client {
    pub fn new(api_key: &str, user_agent: &str) -> Result<Self, ApiError> {
        Self::build(api_key, user_agent, DEFAULT_BASE_URL)
    }

    #[cfg(test)]
    pub(crate) fn with_base_url(
        api_key: &str,
        user_agent: &str,
        base_url: &str,
    ) -> Result<Self, ApiError> {
        Self::build(api_key, user_agent, base_url)
    }

    fn build(api_key: &str, user_agent: &str, base_url: &str) -> Result<Self, ApiError> {
        let mut headers = reqwest::header::HeaderMap::new();
        let api_key =
            reqwest::header::HeaderValue::from_str(api_key).map_err(ApiError::InvalidApiKey)?;
        headers.insert("x-api-key", api_key);
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(ApiError::Client)?;
        Ok(Self {
            http,
            base_url: format!("{}/", base_url.trim_end_matches('/')),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path.trim_start_matches('/'))
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ApiError> {
        let response = request.send().await.map_err(ApiError::Request)?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            let message = message.chars().take(500).collect();
            return Err(ApiError::Status { status, message });
        }
        response.json().await.map_err(ApiError::Request)
    }

    pub async fn games(&self) -> Result<Vec<Game>, ApiError> {
        let mut index = 0;
        let mut games = Vec::new();
        loop {
            let response: PagedResponse<Game> = self
                .send(
                    self.http
                        .get(self.url("games"))
                        .query(&[("index", index), ("pageSize", PAGE_SIZE)]),
                )
                .await?;
            let done = response.pagination.result_count == 0
                || u64::from(response.pagination.index + response.pagination.result_count)
                    >= response.pagination.total_count
                || response.pagination.index + response.pagination.page_size >= MAX_RESULTS;
            games.extend(response.data);
            if done {
                break;
            }
            index = response.pagination.index + response.pagination.page_size;
        }
        Ok(games)
    }

    pub async fn categories(&self, game_id: u32) -> Result<Vec<Category>, ApiError> {
        let response: ApiResponse<Vec<Category>> = self
            .send(
                self.http
                    .get(self.url("categories"))
                    .query(&[("gameId", game_id)]),
            )
            .await?;
        Ok(response.data)
    }

    pub async fn search_mods(
        &self,
        params: SearchModsParams<'_>,
    ) -> Result<PagedResponse<Mod>, ApiError> {
        let mut query = vec![
            ("gameId", params.game_id.to_string()),
            ("classId", params.class_id.to_string()),
            ("index", params.index.to_string()),
            ("pageSize", params.page_size.min(PAGE_SIZE).to_string()),
        ];
        if let Some(value) = params.search_filter {
            query.push(("searchFilter", value.to_string()));
        }
        if let Some(value) = params.slug {
            query.push(("slug", value.to_string()));
        }
        if let Some(value) = params.game_version {
            query.push(("gameVersion", value.to_string()));
        }
        if let Some(value) = params.mod_loader_type {
            query.push(("modLoaderType", (value as u8).to_string()));
        }
        self.send(self.http.get(self.url("mods/search")).query(&query))
            .await
    }

    pub async fn get_mod(&self, project_id: u32) -> Result<Mod, ApiError> {
        let response: ApiResponse<Mod> = self
            .send(self.http.get(self.url(&format!("mods/{project_id}"))))
            .await?;
        Ok(response.data)
    }

    pub async fn get_mods(&self, project_ids: &[u32]) -> Result<Vec<Mod>, ApiError> {
        if project_ids.is_empty() {
            return Ok(Vec::new());
        }
        let response: ApiResponse<Vec<Mod>> = self
            .send(self.http.post(self.url("mods")).json(&GetModsRequest {
                mod_ids: project_ids,
                filter_pc_only: false,
            }))
            .await?;
        Ok(response.data)
    }

    pub async fn get_files(
        &self,
        project_id: u32,
        params: GetFilesParams<'_>,
    ) -> Result<Vec<File>, ApiError> {
        let mut index = 0;
        let mut files = Vec::new();
        loop {
            let mut query = vec![
                ("index", index.to_string()),
                ("pageSize", PAGE_SIZE.to_string()),
            ];
            if let Some(value) = params.game_version {
                query.push(("gameVersion", value.to_string()));
            }
            if let Some(value) = params.mod_loader_type {
                query.push(("modLoaderType", (value as u8).to_string()));
            }
            let response: PagedResponse<File> = self
                .send(
                    self.http
                        .get(self.url(&format!("mods/{project_id}/files")))
                        .query(&query),
                )
                .await?;
            let done = response.pagination.result_count == 0
                || u64::from(response.pagination.index + response.pagination.result_count)
                    >= response.pagination.total_count
                || response.pagination.index + response.pagination.page_size >= MAX_RESULTS;
            files.extend(response.data);
            if done {
                break;
            }
            index = response.pagination.index + response.pagination.page_size;
        }
        Ok(files)
    }

    pub async fn download_url(&self, project_id: u32, file_id: u32) -> Result<String, ApiError> {
        let response: ApiResponse<String> = self
            .send(
                self.http
                    .get(self.url(&format!("mods/{project_id}/files/{file_id}/download-url"))),
            )
            .await?;
        Ok(response.data)
    }

    pub async fn fingerprint_matches(
        &self,
        game_id: u32,
        fingerprints: &[u32],
    ) -> Result<FingerprintMatches, ApiError> {
        if fingerprints.is_empty() {
            return Ok(FingerprintMatches {
                exact_matches: Vec::new(),
            });
        }
        let response: ApiResponse<FingerprintMatches> = self
            .send(
                self.http
                    .post(self.url(&format!("fingerprints/{game_id}")))
                    .json(&FingerprintsRequest { fingerprints }),
            )
            .await?;
        Ok(response.data)
    }
}
