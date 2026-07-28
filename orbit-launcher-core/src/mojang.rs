use serde::Deserialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;

use crate::artifact::{ArtifactRequest, ExpectedHash};
use crate::error::LauncherError;

pub const VERSION_MANIFEST_V2_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ResolvedVanillaServer {
    pub minecraft_version: String,
    pub version_type: String,
    pub version_manifest_sha256: String,
    pub version_json_url: String,
    pub version_json_sha1: String,
    pub server: ArtifactRequest,
    pub java: Option<MojangJavaRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MojangJavaRequirement {
    pub component: String,
    pub major: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftVersionCatalog {
    pub latest_release: String,
    pub latest_snapshot: String,
    pub versions: Vec<MinecraftVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftVersion {
    pub id: String,
    pub version_type: String,
    pub release_time: String,
    pub latest_release: bool,
    pub latest_snapshot: bool,
}

#[derive(Debug, Clone)]
pub struct MojangClient {
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub(crate) struct MojangVersionDocument {
    pub id: String,
    pub version_type: String,
    pub version_manifest_sha256: String,
    pub version_json_url: String,
    pub version_json_sha1: String,
    pub bytes: Vec<u8>,
}

impl MojangClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub(crate) fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn list_versions(&self) -> Result<MinecraftVersionCatalog, LauncherError> {
        let manifest_bytes = self.fetch_metadata(VERSION_MANIFEST_V2_URL).await?;
        let manifest = parse_version_manifest(&manifest_bytes)?;
        Ok(version_catalog(manifest))
    }

    pub async fn resolve_java_requirement(
        &self,
        minecraft_requirement: &str,
    ) -> Result<Option<MojangJavaRequirement>, LauncherError> {
        let document = self.fetch_version_document(minecraft_requirement).await?;
        let version: VersionJavaIdentity =
            serde_json::from_slice(&document.bytes).map_err(|error| {
                LauncherError::InvalidRemoteData(format!(
                    "failed to parse Java requirement for Minecraft '{}': {error}",
                    document.id
                ))
            })?;
        version
            .java_version
            .map(|java| {
                if java.component.trim().is_empty() || java.major_version == 0 {
                    return Err(LauncherError::InvalidRemoteData(format!(
                        "Minecraft '{}' declares an invalid Java requirement",
                        document.id
                    )));
                }
                Ok(MojangJavaRequirement {
                    component: java.component,
                    major: java.major_version,
                })
            })
            .transpose()
    }

    pub async fn resolve_vanilla_server(
        &self,
        requirement: &str,
    ) -> Result<ResolvedVanillaServer, LauncherError> {
        let document = self.fetch_version_document(requirement).await?;
        let version: VersionJson = serde_json::from_slice(&document.bytes).map_err(|error| {
            LauncherError::InvalidRemoteData(format!(
                "failed to parse Mojang version JSON '{}': {error}",
                document.id
            ))
        })?;
        let server = version.downloads.server.ok_or_else(|| {
            LauncherError::UnsupportedRequirement(format!(
                "Minecraft '{}' does not publish a dedicated server artifact",
                document.id
            ))
        })?;
        server.validate("server JAR")?;
        validate_mojang_url(
            &server.url,
            &["piston-data.mojang.com", "launcher.mojang.com"],
            "server JAR",
        )?;
        let java = version.java_version.map(|java| MojangJavaRequirement {
            component: java.component,
            major: java.major_version,
        });
        if java
            .as_ref()
            .is_some_and(|java| java.component.trim().is_empty() || java.major == 0)
        {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Minecraft '{}' declares an invalid Java requirement",
                document.id
            )));
        }
        Ok(ResolvedVanillaServer {
            minecraft_version: document.id.clone(),
            version_type: document.version_type,
            version_manifest_sha256: document.version_manifest_sha256,
            version_json_url: document.version_json_url,
            version_json_sha1: document.version_json_sha1,
            server: ArtifactRequest {
                logical_name: format!("Minecraft {} server", document.id),
                url: server.url,
                expected_hash: ExpectedHash::Sha1(server.sha1),
                expected_size: Some(server.size),
            },
            java,
        })
    }

    pub(crate) async fn fetch_version_document(
        &self,
        requirement: &str,
    ) -> Result<MojangVersionDocument, LauncherError> {
        let manifest_bytes = self.fetch_metadata(VERSION_MANIFEST_V2_URL).await?;
        let manifest_sha256 = hex::encode(Sha256::digest(&manifest_bytes));
        let manifest = parse_version_manifest(&manifest_bytes)?;
        let version_id = manifest.resolve(requirement)?;
        let entry = manifest
            .versions
            .iter()
            .find(|entry| entry.id == version_id)
            .ok_or_else(|| {
                LauncherError::InvalidRemoteData(format!(
                    "Mojang manifest points to missing version '{version_id}'"
                ))
            })?;
        validate_mojang_url(&entry.url, &["piston-meta.mojang.com"], "version JSON")?;
        validate_digest(&entry.sha1, 40, "version JSON SHA-1")?;

        let version_bytes = self.fetch_metadata(&entry.url).await?;
        let actual_version_sha1 = hex::encode(Sha1::digest(&version_bytes));
        if actual_version_sha1 != entry.sha1 {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "Mojang version JSON '{}' did not match manifest SHA-1",
                entry.id
            )));
        }
        let identity: VersionIdentity =
            serde_json::from_slice(&version_bytes).map_err(|error| {
                LauncherError::InvalidRemoteData(format!(
                    "failed to parse Mojang version JSON identity '{}': {error}",
                    entry.id
                ))
            })?;
        if identity.id != entry.id || identity.version_type != entry.version_type {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Mojang version JSON identity for '{}' disagrees with the version manifest",
                entry.id
            )));
        }
        Ok(MojangVersionDocument {
            id: entry.id.clone(),
            version_type: entry.version_type.clone(),
            version_manifest_sha256: manifest_sha256,
            version_json_url: entry.url.clone(),
            version_json_sha1: entry.sha1.clone(),
            bytes: version_bytes,
        })
    }

    async fn fetch_metadata(&self, url: &str) -> Result<Vec<u8>, LauncherError> {
        let response = self.client.get(url).send().await?.error_for_status()?;
        if response.url().scheme() != "https" {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Mojang metadata redirected to non-HTTPS URL '{}'",
                response.url()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_METADATA_BYTES)
        {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Mojang metadata '{}' exceeds {MAX_METADATA_BYTES} bytes",
                response.url()
            )));
        }
        let bytes = response.bytes().await?;
        if bytes.len() as u64 > MAX_METADATA_BYTES {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Mojang metadata '{url}' exceeds {MAX_METADATA_BYTES} bytes"
            )));
        }
        Ok(bytes.to_vec())
    }
}

