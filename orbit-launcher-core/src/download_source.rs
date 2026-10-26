//! Opt-in public download routing. Canonical metadata/lock URLs stay unchanged.
//! Authentication clients never use this transport.
use crate::LauncherError;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{fmt, str::FromStr};

pub const BMCLAPI_DOCUMENTATION: &str = "https://bmclapidoc.bangbang93.com/";
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadSource {
    #[default]
    Official,
    Bmclapi,
}
impl fmt::Display for DownloadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Official => "official",
            Self::Bmclapi => "bmclapi",
        })
    }
}
impl FromStr for DownloadSource {
    type Err = LauncherError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "official" => Ok(Self::Official),
            "bmclapi" => Ok(Self::Bmclapi),
            _ => Err(LauncherError::InvalidConfig(
                "download source must be official or bmclapi".into(),
            )),
        }
    }
}
impl DownloadSource {
    pub fn attribution(self) -> Option<DownloadAttribution> {
        (self == Self::Bmclapi).then_some(DownloadAttribution {
            source: self,
            name: "BMCLAPI · by bangbang93",
            documentation: BMCLAPI_DOCUMENTATION,
        })
    }
    fn route(self, canonical: &url::Url) -> url::Url {
        if self == Self::Official
            || canonical.scheme() != "https"
            || canonical.port().is_some()
            || !canonical.username().is_empty()
            || canonical.password().is_some()
        {
            return canonical.clone();
        }
        let path = canonical.path();
        let mapped = match canonical.host_str() {
            Some(
                "launchermeta.mojang.com"
                | "launcher.mojang.com"
                | "piston-meta.mojang.com"
                | "piston-data.mojang.com",
            ) => path.to_owned(),
            Some("resources.download.minecraft.net") => format!("/assets{path}"),
            Some("libraries.minecraft.net" | "maven.fabricmc.net" | "maven.minecraftforge.net") => {
                format!("/maven{path}")
            }
            Some("files.minecraftforge.net") if path.starts_with("/maven/") => path.to_owned(),
            Some("meta.fabricmc.net") => format!("/fabric-meta{path}"),
            Some("maven.neoforged.net") if path.starts_with("/releases/") => {
                format!("/maven{}", &path["/releases".len()..])
            }
            Some("authlib-injector.yushi.moe") => format!("/mirrors/authlib-injector{path}"),
            // Quilt endpoints are explicitly documented as unavailable by BMCLAPI.
            _ => return canonical.clone(),
        };
        let mut routed = canonical.clone();
        routed
            .set_host(Some("bmclapi2.bangbang93.com"))
            .expect("static valid host");
        routed.set_path(&mapped);
        routed
    }
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DownloadAttribution {
    pub source: DownloadSource,
    pub name: &'static str,
    pub documentation: &'static str,
}
#[derive(Debug, Clone)]
pub struct DownloadClient {
    client: reqwest::Client,
    source: DownloadSource,
}
impl DownloadClient {
    pub fn new(client: reqwest::Client, source: DownloadSource) -> Self {
        Self { client, source }
    }
    pub fn get(&self, canonical: url::Url) -> Result<reqwest::RequestBuilder, LauncherError> {
        let routed = self.source.route(&canonical);
        if self.source == DownloadSource::Bmclapi
            && (routed.scheme() != "https"
                || routed.port().is_some()
                || !routed.username().is_empty()
                || routed.password().is_some()
                || routed.host_str() != Some("bmclapi2.bangbang93.com"))
        {
            return Err(LauncherError::UnsupportedRequirement(format!(
                "BMCLAPI does not provide a verified route for '{}'; select the official download source for this component",
                canonical.host_str().unwrap_or("invalid host")
            )));
        }
        Ok(self.client.get(routed))
    }
    pub(crate) fn source(&self) -> DownloadSource {
        self.source
    }
    /// Legal documents retain their official origin, independently of mirrors.
    pub fn official_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// BMCLAPI reserializes this particular JSON document. Its bytes are source
    /// metadata, not a Mojang byte-exact artifact. Keep the received digest as
    /// its identity; individual Java file digests are never relaxed.
    pub(crate) fn java_manifest_digest(
        &self,
        bytes: &[u8],
        declared: &str,
    ) -> Result<String, LauncherError> {
        let actual = hex::encode(Sha1::digest(bytes));
        if self.source == DownloadSource::Official && actual != declared {
            return Err(LauncherError::ArtifactIntegrity(
                "Mojang Java manifest did not match its declared SHA-1".into(),
            ));
        }
        Ok(actual)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "uses live BMCLAPI metadata, without installing or starting a game"]
    async fn live_bmclapi_minecraft_fabric_and_java_preserve_canonical_identity() {
        let http = reqwest::Client::builder()
            .user_agent("orbit-launcher-source-test")
            .timeout(std::time::Duration::from_secs(60))
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.url().scheme() != "https"
                    || matches!(
                        attempt.url().host_str(),
                        Some(
                            "piston-meta.mojang.com"
                                | "piston-data.mojang.com"
                                | "launchermeta.mojang.com"
                                | "launcher.mojang.com"
                                | "resources.download.minecraft.net"
                                | "libraries.minecraft.net"
                                | "meta.fabricmc.net"
                                | "maven.fabricmc.net"
                                | "authlib-injector.yushi.moe"
                        )
                    )
                {
                    attempt.error("test forbids redirects to the origin")
                } else if attempt.previous().len() >= 10 {
                    attempt.error("too many redirects")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .unwrap();
        let client = DownloadClient::new(http, DownloadSource::Bmclapi);
        let server = crate::mojang::MojangClient::new(client.clone())
            .resolve_vanilla_server("26.2")
            .await
            .unwrap();
        assert!(
            server
                .version_json_url
                .starts_with("https://piston-meta.mojang.com/")
        );
        let vanilla = crate::client::resolve_vanilla_client(
            &crate::mojang::MojangClient::new(client.clone()),
            "26.2",
            &crate::platform::HostPlatform::native().unwrap(),
        )
        .await
        .unwrap();
        assert!(!vanilla.asset_mappings.is_empty());
        assert!(
            server
                .server
                .url
                .starts_with("https://piston-data.mojang.com/")
        );
        let profile = crate::loader::resolve_loader_profile(
            &client,
            crate::instance::LoaderKind::Fabric,
            "26.2",
            "0.19.5",
            crate::loader::LoaderSide::Client,
        )
        .await
        .unwrap();
        assert!(
            profile
                .profile_url
                .starts_with("https://meta.fabricmc.net/")
        );
        let temp = tempfile::tempdir().unwrap();
        let cache = crate::artifact::ArtifactCache::new(temp.path());
        assert!(profile.downloads.iter().any(|download| matches!(
            download.request.expected_hash,
            crate::artifact::ExpectedHash::Sha256(_)
        )));
        let artifact = cache
            .fetch(&client, &profile.downloads[0].request, |_| {})
            .await
            .unwrap();
        assert!(artifact.size > 0);
        let authlib = crate::authlib_injector::resolve_authlib_injector(&client)
            .await
            .unwrap();
        let artifact = cache
            .fetch(&client, &authlib.request, |_| {})
            .await
            .unwrap();
        crate::authlib_injector::verify_authlib_injector(
            &artifact.object_path,
            &authlib,
            &artifact,
        )
        .unwrap();
        let requirement = server.java.unwrap();
        let java = crate::java::plan_mojang_java(
            &client,
            &requirement,
            crate::java::JavaTarget::native().unwrap(),
            |_| {},
        )
        .await
        .unwrap();
        assert!(java.artifact_count() > 0);
    }
    #[test]
    fn routes_documented_public_endpoints_without_modifying_canonical_urls() {
        for (canonical, path) in [
            (
                "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
                "/mc/game/version_manifest_v2.json",
            ),
            (
                "https://resources.download.minecraft.net/ab/abcdef",
                "/assets/ab/abcdef",
            ),
            (
                "https://libraries.minecraft.net/org/example.jar",
                "/maven/org/example.jar",
            ),
            (
                "https://meta.fabricmc.net/v2/versions/loader/26.2",
                "/fabric-meta/v2/versions/loader/26.2",
            ),
            (
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/a.jar",
                "/maven/net/neoforged/neoforge/a.jar",
            ),
        ] {
            let original = url::Url::parse(canonical).unwrap();
            let routed = DownloadSource::Bmclapi.route(&original);
            assert_eq!(routed.host_str(), Some("bmclapi2.bangbang93.com"));
            assert_eq!(routed.path(), path);
            assert_eq!(original.as_str(), canonical);
            assert_eq!(DownloadSource::Official.route(&original), original);
        }
    }
    #[test]
    fn authentication_quilt_and_lookalike_hosts_are_never_redirected() {
        for url in [
            "https://login.live.com/token",
            "https://api.minecraftservices.com/minecraft/profile",
            "https://example.org/authserver/authenticate",
            "https://meta.quiltmc.org/v3/versions/loader",
            "https://maven.fabricmc.net.evil.example/a",
            "https://user:secret@libraries.minecraft.net/a",
        ] {
            let original = url::Url::parse(url).unwrap();
            assert_eq!(DownloadSource::Bmclapi.route(&original), original);
        }
    }
    #[test]
    fn url_encoding_and_queries_survive_routing() {
        let original =
            url::Url::parse("https://libraries.minecraft.net/a%20b/%E4%B8%AD.jar?x=a%2Bb").unwrap();
        let routed = DownloadSource::Bmclapi.route(&original);
        assert_eq!(routed.path(), "/maven/a%20b/%E4%B8%AD.jar");
        assert_eq!(routed.query(), original.query());
    }
    #[test]
    fn integrity_sidecars_follow_the_selected_source_without_an_origin_exception() {
        let url = url::Url::parse("https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.19.5/fabric-loader-0.19.5.jar.sha1").unwrap();
        assert_eq!(
            DownloadSource::Bmclapi.route(&url).host_str(),
            Some("bmclapi2.bangbang93.com")
        );
        let request = DownloadClient::new(reqwest::Client::new(), DownloadSource::Bmclapi)
            .get(url)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().host_str(), Some("bmclapi2.bangbang93.com"));
    }
    #[test]
    fn unsupported_mirror_components_fail_without_contacting_the_origin() {
        let client = DownloadClient::new(reqwest::Client::new(), DownloadSource::Bmclapi);
        assert!(
            client
                .get(url::Url::parse("https://meta.quiltmc.org/v3/versions/loader").unwrap())
                .is_err()
        );
        assert!(
            client
                .get(url::Url::parse("https://user:secret@bmclapi2.bangbang93.com/a").unwrap())
                .is_err()
        );
        assert!(
            client
                .get(url::Url::parse("http://bmclapi2.bangbang93.com/a").unwrap())
                .is_err()
        );
        assert!(
            client
                .get(url::Url::parse("https://unknown.example/a.jar").unwrap())
                .is_err()
        );
    }
    #[test]
    fn java_metadata_reformatting_is_source_specific_not_an_artifact_hash_override() {
        let official = DownloadClient::new(reqwest::Client::new(), DownloadSource::Official);
        let mirror = DownloadClient::new(reqwest::Client::new(), DownloadSource::Bmclapi);
        let bytes = br#"{"files":{}}"#;
        let digest = hex::encode(Sha1::digest(bytes));
        assert_eq!(
            official.java_manifest_digest(bytes, &digest).unwrap(),
            digest
        );
        assert!(
            official
                .java_manifest_digest(bytes, &"a".repeat(40))
                .is_err()
        );
        assert_eq!(
            mirror.java_manifest_digest(bytes, &"a".repeat(40)).unwrap(),
            hex::encode(Sha1::digest(bytes))
        );
    }
}
