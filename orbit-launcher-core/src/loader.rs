use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::artifact::{ArtifactRequest, ExpectedHash};
use crate::client::ClientDownload;
use crate::error::LauncherError;
use crate::instance::LoaderKind;
use crate::lockfile::ArtifactOwner;
use crate::maven::artifact_url;
use crate::versions::LoaderVersion;

const FABRIC_META_ROOT: &str = "https://meta.fabricmc.net/v2/versions/loader";
const QUILT_META_ROOT: &str = "https://meta.quiltmc.org/v3/versions/loader";
const MAX_PROFILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderSide {
    Client,
    Server,
}

#[derive(Debug, Clone)]
pub struct ResolvedLoaderProfile {
    pub kind: LoaderKind,
    pub version: String,
    pub profile_url: String,
    pub profile_sha256: String,
    pub main_class: String,
    pub game_arguments: Vec<String>,
    pub jvm_arguments: Vec<String>,
    pub downloads: Vec<ClientDownload>,
    pub classpath: Vec<String>,
    pub minimum_java_major: Option<u32>,
}

pub(crate) async fn list_profile_loader_versions(
    client: &reqwest::Client,
    kind: LoaderKind,
    minecraft_version: &str,
) -> Result<Vec<LoaderVersion>, LauncherError> {
    validate_path_segment(minecraft_version, "Minecraft version")?;
    match kind {
        LoaderKind::Fabric => list_fabric_versions(client, minecraft_version).await,
        LoaderKind::Quilt => list_quilt_versions(client, minecraft_version).await,
        _ => Err(LauncherError::UnsupportedRequirement(format!(
            "Loader '{}' does not use a launcher profile adapter",
            kind.as_str()
        ))),
    }
}

pub async fn resolve_loader_profile(
    client: &reqwest::Client,
    kind: LoaderKind,
    minecraft_version: &str,
    requirement: &str,
    side: LoaderSide,
) -> Result<ResolvedLoaderProfile, LauncherError> {
    validate_path_segment(minecraft_version, "Minecraft version")?;
    validate_requirement(requirement)?;
    let (version, minimum_java_major) = match kind {
        LoaderKind::Fabric => {
            resolve_fabric_version(client, minecraft_version, requirement).await?
        }
        LoaderKind::Quilt => (
            resolve_quilt_version(client, minecraft_version, requirement).await?,
            None,
        ),
        _ => {
            return Err(LauncherError::UnsupportedRequirement(format!(
                "Loader '{}' does not use a launcher profile adapter",
                kind.as_str()
            )));
        }
    };
    let profile_url = profile_url(kind, minecraft_version, &version, side)?;
    let profile_bytes =
        fetch_bounded(client, &profile_url, MAX_PROFILE_BYTES, "Loader profile").await?;
    let profile_sha256 = hex::encode(Sha256::digest(&profile_bytes));
    let profile: LauncherProfile = serde_json::from_slice(&profile_bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse {} profile for Minecraft {minecraft_version}: {error}",
            kind.as_str()
        ))
    })?;
    if profile.inherits_from != minecraft_version {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{} profile inherits from '{}' instead of resolved Minecraft '{minecraft_version}'",
            kind.as_str(),
            profile.inherits_from
        )));
    }
    validate_text(&profile.id, "Loader profile ID")?;
    validate_text(&profile.main_class, "Loader main class")?;
    let game_arguments = string_arguments(&profile.arguments.game, "Loader game arguments")?;
    let jvm_arguments = string_arguments(&profile.arguments.jvm, "Loader JVM arguments")?;

    let mut resolved = BTreeMap::new();
    for library in profile.libraries {
        let download = resolve_profile_library(client, library).await?;
        match resolved.entry(download.target.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(download);
            }
            std::collections::btree_map::Entry::Occupied(entry)
                if entry.get().request.url == download.request.url
                    && entry.get().request.expected_hash == download.request.expected_hash => {}
            std::collections::btree_map::Entry::Occupied(entry) => {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "{} profile maps conflicting libraries to '{}'",
                    kind.as_str(),
                    entry.key()
                )));
            }
        }
    }
    let downloads: Vec<_> = resolved.into_values().collect();
    let classpath = downloads
        .iter()
        .map(|download| download.target.clone())
        .collect();
    Ok(ResolvedLoaderProfile {
        kind,
        version,
        profile_url,
        profile_sha256,
        main_class: profile.main_class,
        game_arguments,
        jvm_arguments,
        downloads,
        classpath,
        minimum_java_major,
    })
}

async fn resolve_fabric_version(
    client: &reqwest::Client,
    minecraft: &str,
    requirement: &str,
) -> Result<(String, Option<u32>), LauncherError> {
    let versions = list_fabric_versions(client, minecraft).await?;
    let selected = match requirement {
        "latest" => versions.iter().find(|entry| entry.latest),
        "stable" => versions.iter().find(|entry| entry.stable),
        exact => versions.iter().find(|entry| entry.version == exact),
    }
    .ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Fabric Loader requirement '{requirement}' has no version for Minecraft {minecraft}"
        ))
    })?;
    Ok((selected.version.clone(), selected.minimum_java_major))
}