#[derive(Debug, Deserialize)]
struct VersionManifest {
    latest: LatestVersions,
    versions: Vec<VersionEntry>,
}

impl VersionManifest {
    fn resolve(&self, requirement: &str) -> Result<String, LauncherError> {
        let id = match requirement {
            "latest-release" => &self.latest.release,
            "latest-snapshot" => &self.latest.snapshot,
            exact
                if !exact.is_empty()
                    && exact.trim() == exact
                    && !exact.chars().any(char::is_control) =>
            {
                exact
            }
            _ => {
                return Err(LauncherError::UnsupportedRequirement(format!(
                    "Minecraft requirement '{requirement}' must be an exact version, latest-release, or latest-snapshot"
                )));
            }
        };
        if self.versions.iter().any(|version| version.id == id) {
            Ok(id.to_string())
        } else {
            Err(LauncherError::UnsupportedRequirement(format!(
                "Minecraft version '{id}' is not present in Mojang's version manifest"
            )))
        }
    }
}

#[derive(Debug, Deserialize)]
struct LatestVersions {
    release: String,
    snapshot: String,
}

#[derive(Debug, Deserialize)]
struct VersionEntry {
    id: String,
    #[serde(rename = "type")]
    version_type: String,
    url: String,
    sha1: String,
    #[serde(rename = "releaseTime")]
    release_time: String,
}

#[derive(Debug, Deserialize)]
struct VersionJson {
    downloads: VersionDownloads,
    #[serde(rename = "javaVersion")]
    java_version: Option<JavaVersion>,
}

