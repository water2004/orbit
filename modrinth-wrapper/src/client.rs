use std::time::Duration;
use reqwest::{Client as ReqwestClient, header};
use crate::error::Result;

pub struct Client {
    pub(crate) http: ReqwestClient,
    pub(crate) base_url: String,
}

impl Client {
    pub fn new(user_agent: &str) -> Result<Self> {
        Self::with_timeout(user_agent, Duration::from_secs(30))
    }

    pub fn with_timeout(user_agent: &str, timeout: Duration) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_str(user_agent)
                .map_err(|_| crate::error::ModrinthError::Api("Invalid User-Agent".into()))?,
        );

        let http = ReqwestClient::builder()
            .default_headers(headers)
            .timeout(timeout)
            .build()?;

        Ok(Self { http, base_url: "https://api.modrinth.com/v2".to_string() })
    }

    /// 检查 HTTP 响应状态，保留 body 文本用于错误报告
    pub(crate) async fn check_response(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() { return Ok(resp); }
        let body = resp.text().await.unwrap_or_default();
        Err(crate::error::ModrinthError::Api(format!("HTTP {status}: {body}")))
    }
}