async fn list_fabric_versions(
    client: &reqwest::Client,
    minecraft: &str,
) -> Result<Vec<LoaderVersion>, LauncherError> {
    let url = endpoint(FABRIC_META_ROOT, &[minecraft])?;
    let bytes = fetch_bounded(client, &url, MAX_PROFILE_BYTES, "Fabric Loader versions").await?;
    let versions: Vec<FabricMetadata> = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse Fabric Loader versions for Minecraft {minecraft}: {error}"
        ))
    })?;
    Ok(versions
        .into_iter()
        .enumerate()
        .map(|(index, entry)| LoaderVersion {
            version: entry.loader.version,
            stable: entry.loader.stable,
            recommended: false,
            latest: index == 0,
            minimum_java_major: entry.launcher_meta.min_java_version,
        })
        .collect())
}

async fn resolve_quilt_version(
    client: &reqwest::Client,
    minecraft: &str,
    requirement: &str,
) -> Result<String, LauncherError> {
    if requirement == "stable" {
        return Err(LauncherError::UnsupportedRequirement(
            "Quilt Meta does not publish a stable flag for Loader versions; use 'latest' or an exact version"
                .to_string(),
        ));
    }
    let versions = list_quilt_versions(client, minecraft).await?;
    let selected = match requirement {
        "latest" => versions.iter().find(|entry| entry.latest),
        exact => versions.iter().find(|entry| entry.version == exact),
    };
    selected.map(|entry| entry.version.clone()).ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Quilt Loader requirement '{requirement}' has no version for Minecraft {minecraft}"
        ))
    })
}

async fn list_quilt_versions(
    client: &reqwest::Client,
    minecraft: &str,
) -> Result<Vec<LoaderVersion>, LauncherError> {
    let compatible_url = endpoint(QUILT_META_ROOT, &[minecraft])?;
    let compatible_bytes = fetch_bounded(
        client,
        &compatible_url,
        MAX_PROFILE_BYTES,
        "Quilt Loader versions",
    )
    .await?;
    let compatible: Vec<QuiltMetadata> =
        serde_json::from_slice(&compatible_bytes).map_err(|error| {
            LauncherError::InvalidRemoteData(format!(
                "failed to parse Quilt Loader versions for Minecraft {minecraft}: {error}"
            ))
        })?;
    let compatible: HashSet<_> = compatible
        .into_iter()
        .map(|entry| entry.loader.version)
        .collect();
    let all_bytes = fetch_bounded(
        client,
        QUILT_META_ROOT,
        MAX_PROFILE_BYTES,
        "Quilt Loader index",
    )
    .await?;
    let all: Vec<QuiltLoaderVersion> = serde_json::from_slice(&all_bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse the ordered Quilt Loader index: {error}"
        ))
    })?;
    Ok(all
        .into_iter()
        .map(|entry| entry.version)
        .filter(|version| compatible.contains(version))
        .enumerate()
        .map(|(index, version)| LoaderVersion {
            version,
            stable: false,
            recommended: false,
            latest: index == 0,
            minimum_java_major: None,
        })
        .collect())
}

async fn resolve_profile_library(
    client: &reqwest::Client,
    library: ProfileLibrary,
) -> Result<ClientDownload, LauncherError> {
    let (path, url) = artifact_url(&library.url, &library.name, None)?;
    let expected_hash = if let Some(sha256) = library.sha256 {
        ExpectedHash::Sha256(sha256)
    } else if let Some(sha1) = library.sha1 {
        ExpectedHash::Sha1(sha1)
    } else {
        ExpectedHash::Sha1(fetch_sha1_sidecar(client, &url, &library.name).await?)
    };
    let request = ArtifactRequest {
        logical_name: format!("Loader library {}", library.name),
        url,
        expected_hash,
        expected_size: library.size,
    };
    request.validate()?;
    Ok(ClientDownload {
        request,
        target: format!("libraries/{path}"),
        owner: ArtifactOwner::Loader,
        native_extract: None,
    })
}

async fn fetch_sha1_sidecar(
    client: &reqwest::Client,
    artifact_url: &str,
    coordinate: &str,
) -> Result<String, LauncherError> {
    let sidecar = format!("{artifact_url}.sha1");
    let bytes = fetch_bounded(client, &sidecar, 1024, "Maven SHA-1 sidecar").await?;
    let value = std::str::from_utf8(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "Maven SHA-1 sidecar for '{coordinate}' is not UTF-8: {error}"
        ))
    })?;
    let value = value.trim().to_ascii_lowercase();
    if value.len() != 40
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Maven SHA-1 sidecar for '{coordinate}' is invalid"
        )));
    }
    Ok(value)
}

