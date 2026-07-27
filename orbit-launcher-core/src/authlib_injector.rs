use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::artifact::{ArtifactRequest, CachedArtifact, ExpectedHash};
use crate::error::LauncherError;

pub const AUTHLIB_INJECTOR_LATEST_URL: &str =
    "https://authlib-injector.yushi.moe/artifact/latest.json";
const AUTHLIB_INJECTOR_HOST: &str = "authlib-injector.yushi.moe";
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ResolvedAuthlibInjector {
    pub build_number: u32,
    pub version: String,
    pub request: ArtifactRequest,
    pub target: String,
}

pub async fn resolve_authlib_injector(
    client: &reqwest::Client,
) -> Result<ResolvedAuthlibInjector, LauncherError> {
    let response = client
        .get(AUTHLIB_INJECTOR_LATEST_URL)
        .send()
        .await?
        .error_for_status()?;
    validate_official_url(response.url(), "metadata")?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES as u64)
    {
        return Err(LauncherError::InvalidRemoteData(
            "authlib-injector metadata is too large".to_string(),
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(LauncherError::InvalidRemoteData(
            "authlib-injector metadata is too large".to_string(),
        ));
    }
    let latest: LatestArtifact = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse authlib-injector latest artifact metadata: {error}"
        ))
    })?;
    latest.validate()?;
    let url = url::Url::parse(&latest.download_url).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "authlib-injector download URL is invalid: {error}"
        ))
    })?;
    validate_official_url(&url, "artifact")?;
    let sha256 = latest.checksums.get("sha256").cloned().ok_or_else(|| {
        LauncherError::InvalidRemoteData(
            "authlib-injector metadata does not publish a SHA-256 checksum".to_string(),
        )
    })?;
    validate_sha256(&sha256)?;
    let target = format!(
        "libraries/moe/yushi/authlib-injector/{0}/authlib-injector-{0}.jar",
        latest.version
    );
    Ok(ResolvedAuthlibInjector {
        build_number: latest.build_number,
        version: latest.version.clone(),
        request: ArtifactRequest {
            logical_name: format!("authlib-injector {}", latest.version),
            url: latest.download_url,
            expected_hash: ExpectedHash::Sha256(sha256),
            expected_size: None,
        },
        target,
    })
}

pub fn verify_authlib_injector(
    path: &Path,
    resolved: &ResolvedAuthlibInjector,
    artifact: &CachedArtifact,
) -> Result<(), LauncherError> {
    if artifact.size > MAX_ARTIFACT_BYTES {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "authlib-injector artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
        )));
    }
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        LauncherError::ArtifactIntegrity(format!("authlib-injector is not a valid JAR: {error}"))
    })?;
    let mut manifest = archive.by_name("META-INF/MANIFEST.MF").map_err(|error| {
        LauncherError::ArtifactIntegrity(format!("authlib-injector JAR has no manifest: {error}"))
    })?;
    let mut content = String::new();
    manifest.read_to_string(&mut content)?;
    let attributes = parse_manifest(&content)?;
    let title = attributes.get("Implementation-Title").map(String::as_str);
    let version = attributes.get("Implementation-Version").map(String::as_str);
    let build = attributes
        .get("Build-Number")
        .and_then(|value| value.parse::<u32>().ok());
    let premain = attributes.get("Premain-Class").map(String::as_str);
    if title != Some("authlib-injector")
        || version != Some(resolved.version.as_str())
        || build != Some(resolved.build_number)
        || premain != Some("moe.yushi.authlibinjector.Premain")
    {
        return Err(LauncherError::ArtifactIntegrity(
            "authlib-injector JAR identity does not match its official metadata".to_string(),
        ));
    }
    Ok(())
}

fn parse_manifest(content: &str) -> Result<BTreeMap<String, String>, LauncherError> {
    let mut attributes: BTreeMap<String, String> = BTreeMap::new();
    let mut current_key: Option<String> = None;
    for raw_line in content.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(continuation) = line.strip_prefix(' ') {
            let key = current_key.as_ref().ok_or_else(|| {
                LauncherError::ArtifactIntegrity(
                    "authlib-injector manifest begins with a continuation line".to_string(),
                )
            })?;
            attributes
                .get_mut(key)
                .expect("current manifest key was inserted")
                .push_str(continuation);
        } else if line.is_empty() {
            current_key = None;
        } else {
            let (key, value) = line.split_once(": ").ok_or_else(|| {
                LauncherError::ArtifactIntegrity(
                    "authlib-injector manifest contains an invalid attribute".to_string(),
                )
            })?;
            if key.is_empty()
                || attributes
                    .insert(key.to_string(), value.to_string())
                    .is_some()
            {
                return Err(LauncherError::ArtifactIntegrity(
                    "authlib-injector manifest contains a duplicate or empty attribute".to_string(),
                ));
            }
            current_key = Some(key.to_string());
        }
    }
    Ok(attributes)
}

fn validate_official_url(url: &url::Url, subject: &str) -> Result<(), LauncherError> {
    if url.scheme() != "https"
        || url.host_str() != Some(AUTHLIB_INJECTOR_HOST)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "authlib-injector {subject} must use its official HTTPS host"
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), LauncherError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidRemoteData(
            "authlib-injector SHA-256 checksum is invalid".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LatestArtifact {
    build_number: u32,
    version: String,
    release_time: String,
    download_url: String,
    checksums: BTreeMap<String, String>,
}

impl LatestArtifact {
    fn validate(&self) -> Result<(), LauncherError> {
        if self.build_number == 0
            || self.version.is_empty()
            || self.version.trim() != self.version
            || self.version.contains(['/', '\\'])
            || self.version.chars().any(char::is_control)
            || self.release_time.trim().is_empty()
        {
            return Err(LauncherError::InvalidRemoteData(
                "authlib-injector metadata contains an invalid build identity".to_string(),
            ));
        }
        Ok(())
    }
}

fn path_from_portable(value: &str) -> PathBuf {
    value.split('/').collect()
}

pub fn resolved_target_path(root: &Path, resolved: &ResolvedAuthlibInjector) -> PathBuf {
    root.join(path_from_portable(&resolved.target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parser_joins_continuation_lines() {
        let parsed = parse_manifest("Manifest-Version: 1.0\r\nLong: first\r\n second\r\n").unwrap();
        assert_eq!(parsed["Long"], "firstsecond");
    }

    #[tokio::test]
    #[ignore = "uses the live official authlib-injector artifact service"]
    async fn live_metadata_resolves_a_sha256_verified_artifact() {
        let client = reqwest::Client::new();
        let resolved = resolve_authlib_injector(&client).await.unwrap();
        assert!(matches!(
            resolved.request.expected_hash,
            ExpectedHash::Sha256(_)
        ));
        let directory = tempfile::tempdir().unwrap();
        let artifact = crate::ArtifactCache::new(directory.path())
            .fetch(&client, &resolved.request, |_| {})
            .await
            .unwrap();
        verify_authlib_injector(&artifact.object_path, &resolved, &artifact).unwrap();
    }
}
