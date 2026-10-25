//! Opt-in public download routing. Canonical metadata/lock URLs stay unchanged.
//! Authentication clients never use this transport.
use crate::LauncherError;
use serde::{Deserialize, Serialize};
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
    pub fn route(self, canonical: &url::Url) -> url::Url {
        if self == Self::Official
            || canonical.scheme() != "https"
            || canonical.port().is_some()
            || !canonical.username().is_empty()
            || canonical.password().is_some()
        {
            return canonical.clone();
        }
        let path = canonical.path();
        // Hash sidecars are integrity inputs, not bulk artifacts. BMCLAPI's
        // Maven mirror does not consistently publish them; fetch them from
        // their declared origin directly, never by retry/fallback after a 404.
        if [".sha1", ".sha256", ".sha512", ".asc"]
            .iter()
            .any(|suffix| path.ends_with(suffix))
        {
            return canonical.clone();
        }
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
    pub fn get(&self, canonical: url::Url) -> reqwest::RequestBuilder {
        self.client.get(self.source.route(&canonical))
    }
    pub(crate) fn source(&self) -> DownloadSource {
        self.source
    }
    /// Legal documents retain their official origin, independently of mirrors.
    pub fn official_client(&self) -> &reqwest::Client {
        &self.client
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "uses live BMCLAPI metadata, without installing or starting a game"]
    async fn live_bmclapi_minecraft_fabric_and_java_preserve_canonical_identity() {
        let config = crate::config::GlobalConfig::default();
        let client = DownloadClient::new(config.http_client().unwrap(), DownloadSource::Bmclapi);
        let server = crate::mojang::MojangClient::new(client.clone())
            .resolve_vanilla_server("26.2")
            .await
            .unwrap();
        assert!(
            server
                .version_json_url
                .starts_with("https://piston-meta.mojang.com/")
        );
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
        let artifact = cache
            .fetch(&client, &profile.downloads[0].request, |_| {})
            .await
            .unwrap();
        assert!(artifact.size > 0);
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
    fn integrity_sidecars_are_always_fetched_from_the_declared_origin() {
        let url = url::Url::parse("https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.19.5/fabric-loader-0.19.5.jar.sha1").unwrap();
        assert_eq!(DownloadSource::Bmclapi.route(&url), url);
    }
}
