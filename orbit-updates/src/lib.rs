//! Read-only suite updates. No download, installation, storage or frontend policy.
use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};

pub const REPOSITORY: &str = "https://github.com/water2004/orbit";
const LATEST: &str = "https://api.github.com/repos/water2004/orbit/releases/latest";
const MAX_RESPONSE: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("update check failed: {0}")]
    Transport(#[from] orbit_http::Error),
    #[error("GitHub Release check returned HTTP {status}{detail}")]
    Http { status: u16, detail: String },
    #[error("invalid GitHub Release response: {0}")]
    InvalidRelease(String),
    #[error("invalid installed version: {0}")]
    InvalidVersion(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    UpdateAvailable,
    UpToDate,
    Ahead,
    NoRelease,
}

#[derive(Debug, Serialize)]
pub struct UpdateCheck {
    pub current_version: String,
    pub status: Status,
    pub release: Option<Release>,
}

#[derive(Debug, Serialize)]
pub struct Release {
    pub version: String,
    pub name: String,
    pub url: String,
    pub published_at: String,
    pub notes: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    html_url: String,
    published_at: String,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
}

/// GitHub's latest published, non-prerelease release. Explicit user action only.
/// The caller supplies its normal HTTP client (timeouts/proxy), never credentials.
pub async fn check(
    client: &Client,
    current_version: &str,
    retries: u32,
) -> Result<UpdateCheck, Error> {
    check_at(client, current_version, retries, LATEST).await
}

async fn check_at(
    client: &Client,
    current: &str,
    retries: u32,
    endpoint: &str,
) -> Result<UpdateCheck, Error> {
    let installed = Version::parse(current).map_err(|e| Error::InvalidVersion(e.to_string()))?;
    let response = orbit_http::read(
        client
            .get(endpoint)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(
                "User-Agent",
                concat!("orbit-updates/", env!("CARGO_PKG_VERSION")),
            ),
        retries,
        MAX_RESPONSE,
    )
    .await?;
    let endpoint_url =
        reqwest::Url::parse(endpoint).map_err(|e| Error::InvalidRelease(e.to_string()))?;
    if response.url.origin() != endpoint_url.origin() {
        return Err(Error::InvalidRelease(
            "GitHub API redirected outside its origin".into(),
        ));
    }
    if response.status == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateCheck {
            current_version: current.into(),
            status: Status::NoRelease,
            release: None,
        });
    }
    if !response.status.is_success() {
        let detail = if response.status == reqwest::StatusCode::FORBIDDEN
            || response.status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            "; GitHub API access was denied or rate-limited; try again later".into()
        } else {
            String::new()
        };
        return Err(Error::Http {
            status: response.status.as_u16(),
            detail,
        });
    }
    let release: GitHubRelease =
        serde_json::from_slice(&response.body).map_err(|e| Error::InvalidRelease(e.to_string()))?;
    interpret(installed, release)
}

fn interpret(installed: Version, remote: GitHubRelease) -> Result<UpdateCheck, Error> {
    if remote.draft || remote.prerelease {
        return Err(Error::InvalidRelease(
            "latest release is not a published stable release".into(),
        ));
    }
    let version = Version::parse(
        remote
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&remote.tag_name),
    )
    .map_err(|e| Error::InvalidRelease(e.to_string()))?;
    if !version.pre.is_empty() {
        return Err(Error::InvalidRelease(
            "stable release tag contains a prerelease version".into(),
        ));
    }
    let url =
        reqwest::Url::parse(&remote.html_url).map_err(|e| Error::InvalidRelease(e.to_string()))?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with("/water2004/orbit/releases/tag/")
    {
        return Err(Error::InvalidRelease(
            "release link is outside the Orbit repository".into(),
        ));
    }
    let status = match version.cmp_precedence(&installed) {
        std::cmp::Ordering::Greater => Status::UpdateAvailable,
        std::cmp::Ordering::Equal => Status::UpToDate,
        std::cmp::Ordering::Less => Status::Ahead,
    };
    Ok(UpdateCheck {
        current_version: installed.to_string(),
        status,
        release: Some(Release {
            version: version.to_string(),
            name: remote
                .name
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(remote.tag_name),
            url: url.to_string(),
            published_at: remote.published_at,
            notes: remote.body.unwrap_or_default(),
        }),
    })
}

#[cfg(test)]
mod tests;
