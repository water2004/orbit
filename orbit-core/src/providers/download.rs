//! Provider-owned artifact downloads.
//!
//! Provider credentials are runtime-only and are scoped to trusted hosts here.
//! Callers share one download path and never copy authentication rules into the
//! installer, resolver, or lockfile.

use reqwest::header::{HeaderName, HeaderValue, LOCATION};
use url::Url;

use crate::error::OrbitError;

const MAX_REDIRECTS: usize = 5;

#[derive(Clone)]
pub struct ArtifactDownloadClient {
    http: reqwest::Client,
    authorization: Option<ScopedAuthorization>,
}

#[derive(Clone)]
struct ScopedAuthorization {
    header_name: HeaderName,
    header_value: HeaderValue,
    trusted_domain: String,
}

impl ArtifactDownloadClient {
    pub fn anonymous(user_agent: &str) -> Result<Self, OrbitError> {
        Self::build(user_agent, None)
    }

    pub fn authenticated_for_domain(
        user_agent: &str,
        header_name: &str,
        header_value: &str,
        trusted_domain: &str,
    ) -> Result<Self, OrbitError> {
        if header_value.trim().is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "artifact download credential cannot be empty"
            )));
        }
        let trusted_domain = trusted_domain
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if trusted_domain.is_empty() || trusted_domain.contains('/') || trusted_domain.contains(':')
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "artifact download trusted domain is invalid"
            )));
        }
        let header_name = HeaderName::from_bytes(header_name.as_bytes())
            .map_err(|error| OrbitError::Other(error.into()))?;
        let header_value =
            HeaderValue::from_str(header_value).map_err(|error| OrbitError::Other(error.into()))?;
        Self::build(
            user_agent,
            Some(ScopedAuthorization {
                header_name,
                header_value,
                trusted_domain,
            }),
        )
    }

    fn build(
        user_agent: &str,
        authorization: Option<ScopedAuthorization>,
    ) -> Result<Self, OrbitError> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(std::time::Duration::from_secs(60))
            // Authentication is applied per request, so every redirect target
            // must be validated before a credential can be attached.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| OrbitError::Other(error.into()))?;
        Ok(Self {
            http,
            authorization,
        })
    }

    pub async fn download(&self, url: &str, filename: &str) -> Result<Vec<u8>, OrbitError> {
        let mut current = Url::parse(url).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "invalid download URL for '{filename}': {error}"
            ))
        })?;

        for redirect_count in 0..=MAX_REDIRECTS {
            let response = self
                .request(&current)?
                .send()
                .await
                .map_err(OrbitError::Network)?;
            let status = response.status();
            if status.is_redirection() {
                if redirect_count == MAX_REDIRECTS {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "download of '{filename}' exceeded {MAX_REDIRECTS} redirects"
                    )));
                }
                let location = response.headers().get(LOCATION).ok_or_else(|| {
                    OrbitError::Other(anyhow::anyhow!(
                        "download of '{filename}' returned HTTP {status} without a Location header"
                    ))
                })?;
                let location = location.to_str().map_err(|error| {
                    OrbitError::Other(anyhow::anyhow!(
                        "download of '{filename}' returned an invalid redirect: {error}"
                    ))
                })?;
                current = current.join(location).map_err(|error| {
                    OrbitError::Other(anyhow::anyhow!(
                        "download of '{filename}' returned an invalid redirect URL: {error}"
                    ))
                })?;
                continue;
            }
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                let body: String = body.chars().take(500).collect();
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "download of '{filename}' failed with HTTP {status}: {body}"
                )));
            }
            return response
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(OrbitError::Network);
        }

        Err(OrbitError::Other(anyhow::anyhow!(
            "download of '{filename}' exceeded {MAX_REDIRECTS} redirects"
        )))
    }

    fn request(&self, url: &Url) -> Result<reqwest::RequestBuilder, OrbitError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "unsupported artifact download URL scheme '{}'",
                url.scheme()
            )));
        }
        let mut request = self.http.get(url.clone());
        if let Some(authorization) = &self.authorization {
            authorization.validate_url(url)?;
            request = request.header(
                authorization.header_name.clone(),
                authorization.header_value.clone(),
            );
        }
        Ok(request)
    }
}

impl ScopedAuthorization {
    fn validate_url(&self, url: &Url) -> Result<(), OrbitError> {
        if url.scheme() != "https" {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "refusing to send an artifact download credential over non-HTTPS URL"
            )));
        }
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if host != self.trusted_domain && !host.ends_with(&format!(".{}", self.trusted_domain)) {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "refusing to send an artifact download credential to untrusted host '{host}'"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_downloads_scope_credentials_to_https_domain() {
        let client = ArtifactDownloadClient::authenticated_for_domain(
            "orbit-test",
            "x-api-key",
            "secret",
            "forgecdn.net",
        )
        .unwrap();
        let trusted = Url::parse("https://edge.forgecdn.net/files/example.jar").unwrap();
        let request = client.request(&trusted).unwrap().build().unwrap();
        assert_eq!(
            request
                .headers()
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("secret")
        );

        let untrusted = Url::parse("https://example.invalid/example.jar").unwrap();
        assert!(client.request(&untrusted).is_err());
        let insecure = Url::parse("http://edge.forgecdn.net/files/example.jar").unwrap();
        assert!(client.request(&insecure).is_err());
    }

    #[test]
    fn anonymous_downloads_do_not_add_credentials() {
        let client = ArtifactDownloadClient::anonymous("orbit-test").unwrap();
        let url = Url::parse("https://cdn.modrinth.com/example.jar").unwrap();
        let request = client.request(&url).unwrap().build().unwrap();
        assert!(request.headers().get("x-api-key").is_none());
    }
}
