//! Replay-safe metadata reads only. Authentication requests deliberately do not
//! go through this entry point; their semantics belong to the auth protocol.
use crate::LauncherError;

pub(crate) fn error(error: orbit_http::Error) -> LauncherError {
    match error {
        orbit_http::Error::Network(error) => LauncherError::Network(error),
        error => LauncherError::InvalidRemoteData(error.to_string()),
    }
}

pub(crate) async fn fetch_bounded(
    client: &crate::download_source::DownloadClient,
    url: &str,
    maximum: u64,
    subject: &str,
) -> Result<Vec<u8>, LauncherError> {
    Ok(fetch_response(client, url, maximum, subject).await?.body)
}

/// Mirrors may reserialize JSON. Byte-digest-pinned metadata is always read
/// from its declared origin, rather than relaxing validation or falling back.
pub(crate) async fn fetch_integrity_metadata(
    client: &crate::download_source::DownloadClient,
    url: &str,
    maximum: u64,
    subject: &str,
) -> Result<Vec<u8>, LauncherError> {
    let origin = crate::download_source::DownloadClient::new(
        client.official_client().clone(),
        Default::default(),
    );
    fetch_bounded(&origin, url, maximum, subject).await
}

pub(crate) async fn fetch_response(
    client: &crate::download_source::DownloadClient,
    url: &str,
    maximum: u64,
    subject: &str,
) -> Result<orbit_http::BufferedResponse, LauncherError> {
    let parsed = reqwest::Url::parse(url).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("{subject} URL is invalid: {error}"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} URL must use HTTPS"
        )));
    }
    let response = orbit_http::read(client.get(parsed), 3, maximum)
        .await
        .map_err(error)?;
    if response.url.scheme() != "https" {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} redirected to a non-HTTPS URL"
        )));
    }
    if !response.status.is_success() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} returned HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        )));
    }
    Ok(response)
}
