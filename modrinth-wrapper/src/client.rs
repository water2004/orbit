use crate::error::Result;
use reqwest::{Client as ReqwestClient, header};
use serde::de::DeserializeOwned;
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
    base_url: url::Url,
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
            .redirect(reqwest::redirect::Policy::none())
            .retry(retry_policy);
        if let Some(proxy) = config.proxy.as_deref() {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        let http = builder.build()?;

        let base_url = url::Url::parse("https://api.modrinth.com/v2/")
            .map_err(|error| crate::error::ModrinthError::Api(error.to_string()))?;
        Ok(Self { http, base_url })
    }

    /// 检查 HTTP 响应状态，保留 body 文本用于错误报告
    pub(crate) async fn check_response(
        &self,
        mut resp: reqwest::Response,
    ) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = bounded_error_body(&mut resp).await;
        Err(crate::error::ModrinthError::Api(format!(
            "HTTP {status}: {body}"
        )))
    }

    pub(crate) fn endpoint(&self, segments: &[&str], query: &[(&str, String)]) -> Result<url::Url> {
        let mut url = self.base_url.clone();
        {
            let mut path = url.path_segments_mut().map_err(|_| {
                crate::error::ModrinthError::Api(
                    "Modrinth API base URL cannot be extended".to_string(),
                )
            })?;
            path.pop_if_empty();
            for segment in segments {
                if segment.is_empty() || segment.chars().any(char::is_control) {
                    return Err(crate::error::ModrinthError::Api(
                        "Modrinth API path segment is empty or contains control characters"
                            .to_string(),
                    ));
                }
                path.push(segment);
            }
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().cloned());
        }
        Ok(url)
    }

    pub(crate) async fn decode_json<T: DeserializeOwned>(
        &self,
        mut response: reqwest::Response,
    ) -> Result<T> {
        const LIMIT: usize = 32 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|length| length > LIMIT as u64)
        {
            return Err(crate::error::ModrinthError::Api(
                "Modrinth response exceeds the 32 MiB JSON limit".to_string(),
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len().saturating_add(chunk.len()) > LIMIT {
                return Err(crate::error::ModrinthError::Api(
                    "Modrinth response exceeds the 32 MiB JSON limit".to_string(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&body)?)
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

fn authorization_header(token: Option<&str>) -> Result<Option<header::HeaderValue>> {
    token
        .map(|token| {
            let token = token.trim();
            if token.is_empty() {
                return Err(crate::error::ModrinthError::Api(
                    "Modrinth authorization token must not be empty".into(),
                ));
            }
            let mut value = header::HeaderValue::from_str(token).map_err(|_| {
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

    #[test]
    fn endpoint_encodes_project_identifiers_as_one_path_segment() {
        let client = Client::new("orbit-test", &ClientConfig::default()).unwrap();

        let endpoint = client
            .endpoint(&["project", "project/with?syntax"], &[])
            .unwrap();

        assert_eq!(
            endpoint.as_str(),
            "https://api.modrinth.com/v2/project/project%2Fwith%3Fsyntax"
        );
        assert!(client.endpoint(&["project", ""], &[]).is_err());
    }

    #[test]
    fn empty_authorization_is_rejected() {
        assert!(authorization_header(Some("  ")).is_err());
    }
}