fn string_arguments(
    values: &[serde_json::Value],
    subject: &str,
) -> Result<Vec<String>, LauncherError> {
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.chars().any(char::is_control))
                .map(str::to_string)
                .ok_or_else(|| {
                    LauncherError::InvalidRemoteData(format!(
                        "{subject} contain an unsupported non-string value"
                    ))
                })
        })
        .collect()
}

fn profile_url(
    kind: LoaderKind,
    minecraft: &str,
    version: &str,
    side: LoaderSide,
) -> Result<String, LauncherError> {
    let root = match kind {
        LoaderKind::Fabric => FABRIC_META_ROOT,
        LoaderKind::Quilt => QUILT_META_ROOT,
        _ => unreachable!("profile URL is only called for profile loaders"),
    };
    let suffix = match side {
        LoaderSide::Client => ["profile", "json"],
        LoaderSide::Server => ["server", "json"],
    };
    endpoint(root, &[minecraft, version, suffix[0], suffix[1]])
}

fn endpoint(root: &str, segments: &[&str]) -> Result<String, LauncherError> {
    let mut url = url::Url::parse(root).expect("hard-coded Loader Meta root is valid");
    {
        let mut path = url.path_segments_mut().map_err(|_| {
            LauncherError::InvalidRemoteData("Loader Meta base URL cannot be extended".to_string())
        })?;
        path.pop_if_empty();
        for segment in segments {
            validate_path_segment(segment, "Loader Meta path")?;
            path.push(segment);
        }
    }
    Ok(url.to_string())
}

async fn fetch_bounded(
    client: &reqwest::Client,
    url: &str,
    maximum: u64,
    subject: &str,
) -> Result<Vec<u8>, LauncherError> {
    let parsed = url::Url::parse(url).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("{subject} URL is invalid: {error}"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} URL must use HTTPS"
        )));
    }
    let response = client.get(parsed).send().await?.error_for_status()?;
    if response.url().scheme() != "https"
        || response.content_length().is_some_and(|size| size > maximum)
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} response URL or size is invalid"
        )));
    }
    let bytes = response.bytes().await?;
    if bytes.len() as u64 > maximum {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} exceeds {maximum} bytes"
        )));
    }
    Ok(bytes.to_vec())
}

fn validate_requirement(value: &str) -> Result<(), LauncherError> {
    if matches!(value, "latest" | "stable") {
        return Ok(());
    }
    validate_path_segment(value, "Loader version")
}

fn validate_path_segment(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 160
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
        })
    {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "{subject} '{value}' is not a supported exact identifier"
        )));
    }
    Ok(())
}

fn validate_text(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} is invalid"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct FabricMetadata {
    loader: FabricLoaderVersion,
    #[serde(rename = "launcherMeta")]
    launcher_meta: FabricLauncherMeta,
}

#[derive(Debug, Deserialize)]
struct FabricLoaderVersion {
    version: String,
    stable: bool,
}

#[derive(Debug, Deserialize)]
struct FabricLauncherMeta {
    #[serde(rename = "min_java_version")]
    min_java_version: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct QuiltMetadata {
    loader: QuiltLoaderVersion,
}

#[derive(Debug, Deserialize)]
struct QuiltLoaderVersion {
    version: String,
}

#[derive(Debug, Deserialize)]
struct LauncherProfile {
    id: String,
    #[serde(rename = "inheritsFrom")]
    inherits_from: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    #[serde(default)]
    arguments: ProfileArguments,
    #[serde(default)]
    libraries: Vec<ProfileLibrary>,
}

#[derive(Debug, Default, Deserialize)]
struct ProfileArguments {
    #[serde(default)]
    game: Vec<serde_json::Value>,
    #[serde(default)]
    jvm: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ProfileLibrary {
    name: String,
    url: String,
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_endpoints_encode_only_valid_exact_identifiers() {
        assert_eq!(
            profile_url(LoaderKind::Fabric, "1.21.1", "0.16.14", LoaderSide::Client).unwrap(),
            "https://meta.fabricmc.net/v2/versions/loader/1.21.1/0.16.14/profile/json"
        );
        assert!(profile_url(LoaderKind::Quilt, "../escape", "0.27.1", LoaderSide::Server).is_err());
    }

    #[tokio::test]
    #[ignore = "uses live Fabric and Quilt metadata services"]
    async fn live_profiles_resolve_verified_library_queues() {
        let client = reqwest::Client::builder()
            .user_agent("orbit-launcher-tests/0.1")
            .build()
            .unwrap();
        for (kind, requirement) in [
            (LoaderKind::Fabric, "stable"),
            (LoaderKind::Quilt, "latest"),
        ] {
            let profile =
                resolve_loader_profile(&client, kind, "1.21.1", requirement, LoaderSide::Client)
                    .await
                    .unwrap();
            assert!(!profile.downloads.is_empty());
            assert_eq!(profile.downloads.len(), profile.classpath.len());
            assert!(profile.downloads.iter().all(|download| {
                !matches!(download.request.expected_hash, ExpectedHash::Unverified)
            }));
        }
    }
}
