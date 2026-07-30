use crate::error::Result;
use reqwest::{Client as ReqwestClient, header};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub timeout: Duration,
    pub max_retries: u32,
    pub proxy: Option<String>,
    pub authorization: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_retries: 3,
            proxy: None,
            authorization: None,
        }
    }
}

pub struct Client {
    pub(crate) http: ReqwestClient,
    pub(crate) base_url: String,
}

impl Client {
    pub fn new(user_agent: &str, config: &ClientConfig) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_str(user_agent)
                .map_err(|_| crate::error::ModrinthError::Api("Invalid User-Agent".into()))?,
        );
        if let Some(value) = authorization_header(config.authorization.as_deref())? {
            headers.insert(header::AUTHORIZATION, value);
        }

        let retry_policy = reqwest::retry::for_host("api.modrinth.com")
            .no_budget()
            .max_retries_per_request(config.max_retries)
            .classify_fn(|request| {
                if request.error().is_some()
                    || request.status().is_some_and(|status| {
                        status == reqwest::StatusCode::REQUEST_TIMEOUT
                            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                            || status.is_server_error()
                    })
                {
                    request.retryable()
                } else {
                    request.success()
                }
            });

        let mut builder = ReqwestClient::builder()
            .default_headers(headers)
            .timeout(config.timeout)
            .retry(retry_policy);
        if let Some(proxy) = config.proxy.as_deref() {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        let http = builder.build()?;

        Ok(Self {
            http,
            base_url: "https://api.modrinth.com/v2".to_string(),
        })
    }

    /// 检查 HTTP 响应状态，保留 body 文本用于错误报告
    pub(crate) async fn check_response(
        &self,
        resp: reqwest::Response,
    ) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(crate::error::ModrinthError::Api(format!(
            "HTTP {status}: {body}"
        )))
    }
}

fn authorization_header(token: Option<&str>) -> Result<Option<header::HeaderValue>> {
    token
        .map(|token| {
            let mut value = header::HeaderValue::from_str(token.trim()).map_err(|_| {
                crate::error::ModrinthError::Api("Invalid Modrinth authorization token".into())
            })?;
            value.set_sensitive(true);
            Ok(value)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_authorization_is_attached_and_marked_sensitive() {
        let authorization = authorization_header(Some("mrp_secret")).unwrap().unwrap();
        assert_eq!(authorization.to_str().unwrap(), "mrp_secret");
        assert!(authorization.is_sensitive());
    }

    #[test]
    fn invalid_proxy_is_rejected_during_client_construction() {
        let error = Client::new(
            "orbit-test",
            &ClientConfig {
                proxy: Some("://invalid".to_string()),
                ..ClientConfig::default()
            },
        )
        .err()
        .expect("invalid proxy should fail");
        assert!(error.to_string().contains("builder error"));
    }
}
