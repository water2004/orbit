//! External Yggdrasil endpoint discovery and exact API-root handling.
//!
//! User-entered addresses are resolved once, when a provider is added. All
//! authentication and launch paths then consume the persisted API root
//! without probing alternate routes.

use reqwest::header::HeaderValue;

use crate::config::YggdrasilProviderConfig;
use crate::error::LauncherError;

pub(crate) const MAX_METADATA_BYTES: usize = 1024 * 1024;
const API_LOCATION_HEADER: &str = "x-authlib-injector-api-location";

/// Resolves a user-entered address through authlib-injector's API Location
/// Indication (ALI), validates the API metadata, and returns the exact root to
/// persist in launcher configuration.
pub async fn discover_yggdrasil_api_root(
    client: &reqwest::Client,
    address: &str,
    allow_insecure_http: bool,
) -> Result<String, LauncherError> {
    let entered = parse_entered_address(address, allow_insecure_http)?;
    let response = client.get(entered).send().await?;
    let status = response.status();
    let response_url = response.url().clone();
    if !status.is_success() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Yggdrasil endpoint discovery failed with HTTP {} at '{}'",
            status.as_u16(),
            response.url()
        )));
    }

    let location = response.headers().get(API_LOCATION_HEADER).cloned();
    let api_root = resolve_api_location(&response_url, location.as_ref(), allow_insecure_http)?;
    if api_root == normalized_api_url(response_url, allow_insecure_http)? {
        validate_metadata_response(response).await?;
    } else {
        let metadata = client.get(api_root.clone()).send().await?;
        validate_metadata_response(metadata).await?;
    }
    Ok(api_root.to_string())
}

pub(crate) fn provider_api_root(
    provider: &YggdrasilProviderConfig,
) -> Result<url::Url, LauncherError> {
    let parsed = url::Url::parse(&provider.api_root).map_err(|error| {
        LauncherError::InvalidConfig(format!(
            "Yggdrasil provider '{}' has an invalid API root: {error}",
            provider.id
        ))
    })?;
    normalized_api_url(parsed, provider.allow_insecure_http).map_err(|error| match error {
        LauncherError::InvalidRemoteData(message) => LauncherError::InvalidConfig(format!(
            "Yggdrasil provider '{}' has an invalid API root: {message}",
            provider.id
        )),
        other => other,
    })
}

pub(crate) async fn fetch_provider_metadata(
    client: &reqwest::Client,
    provider: &YggdrasilProviderConfig,
) -> Result<String, LauncherError> {
    let response = client.get(provider_api_root(provider)?).send().await?;
    metadata_json(response).await.map_err(|error| match error {
        LauncherError::InvalidRemoteData(message) => LauncherError::Authentication(message),
        other => other,
    })
}

fn parse_entered_address(
    address: &str,
    allow_insecure_http: bool,
) -> Result<url::Url, LauncherError> {
    let address = address.trim();
    if address.is_empty() || address.chars().any(char::is_control) {
        return Err(LauncherError::InvalidConfig(
            "Yggdrasil endpoint address is empty or contains control characters".to_string(),
        ));
    }
    let parsed = match url::Url::parse(address) {
        Ok(url) if url.has_host() => url,
        Ok(_) | Err(url::ParseError::RelativeUrlWithoutBase) => {
            url::Url::parse(&format!("https://{address}")).map_err(|error| {
                LauncherError::InvalidConfig(format!(
                    "invalid Yggdrasil endpoint address '{address}': {error}"
                ))
            })?
        }
        Err(error) => {
            return Err(LauncherError::InvalidConfig(format!(
                "invalid Yggdrasil endpoint address '{address}': {error}"
            )));
        }
    };
    normalized_api_url(parsed, allow_insecure_http)
        .map_err(|error| LauncherError::InvalidConfig(error.to_string()))
}

fn resolve_api_location(
    response_url: &url::Url,
    location: Option<&HeaderValue>,
    allow_insecure_http: bool,
) -> Result<url::Url, LauncherError> {
    let Some(location) = location else {
        return Ok(response_url.clone());
    };
    let location = location.to_str().map_err(|_| {
        LauncherError::InvalidRemoteData(
            "Yggdrasil API Location header is not valid UTF-8".to_string(),
        )
    })?;
    if location.trim().is_empty() {
        return Err(LauncherError::InvalidRemoteData(
            "Yggdrasil API Location header is empty".to_string(),
        ));
    }
    let resolved = response_url.join(location).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "Yggdrasil API Location '{location}' is invalid: {error}"
        ))
    })?;
    normalized_api_url(resolved, allow_insecure_http)
}

fn normalized_api_url(
    mut url: url::Url,
    allow_insecure_http: bool,
) -> Result<url::Url, LauncherError> {
    match url.scheme() {
        "https" => {}
        "http" if allow_insecure_http => {}
        "http" => {
            return Err(LauncherError::InvalidRemoteData(
                "Yggdrasil API root must use HTTPS unless insecure HTTP is explicitly allowed"
                    .to_string(),
            ));
        }
        scheme => {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Yggdrasil API root uses unsupported scheme '{scheme}'"
            )));
        }
    }
    if url.host_str().is_none() {
        return Err(LauncherError::InvalidRemoteData(
            "Yggdrasil API root has no host".to_string(),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(LauncherError::InvalidRemoteData(
            "Yggdrasil API root cannot contain credentials, a query, or a fragment".to_string(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

async fn validate_metadata_response(response: reqwest::Response) -> Result<(), LauncherError> {
    metadata_json(response).await.map(|_| ())
}

async fn metadata_json(response: reqwest::Response) -> Result<String, LauncherError> {
    let status = response.status();
    let response_url = response.url().clone();
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Yggdrasil provider metadata exceeds {MAX_METADATA_BYTES} bytes"
        )));
    }
    if !status.is_success() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Yggdrasil provider metadata failed with HTTP {} at '{response_url}'",
            status.as_u16()
        )));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "Yggdrasil provider metadata is invalid JSON: {error}"
        ))
    })?;
    if !metadata.is_object() {
        return Err(LauncherError::InvalidRemoteData(
            "Yggdrasil provider metadata must be a JSON object".to_string(),
        ));
    }
    serde_json::to_string(&metadata).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to preserve Yggdrasil provider metadata: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviated_addresses_default_to_https() {
        let url = parse_entered_address("accounts.example.test/api/yggdrasil", false).unwrap();
        assert_eq!(url.as_str(), "https://accounts.example.test/api/yggdrasil/");
    }

    #[test]
    fn relative_api_location_is_resolved_from_the_response_url() {
        let response = url::Url::parse("https://accounts.example.test/landing/").unwrap();
        let location = HeaderValue::from_static("../api/yggdrasil/");
        let resolved = resolve_api_location(&response, Some(&location), false).unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://accounts.example.test/api/yggdrasil/"
        );
    }

    #[test]
    fn insecure_discovered_roots_require_explicit_permission() {
        let response = url::Url::parse("https://accounts.example.test/").unwrap();
        let location = HeaderValue::from_static("http://accounts.example.test/api/yggdrasil/");
        assert!(resolve_api_location(&response, Some(&location), false).is_err());
        assert_eq!(
            resolve_api_location(&response, Some(&location), true)
                .unwrap()
                .as_str(),
            "http://accounts.example.test/api/yggdrasil/"
        );
    }
}