#[derive(Debug, Deserialize)]
struct VersionJavaIdentity {
    #[serde(rename = "javaVersion")]
    java_version: Option<JavaVersion>,
}

#[derive(Debug, Deserialize)]
struct VersionIdentity {
    id: String,
    #[serde(rename = "type")]
    version_type: String,
}

#[derive(Debug, Deserialize)]
struct VersionDownloads {
    server: Option<Download>,
}

#[derive(Debug, Deserialize)]
struct Download {
    sha1: String,
    size: u64,
    url: String,
}

impl Download {
    fn validate(&self, subject: &str) -> Result<(), LauncherError> {
        validate_digest(&self.sha1, 40, &format!("{subject} SHA-1"))?;
        if self.size == 0 {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Mojang {subject} has zero size"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct JavaVersion {
    component: String,
    #[serde(rename = "majorVersion")]
    major_version: u32,
}

fn parse_version_manifest(bytes: &[u8]) -> Result<VersionManifest, LauncherError> {
    let manifest: VersionManifest = serde_json::from_slice(bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse Mojang version manifest v2: {error}"
        ))
    })?;
    if manifest.latest.release.trim().is_empty()
        || manifest.latest.snapshot.trim().is_empty()
        || manifest.versions.iter().any(|version| {
            version.id.trim().is_empty()
                || version.version_type.trim().is_empty()
                || version.release_time.trim().is_empty()
        })
    {
        return Err(LauncherError::InvalidRemoteData(
            "Mojang version manifest contains an incomplete version entry".to_string(),
        ));
    }
    Ok(manifest)
}

fn version_catalog(manifest: VersionManifest) -> MinecraftVersionCatalog {
    let versions = manifest
        .versions
        .into_iter()
        .map(|version| MinecraftVersion {
            latest_release: version.id == manifest.latest.release,
            latest_snapshot: version.id == manifest.latest.snapshot,
            id: version.id,
            version_type: version.version_type,
            release_time: version.release_time,
        })
        .collect();
    MinecraftVersionCatalog {
        latest_release: manifest.latest.release,
        latest_snapshot: manifest.latest.snapshot,
        versions,
    }
}

fn validate_mojang_url(
    value: &str,
    allowed_hosts: &[&str],
    subject: &str,
) -> Result<(), LauncherError> {
    let url = url::Url::parse(value).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("Mojang {subject} URL is invalid: {error}"))
    })?;
    if url.scheme() != "https"
        || !url
            .host_str()
            .is_some_and(|host| allowed_hosts.contains(&host))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Mojang {subject} URL '{value}' is not an allowed official HTTPS URL"
        )));
    }
    Ok(())
}

fn validate_digest(value: &str, length: usize, subject: &str) -> Result<(), LauncherError> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Mojang {subject} '{value}' is invalid"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_resolves_only_documented_requirement_forms() {
        let manifest: VersionManifest = serde_json::from_str(
            r#"{
                "latest":{"release":"1.21.1","snapshot":"25w01a"},
                "versions":[
                    {"id":"1.21.1","type":"release","url":"https://piston-meta.mojang.com/a","sha1":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","releaseTime":"2024-08-08T12:00:00Z"},
                    {"id":"25w01a","type":"snapshot","url":"https://piston-meta.mojang.com/b","sha1":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","releaseTime":"2025-01-02T12:00:00Z"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.resolve("latest-release").unwrap(), "1.21.1");
        assert_eq!(manifest.resolve("25w01a").unwrap(), "25w01a");
        assert!(manifest.resolve("latest").is_err());
        assert!(manifest.resolve("missing").is_err());
        let catalog = version_catalog(manifest);
        assert!(catalog.versions[0].latest_release);
        assert!(catalog.versions[1].latest_snapshot);
        assert_eq!(catalog.versions[0].release_time, "2024-08-08T12:00:00Z");
    }

    #[test]
    fn official_url_validation_rejects_lookalike_hosts() {
        assert!(
            validate_mojang_url(
                "https://piston-data.mojang.com.evil.example/server.jar",
                &["piston-data.mojang.com"],
                "server JAR"
            )
            .is_err()
        );
    }
}
