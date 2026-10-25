//! External Yggdrasil endpoint discovery and exact API-root handling.
//!
//! User-entered addresses are resolved once, when a provider is added. All
//! authentication and launch paths then consume the persisted API root
//! without probing alternate routes.

use reqwest::header::HeaderValue;

use crate::config::YggdrasilProviderConfig;
use crate::error::LauncherError;
mod input;
use input::parse_entered_address;

pub(crate) const MAX_METADATA_BYTES: usize = 1024 * 1024;
const API_LOCATION_HEADER: &str = "x-authlib-injector-api-location";

/// Read-only endpoint discovery result. This contains no account credentials.
#[derive(Debug, Clone)]
pub struct YggdrasilEndpoint {
    pub api_root: String,
    pub server_name: Option<String>,
    pub suggested_id: String,
    pub existing_provider_id: Option<String>,
    pub insecure_http: bool,
}

/// Decode standard drag input when present, follow ALI, and validate metadata.
/// This never saves configuration or authenticates an account.
pub async fn inspect_yggdrasil_provider(
    client: &reqwest::Client,
    providers: &[YggdrasilProviderConfig],
    address: &str,
    allow_insecure_http: bool,
) -> Result<YggdrasilEndpoint, LauncherError> {
    let entered = parse_entered_address(address, allow_insecure_http)?;
    let response = orbit_http::read(client.get(entered), 3, MAX_METADATA_BYTES as u64)
        .await
        .map_err(crate::network::error)?;
    let status = response.status;
    let response_url = response.url.clone();
    if !status.is_success() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Yggdrasil endpoint discovery failed with HTTP {} at '{}'",
            status.as_u16(),
            response.url
        )));
    }

    let location = response.headers.get(API_LOCATION_HEADER).cloned();
    let api_root = resolve_api_location(&response_url, location.as_ref(), allow_insecure_http)?;
    let metadata = if api_root == normalized_api_url(response_url, allow_insecure_http)? {
        metadata_value(response).await?
    } else {
        let metadata = orbit_http::read(client.get(api_root.clone()), 3, MAX_METADATA_BYTES as u64)
            .await
            .map_err(crate::network::error)?;
        metadata_value(metadata).await?
    };
    Ok(endpoint_description(api_root, &metadata, providers))
}

fn endpoint_description(
    api_root: url::Url,
    metadata: &serde_json::Value,
    providers: &[YggdrasilProviderConfig],
) -> YggdrasilEndpoint {
    let server_name = metadata
        .get("meta")
        .and_then(|meta| meta.get("serverName"))
        .and_then(|name| name.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            name.chars()
                .filter(|character| !character.is_control())
                .take(256)
                .collect::<String>()
        })
        .filter(|name| !name.is_empty());
    let name = server_name
        .as_deref()
        .unwrap_or_else(|| api_root.host_str().unwrap_or("Yggdrasil"));
    let mut suggested_id = String::new();
    for character in name.chars().filter(|character| !character.is_control()) {
        if suggested_id.len() + character.len_utf8() > 64 {
            break;
        }
        suggested_id.push(character);
    }
    suggested_id = suggested_id.trim().to_owned();
    if suggested_id.is_empty() {
        suggested_id = "Yggdrasil".into();
    }
    let existing_provider_id = providers
        .iter()
        .find(|provider| provider_api_root(provider).is_ok_and(|url| url == api_root))
        .map(|provider| provider.id.clone());
    YggdrasilEndpoint {
        insecure_http: api_root.scheme() == "http",
        api_root: api_root.to_string(),
        server_name,
        suggested_id,
        existing_provider_id,
    }
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
    let response = orbit_http::read(
        client.get(provider_api_root(provider)?),
        3,
        MAX_METADATA_BYTES as u64,
    )
    .await
    .map_err(crate::network::error)?;
    metadata_json(response).await.map_err(|error| match error {
        LauncherError::InvalidRemoteData(message) => LauncherError::Authentication(message),
        other => other,
    })
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

async fn metadata_json(response: orbit_http::BufferedResponse) -> Result<String, LauncherError> {
    let metadata = metadata_value(response).await?;
    serde_json::to_string(&metadata).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to preserve Yggdrasil provider metadata: {error}"
        ))
    })
}

async fn metadata_value(
    response: orbit_http::BufferedResponse,
) -> Result<serde_json::Value, LauncherError> {
    let status = response.status;
    let response_url = response.url;
    let bytes = response.body;
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
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviated_addresses_default_to_https() {
        let url = parse_entered_address("accounts.example.test/api/yggdrasil", false).unwrap();
        assert_eq!(url.as_str(), "https://accounts.example.test/api/yggdrasil/");
        assert_eq!(
            parse_entered_address("accounts.example.test:8443/api", false)
                .unwrap()
                .as_str(),
            "https://accounts.example.test:8443/api/"
        );
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

    #[test]
    fn provider_names_are_safe_bounded_identifiers_and_existing_roots_are_reused() {
        let root = url::Url::parse("https://accounts.example.test/api/").unwrap();
        let providers = vec![YggdrasilProviderConfig {
            id: "known".into(),
            api_root: root.to_string(),
            allow_insecure_http: false,
        }];
        let endpoint = endpoint_description(
            root.clone(),
            &serde_json::json!({"meta":{"serverName": "\u{1b}".to_string() + &"名".repeat(100)}}),
            &providers,
        );
        assert_eq!(endpoint.existing_provider_id.as_deref(), Some("known"));
        assert!(endpoint.suggested_id.len() <= 64);
        assert!(
            !endpoint
                .server_name
                .as_ref()
                .unwrap()
                .chars()
                .any(char::is_control)
        );
        assert!(!endpoint.insecure_http);
        let unnamed = endpoint_description(root, &serde_json::json!({}), &[]);
        assert_eq!(unnamed.suggested_id, "accounts.example.test");
    }

    #[tokio::test]
    async fn encoded_drag_input_and_plain_input_share_ali_discovery_and_metadata() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut paths = Vec::new();
            for _ in 0..4 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 4096];
                    let size = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(size, 0);
                    request.extend_from_slice(&buffer[..size]);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.lines().next().unwrap().to_string();
                assert!(path.starts_with("GET "));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                let (headers, body) = if path == "GET /landing/ HTTP/1.1" {
                    ("X-Authlib-Injector-API-Location: /api/\r\n", "")
                } else {
                    assert_eq!(path, "GET /api/ HTTP/1.1");
                    ("", r#"{"meta":{"serverName":"Example Skin"}}"#)
                };
                paths.push(path);
                let response = format!(
                    "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            paths
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let entered = format!("{root}/landing");
        let encoded = format!(
            "authlib-injector:yggdrasil-server:{}",
            percent_encoding::utf8_percent_encode(&entered, percent_encoding::NON_ALPHANUMERIC)
        );
        let first = inspect_yggdrasil_provider(&client, &[], &encoded, true)
            .await
            .unwrap();
        let providers = [YggdrasilProviderConfig {
            id: "already-added".into(),
            api_root: first.api_root.clone(),
            allow_insecure_http: true,
        }];
        let second = inspect_yggdrasil_provider(&client, &providers, &entered, true)
            .await
            .unwrap();
        assert_eq!(first.api_root, format!("{root}/api/"));
        assert_eq!(first.server_name.as_deref(), Some("Example Skin"));
        assert!(first.insecure_http);
        assert_eq!(first.api_root, second.api_root);
        assert_eq!(
            second.existing_provider_id.as_deref(),
            Some("already-added")
        );
        assert_eq!(task.await.unwrap().len(), 4);
    }
}
