use reqwest::StatusCode;
use serde::de::DeserializeOwned;

use crate::models::{
    ApiResponse, Category, File, FingerprintMatches, FingerprintsRequest, Game, GetFilesParams,
    GetModsRequest, Mod, PagedResponse, SearchModsParams,
};

const DEFAULT_BASE_URL: &str = "https://api.curseforge.com/v1/";
const PAGE_SIZE: u32 = 50;
pub const MAX_RESULTS: u32 = 10_000;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub timeout: std::time::Duration,
    pub max_retries: u32,
    pub proxy: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            timeout: std::time::Duration::from_secs(30),
            max_retries: 3,
            proxy: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("CurseForge API key is required")]
    MissingApiKey,
    #[error("invalid CurseForge API key header: {0}")]
    InvalidApiKey(#[source] reqwest::header::InvalidHeaderValue),
    #[error("failed to build CurseForge HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("invalid CurseForge API base URL: {0}")]
    BaseUrl(#[source] url::ParseError),
    #[error("invalid CurseForge API base URL: {0}")]
    InvalidBaseUrl(String),
    #[error("CurseForge request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("CurseForge response exceeds the {0} MiB JSON limit")]
    ResponseTooLarge(usize),
    #[error("invalid CurseForge JSON response: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("CurseForge API returned HTTP {status}: {message}")]
    Status { status: StatusCode, message: String },
}

impl ApiError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::MissingApiKey
            | Self::InvalidApiKey(_)
            | Self::Client(_)
            | Self::BaseUrl(_)
            | Self::InvalidBaseUrl(_)
            | Self::Request(_)
            | Self::ResponseTooLarge(_)
            | Self::Decode(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: url::Url,
}

impl Client {
    pub fn new(api_key: &str, user_agent: &str, config: &ClientConfig) -> Result<Self, ApiError> {
        Self::build(api_key, user_agent, DEFAULT_BASE_URL, config)
    }

    /// Creates a client for a compatible API endpoint.
    ///
    /// This is primarily useful for integration tests and self-hosted proxies.
    pub fn with_base_url(
        api_key: &str,
        user_agent: &str,
        base_url: &str,
        config: &ClientConfig,
    ) -> Result<Self, ApiError> {
        Self::build(api_key, user_agent, base_url, config)
    }

    fn build(
        api_key: &str,
        user_agent: &str,
        base_url: &str,
        config: &ClientConfig,
    ) -> Result<Self, ApiError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-api-key", api_key_header(api_key)?);
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let mut parsed_base = url::Url::parse(base_url).map_err(ApiError::BaseUrl)?;
        let loopback_http = parsed_base.scheme() == "http"
            && parsed_base.host().is_some_and(|host| match host {
                url::Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
                url::Host::Ipv4(address) => address.is_loopback(),
                url::Host::Ipv6(address) => address.is_loopback(),
            });
        if parsed_base.scheme() != "https" && !loopback_http {
            return Err(ApiError::InvalidBaseUrl(
                "scheme must be HTTPS (plain HTTP is limited to loopback test endpoints because the endpoint receives an API key)".to_string(),
            ));
        }
        if parsed_base.cannot_be_a_base()
            || !parsed_base.username().is_empty()
            || parsed_base.password().is_some()
            || parsed_base.query().is_some()
            || parsed_base.fragment().is_some()
        {
            return Err(ApiError::InvalidBaseUrl(
                "URL must be an absolute HTTP endpoint without credentials, query, or fragment"
                    .to_string(),
            ));
        }
        if !parsed_base.path().ends_with('/') {
            parsed_base
                .path_segments_mut()
                .map_err(|_| ApiError::InvalidBaseUrl("URL cannot be extended".to_string()))?
                .push("");
        }
        let retry_host = parsed_base
            .host_str()
            .ok_or_else(|| ApiError::InvalidBaseUrl("host is missing".to_string()))?
            .to_string();
        let retry_policy = reqwest::retry::for_host(retry_host)
            .no_budget()
            .max_retries_per_request(config.max_retries)
            .classify_fn(|request| {
                if request.error().is_some()
                    || request.status().is_some_and(|status| {
                        status == StatusCode::REQUEST_TIMEOUT
                            || status == StatusCode::TOO_MANY_REQUESTS
                            || status.is_server_error()
                    })
                {
                    request.retryable()
                } else {
                    request.success()
                }
            });
        let mut builder = reqwest::Client::builder()
            .user_agent(user_agent)
            .default_headers(headers)
            .timeout(config.timeout)
            // Never forward x-api-key through an unvalidated redirect.
            .redirect(reqwest::redirect::Policy::none())
            // CurseForge operations are read-only, including its POST lookup endpoints.
            .retry(retry_policy);
        if let Some(proxy) = config.proxy.as_deref() {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(ApiError::Client)?);
        }
        let http = builder.build().map_err(ApiError::Client)?;
        Ok(Self {
            http,
            base_url: parsed_base,
        })
    }

    fn url(&self, path: &str) -> Result<url::Url, ApiError> {
        let mut url = self.base_url.clone();
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| ApiError::InvalidBaseUrl("URL cannot be extended".to_string()))?;
        segments.pop_if_empty();
        for segment in path.split('/') {
            if segment.is_empty() || segment.chars().any(char::is_control) {
                return Err(ApiError::InvalidBaseUrl(
                    "API path contains an invalid segment".to_string(),
                ));
            }
            segments.push(segment);
        }
        drop(segments);
        Ok(url)
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ApiError> {
        let mut response = request.send().await.map_err(ApiError::Request)?;
        let status = response.status();
        if !status.is_success() {
            let message = bounded_error_body(&mut response).await;
            return Err(ApiError::Status { status, message });
        }
        const LIMIT: usize = 32 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|length| length > LIMIT as u64)
        {
            return Err(ApiError::ResponseTooLarge(LIMIT / (1024 * 1024)));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ApiError::Request)? {
            if body.len().saturating_add(chunk.len()) > LIMIT {
                return Err(ApiError::ResponseTooLarge(LIMIT / (1024 * 1024)));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(ApiError::Decode)
    }

    pub async fn games(&self) -> Result<Vec<Game>, ApiError> {
        let mut index = 0;
        let mut games = Vec::new();
        loop {
            let response: PagedResponse<Game> = self
                .send(
                    self.http
                        .get(self.url("games")?)
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
                    .get(self.url("categories")?)
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
        self.send(self.http.get(self.url("mods/search")?).query(&query))
            .await
    }

    pub async fn get_mod(&self, project_id: u32) -> Result<Mod, ApiError> {
        let response: ApiResponse<Mod> = self
            .send(self.http.get(self.url(&format!("mods/{project_id}"))?))
            .await?;
        Ok(response.data)
    }

    pub async fn get_mods(&self, project_ids: &[u32]) -> Result<Vec<Mod>, ApiError> {
        if project_ids.is_empty() {
            return Ok(Vec::new());
        }
        let response: ApiResponse<Vec<Mod>> = self
            .send(self.http.post(self.url("mods")?).json(&GetModsRequest {
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
                        .get(self.url(&format!("mods/{project_id}/files"))?)
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
                    .get(self.url(&format!("mods/{project_id}/files/{file_id}/download-url"))?),
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
                    .post(self.url(&format!("fingerprints/{game_id}"))?)
                    .json(&FingerprintsRequest { fingerprints }),
            )
            .await?;
        Ok(response.data)
    }
}

async fn bounded_error_body(response: &mut reqwest::Response) -> String {
    const LIMIT: usize = 4 * 1024;
    let mut body = Vec::new();
    while body.len() < LIMIT {
        let Ok(Some(chunk)) = response.chunk().await else {
            break;
        };
        let remaining = LIMIT - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    String::from_utf8_lossy(&body).into_owned()
}

fn api_key_header(api_key: &str) -> Result<reqwest::header::HeaderValue, ApiError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(ApiError::MissingApiKey);
    }
    let mut value =
        reqwest::header::HeaderValue::from_str(api_key).map_err(ApiError::InvalidApiKey)?;
    value.set_sensitive(true);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_is_sensitive_and_empty_values_are_rejected() {
        let value = api_key_header(" secret ").unwrap();
        assert_eq!(value.to_str().unwrap(), "secret");
        assert!(value.is_sensitive());
        assert!(matches!(api_key_header("  "), Err(ApiError::MissingApiKey)));
    }

    #[test]
    fn base_url_and_endpoint_paths_are_structured() {
        let client = Client::with_base_url(
            "secret",
            "orbit-test",
            "https://example.invalid/api/v1",
            &ClientConfig::default(),
        )
        .unwrap();
        assert_eq!(
            client.url("mods/123").unwrap().as_str(),
            "https://example.invalid/api/v1/mods/123"
        );
        assert!(client.url("mods//123").is_err());
        assert!(
            Client::with_base_url(
                "secret",
                "orbit-test",
                "https://example.invalid/api?token=wrong-place",
                &ClientConfig::default(),
            )
            .is_err()
        );
        assert!(
            Client::with_base_url(
                "secret",
                "orbit-test",
                "http://example.invalid/api/v1/",
                &ClientConfig::default(),
            )
            .is_err()
        );
    }
}
