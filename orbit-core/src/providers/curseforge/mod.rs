use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use curseforge_wrapper::models::{File, GetFilesParams, Mod, ModLoaderType, SearchModsParams};
use curseforge_wrapper::{ApiError, Client, ClientConfig, MAX_RESULTS};
use reqwest::StatusCode;
use tokio::sync::OnceCell;

use super::rate_limiter::RateLimiter;
use super::{
    ArtifactDownloadClient, ArtifactFingerprint, CatalogDependency, CurseForgeResolvedInfo,
    ModInfo, ModProvider, ModVersionInfo, ProjectImage, RemoteArtifact, RemoteProjectLocator,
    SearchResultItem,
};
use crate::error::OrbitError;

const OPTIONAL_DEPENDENCY: u8 = 2;
const REQUIRED_DEPENDENCY: u8 = 3;

#[derive(Debug)]
struct MinecraftContext {
    game_id: u32,
    mods_class_id: u32,
}

pub struct CurseForgeProvider {
    client: Client,
    downloader: ArtifactDownloadClient,
    rate_limiter: RateLimiter,
    minecraft: OnceCell<MinecraftContext>,
}

impl CurseForgeProvider {
    pub(crate) fn new(
        api_key: &str,
        user_agent: &str,
        http: &super::ProviderHttpConfig,
    ) -> Result<Self, OrbitError> {
        let api_key = required_api_key(api_key)?;
        let client_config = ClientConfig {
            timeout: http.timeout,
            max_retries: http.max_retries,
            proxy: http.proxy.clone(),
        };
        Ok(Self {
            client: Client::new(api_key, user_agent, &client_config).map_err(map_client_error)?,
            downloader: ArtifactDownloadClient::authenticated_for_domain(
                user_agent,
                "x-api-key",
                api_key,
                "forgecdn.net",
                http,
            )?,
            rate_limiter: RateLimiter::new(http.max_concurrency),
            minecraft: OnceCell::new(),
        })
    }

    #[cfg(test)]
    fn with_base_url(api_key: &str, user_agent: &str, base_url: &str) -> Result<Self, OrbitError> {
        let api_key = required_api_key(api_key)?;
        let http = super::ProviderHttpConfig::test_default();
        let client_config = ClientConfig {
            timeout: http.timeout,
            max_retries: http.max_retries,
            proxy: http.proxy.clone(),
        };
        Ok(Self {
            client: Client::with_base_url(api_key, user_agent, base_url, &client_config)
                .map_err(map_client_error)?,
            downloader: ArtifactDownloadClient::authenticated_for_domain(
                user_agent,
                "x-api-key",
                api_key,
                "forgecdn.net",
                &http,
            )?,
            rate_limiter: RateLimiter::new(http.max_concurrency),
            minecraft: OnceCell::new(),
        })
    }

    async fn minecraft(&self) -> Result<&MinecraftContext, OrbitError> {
        self.minecraft
            .get_or_try_init(|| async {
                let games = self.client.games().await.map_err(map_client_error)?;
                let game = games
                    .into_iter()
                    .find(|game| game.slug.eq_ignore_ascii_case("minecraft"))
                    .ok_or_else(|| {
                        OrbitError::Other(anyhow::anyhow!(
                            "CurseForge API key does not expose a game with slug 'minecraft'"
                        ))
                    })?;
                let categories = self
                    .client
                    .categories(game.id)
                    .await
                    .map_err(map_client_error)?;
                let mods_class = categories
                    .iter()
                    .find(|category| {
                        category.is_class == Some(true)
                            && (category.name.eq_ignore_ascii_case("mods")
                                || category.slug.eq_ignore_ascii_case("mods")
                                || category.slug.eq_ignore_ascii_case("mc-mods"))
                    })
                    .ok_or_else(|| {
                        OrbitError::Other(anyhow::anyhow!(
                            "CurseForge returned no top-level Minecraft Mods class"
                        ))
                    })?;
                Ok(MinecraftContext {
                    game_id: game.id,
                    mods_class_id: mods_class.id,
                })
            })
            .await
    }

    async fn project(&self, slug_or_id: &str) -> Result<Mod, OrbitError> {
        if let Ok(project_id) = slug_or_id.parse::<u32>() {
            return self
                .client
                .get_mod(project_id)
                .await
                .map_err(|error| map_project_error(error, slug_or_id));
        }
        let context = self.minecraft().await?;
        let mut projects = self
            .client
            .search_mods(SearchModsParams {
                game_id: context.game_id,
                class_id: context.mods_class_id,
                slug: Some(slug_or_id),
                page_size: 50,
                ..SearchModsParams::default()
            })
            .await
            .map_err(|error| map_project_error(error, slug_or_id))?
            .data;
        projects
            .drain(..)
            .find(|project| project.slug.eq_ignore_ascii_case(slug_or_id))
            .ok_or_else(|| OrbitError::ModNotFound(slug_or_id.to_string()))
    }

    async fn dependency_slugs(&self, files: &[File]) -> Result<HashMap<u32, String>, OrbitError> {
        let ids: HashSet<u32> = files
            .iter()
            .flat_map(|file| &file.dependencies)
            .map(|dependency| dependency.mod_id)
            .collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.client
            .get_mods(&ids.into_iter().collect::<Vec<_>>())
            .await
            .map_err(map_client_error)
            .map(|projects| {
                projects
                    .into_iter()
                    .map(|project| (project.id, project.slug))
                    .collect()
            })
    }

    async fn resolved_file(
        &self,
        project: &Mod,
        file: &File,
        require_download: bool,
    ) -> Result<Option<RemoteArtifact>, OrbitError> {
        if !file.is_available {
            return Ok(None);
        }
        let sha1 = file.sha1();
        if require_download && sha1.is_empty() {
            return Ok(None);
        }
        let download_url = match &file.download_url {
            Some(url) if !url.is_empty() => Some(url.clone()),
            _ if !require_download => None,
            _ => match self.client.download_url(project.id, file.id).await {
                Ok(url) => Some(url),
                Err(error)
                    if matches!(
                        error.status(),
                        Some(StatusCode::FORBIDDEN | StatusCode::NOT_FOUND)
                    ) =>
                {
                    None
                }
                Err(error) => return Err(map_client_error(error)),
            },
        };
        if require_download && download_url.is_none() {
            return Ok(None);
        }
        let dependencies = file
            .dependencies
            .iter()
            .map(|dependency| RemoteProjectLocator {
                slug: None,
                project_id: Some(dependency.mod_id.to_string()),
            })
            .collect();
        Ok(Some(RemoteArtifact {
            sha1,
            sha512: String::new(),
            slug: project.slug.clone(),
            provider: "curseforge".to_string(),
            modrinth: None,
            curseforge: Some(CurseForgeResolvedInfo {
                project_id: project.id,
                file_id: file.id,
                fingerprint: u32::try_from(file.file_fingerprint).unwrap_or_default(),
            }),
            download_url: download_url.unwrap_or_default(),
            filename: file.file_name.clone(),
            related_projects: dependencies,
        }))
    }

    async fn versions_for_project(
        &self,
        project: &Mod,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<RemoteArtifact>, OrbitError> {
        let mod_loader_type = loader.map(parse_loader).transpose()?;
        let files = self
            .client
            .get_files(
                project.id,
                GetFilesParams {
                    game_version: mc_version,
                    mod_loader_type,
                },
            )
            .await
            .map_err(map_client_error)?;
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let mut versions = Vec::new();
        let mut unavailable = Vec::new();
        for file in files {
            if let Some(resolved) = self.resolved_file(project, &file, true).await? {
                versions.push(resolved);
            } else {
                unavailable.push(format!("{} ({})", file.file_name, file.id));
            }
        }
        if !unavailable.is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "CurseForge project '{}' has matching files that cannot be included in a complete \
                 candidate queue because they are unavailable, lack SHA-1, or have no API download \
                 URL: {}",
                project.slug,
                unavailable.join(", ")
            )));
        }
        Ok(versions)
    }
}

fn required_api_key(api_key: &str) -> Result<&str, OrbitError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(OrbitError::ProviderApiKeyRequired {
            provider: "CurseForge",
            environment_variable: "ORBIT_CURSEFORGE_API_KEY",
            config_key: "auth.curseforge_api_key",
        });
    }
    Ok(api_key)
}

fn map_client_error(error: ApiError) -> OrbitError {
    OrbitError::Other(error.into())
}

fn map_project_error(error: ApiError, slug: &str) -> OrbitError {
    if error.status() == Some(StatusCode::NOT_FOUND) {
        OrbitError::ModNotFound(slug.to_string())
    } else {
        map_client_error(error)
    }
}

fn parse_loader(loader: &str) -> Result<ModLoaderType, OrbitError> {
    ModLoaderType::parse(loader).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "CurseForge does not define a loader enum for '{loader}'"
        ))
    })
}

fn project_versions(project: &Mod, loader: Option<ModLoaderType>) -> Vec<String> {
    let requested_loader = loader.map(|value| value as u8);
    let mut versions = Vec::new();
    for index in &project.latest_files_indexes {
        if requested_loader.is_none_or(|loader| index.mod_loader == 0 || index.mod_loader == loader)
            && !versions.contains(&index.game_version)
        {
            versions.push(index.game_version.clone());
        }
    }
    versions
}

fn file_loader(file: &File) -> String {
    file.game_versions
        .iter()
        .find_map(|version| ModLoaderType::parse(version).map(|loader| loader as u8))
        .map(ModLoaderType::name)
        .unwrap_or_default()
        .to_string()
}

fn file_game_versions(file: &File) -> Vec<String> {
    let mut versions = Vec::new();
    for version in &file.sortable_game_versions {
        if !versions.contains(&version.game_version_name) {
            versions.push(version.game_version_name.clone());
        }
    }
    if versions.is_empty() {
        for version in &file.game_versions {
            if ModLoaderType::parse(version).is_none() && !versions.contains(version) {
                versions.push(version.clone());
            }
        }
    }
    versions
}

#[async_trait]
impl ModProvider for CurseForgeProvider {
    fn name(&self) -> &'static str {
        "curseforge"
    }

    fn artifact_downloader(&self) -> &ArtifactDownloadClient {
        &self.downloader
    }

    async fn search(
        &self,
        query: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResultItem>, OrbitError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let _permit = self.rate_limiter.acquire().await?;
        let context = self.minecraft().await?;
        let loader_type = loader.map(parse_loader).transpose()?;
        let mut index = 0;
        let mut results = Vec::new();
        while results.len() < limit {
            let response = self
                .client
                .search_mods(SearchModsParams {
                    game_id: context.game_id,
                    class_id: context.mods_class_id,
                    search_filter: Some(query),
                    game_version: mc_version,
                    mod_loader_type: loader_type,
                    index,
                    page_size: 50,
                    ..SearchModsParams::default()
                })
                .await
                .map_err(map_client_error)?;
            let done = response.pagination.result_count == 0
                || u64::from(response.pagination.index + response.pagination.result_count)
                    >= response.pagination.total_count
                || response.pagination.index + response.pagination.page_size >= MAX_RESULTS;
            for project in response.data.into_iter().filter(|project| {
                project.is_available
                    && loader_type.is_none_or(|loader| {
                        project
                            .latest_files_indexes
                            .iter()
                            .any(|index| index.mod_loader == 0 || index.mod_loader == loader as u8)
                    })
            }) {
                let mc_versions = project_versions(&project, loader_type);
                let latest_version = project
                    .latest_files
                    .iter()
                    .max_by_key(|file| &file.file_date)
                    .map(|file| file.display_name.clone())
                    .unwrap_or_default();
                results.push(SearchResultItem {
                    project_id: project.id.to_string(),
                    slug: project.slug,
                    name: project.name,
                    description: project.summary,
                    latest_version,
                    downloads: project.download_count,
                    mc_versions,
                    client_side: None,
                    server_side: None,
                    categories: project
                        .categories
                        .into_iter()
                        .map(|category| category.name)
                        .collect(),
                    icon_url: project.logo.map(|logo| logo.thumbnail_url),
                    accent_color: None,
                });
                if results.len() == limit {
                    break;
                }
            }
            if done {
                break;
            }
            index = response.pagination.index + response.pagination.page_size;
        }
        Ok(results)
    }

    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let project = self.project(slug).await?;
        let mut files = self
            .client
            .get_files(project.id, GetFilesParams::default())
            .await
            .map_err(map_client_error)?;
        files.sort_by(|left, right| right.file_date.cmp(&left.file_date));
        let dependencies = files
            .first()
            .map(|file| file.dependencies.clone())
            .unwrap_or_default();
        let dependency_projects = self.dependency_slugs(&files).await?;
        let links = project.links.clone();
        Ok(ModInfo {
            project_id: project.id.to_string(),
            slug: project.slug,
            name: project.name,
            description: project.summary,
            authors: project
                .authors
                .into_iter()
                .map(|author| author.name)
                .collect(),
            latest_version: files
                .first()
                .map(|file| file.display_name.clone())
                .unwrap_or_default(),
            downloads: project.download_count,
            license: None,
            client_side: None,
            server_side: None,
            categories: project
                .categories
                .into_iter()
                .map(|category| category.name)
                .collect(),
            icon_url: project.logo.map(|logo| logo.thumbnail_url),
            accent_color: None,
            website_url: links.as_ref().and_then(|value| value.website_url.clone()),
            source_url: links.as_ref().and_then(|value| value.source_url.clone()),
            issues_url: links.as_ref().and_then(|value| value.issues_url.clone()),
            wiki_url: links.as_ref().and_then(|value| value.wiki_url.clone()),
            gallery: project
                .screenshots
                .into_iter()
                .map(|image| ProjectImage {
                    url: image.url,
                    thumbnail_url: Some(image.thumbnail_url),
                    title: (!image.title.is_empty()).then_some(image.title),
                    description: (!image.description.is_empty()).then_some(image.description),
                })
                .collect(),
            recent_versions: files
                .iter()
                .take(5)
                .map(|file| ModVersionInfo {
                    version: file.display_name.clone(),
                    mc_versions: file_game_versions(file),
                    loader: file_loader(file),
                    released_at: file.file_date.clone(),
                })
                .collect(),
            dependencies: dependencies
                .into_iter()
                .filter(|dependency| {
                    matches!(
                        dependency.relation_type,
                        OPTIONAL_DEPENDENCY | REQUIRED_DEPENDENCY
                    )
                })
                .map(|dependency| CatalogDependency {
                    slug: dependency_projects.get(&dependency.mod_id).cloned(),
                    required: dependency.relation_type == REQUIRED_DEPENDENCY,
                    project_id: Some(dependency.mod_id.to_string()),
                })
                .collect(),
        })
    }

    async fn identify_artifacts(
        &self,
        artifacts: &[ArtifactFingerprint],
    ) -> Result<Vec<RemoteArtifact>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let fingerprints: Vec<u32> = artifacts
            .iter()
            .map(|artifact| artifact.curseforge)
            .filter(|fingerprint| *fingerprint != 0)
            .collect();
        if fingerprints.is_empty() {
            return Ok(Vec::new());
        }
        let context = self.minecraft().await?;
        let matches = self
            .client
            .fingerprint_matches(context.game_id, &fingerprints)
            .await
            .map_err(map_client_error)?;
        let project_ids: HashSet<u32> = matches
            .exact_matches
            .iter()
            .map(|matched| matched.file.mod_id)
            .collect();
        let projects: HashMap<u32, Mod> = self
            .client
            .get_mods(&project_ids.into_iter().collect::<Vec<_>>())
            .await
            .map_err(map_client_error)?
            .into_iter()
            .map(|project| (project.id, project))
            .collect();
        let mut identified = Vec::new();
        for matched in matches.exact_matches {
            let Some(project) = projects.get(&matched.file.mod_id) else {
                continue;
            };
            if let Some(resolved) = self.resolved_file(project, &matched.file, false).await? {
                identified.push(resolved);
            }
        }
        Ok(identified)
    }

    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<RemoteArtifact>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;
        let project = self.project(slug).await?;
        self.versions_for_project(&project, mc_version, loader)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::sync::Arc;

    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response, body::Incoming};
    use hyper_util::rt::TokioIo;
    use tokio::task::JoinHandle;

    use super::*;

    #[derive(Clone, Copy)]
    struct MockResponse {
        request_contains: &'static str,
        status: &'static str,
        body: &'static str,
    }

    async fn mock_server(responses: Vec<MockResponse>) -> (String, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let responses = Arc::new(tokio::sync::Mutex::new((
                VecDeque::from(responses),
                None::<MockResponse>,
            )));
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.unwrap();
                        let service_responses = responses.clone();
                        connections.spawn(async move {
                            http1::Builder::new()
                                .serve_connection(
                                    TokioIo::new(stream),
                                    service_fn(move |request: Request<Incoming>| {
                                        let responses = service_responses.clone();
                                        async move {
                                            let request_line =
                                                format!("{} {} ", request.method(), request.uri());
                                            let expected = {
                                                let mut responses = responses.lock().await;
                                                if responses
                                                    .0
                                                    .front()
                                                    .is_some_and(|expected| {
                                                        request_line
                                                            .contains(expected.request_contains)
                                                    })
                                                {
                                                    let expected = responses.0.pop_front().unwrap();
                                                    responses.1 = Some(expected);
                                                    expected
                                                } else if responses
                                                    .1
                                                    .is_some_and(|expected| {
                                                        request_line
                                                            .contains(expected.request_contains)
                                                    })
                                                {
                                                    responses.1.unwrap()
                                                } else {
                                                    panic!("unexpected HTTP request: {request_line}")
                                                }
                                            };
                                            assert!(
                                                request_line.contains(expected.request_contains),
                                                "request did not contain {:?}: {request_line}",
                                                expected.request_contains
                                            );
                                            assert_eq!(
                                                request
                                                    .headers()
                                                    .get("x-api-key")
                                                    .and_then(|value| value.to_str().ok()),
                                                Some("test-key"),
                                                "API key header missing"
                                            );
                                            let _ = request.into_body().collect().await.unwrap();
                                            let status = expected
                                                .status
                                                .split_once(' ')
                                                .map(|(code, _)| code)
                                                .unwrap_or(expected.status)
                                                .parse::<u16>()
                                                .unwrap();
                                            Ok::<_, Infallible>(
                                                Response::builder()
                                                    .status(status)
                                                    .header("content-type", "application/json")
                                                    .body(Full::new(Bytes::from_static(
                                                        expected.body.as_bytes(),
                                                    )))
                                                    .unwrap(),
                                            )
                                        }
                                    }),
                                )
                                .await
                                .unwrap();
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        result.unwrap();
                    }
                }
            }
        });
        (format!("http://{address}/v1/"), handle)
    }

    async fn stop_mock_server(server: JoinHandle<()>) {
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[test]
    fn direct_construction_requires_an_api_key() {
        let error = CurseForgeProvider::new(
            " \t ",
            "orbit-test",
            &super::super::ProviderHttpConfig::test_default(),
        )
        .err()
        .expect("blank key should fail");
        assert!(matches!(
            error,
            OrbitError::ProviderApiKeyRequired {
                provider: "CurseForge",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn provider_downloads_reject_untrusted_hosts_before_network_access() {
        let provider = CurseForgeProvider::new(
            "test-key",
            "orbit-test",
            &super::super::ProviderHttpConfig::test_default(),
        )
        .unwrap();
        let error = provider
            .artifact_downloader()
            .download("https://example.invalid/example.jar", "example.jar")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("untrusted host"));
    }

    #[tokio::test]
    async fn discovers_ids_and_searches_with_official_filters() {
        let (base_url, server) = mock_server(vec![
            MockResponse {
                request_contains: "GET /v1/games?index=0&pageSize=50 ",
                status: "200 OK",
                body: r#"{"data":[{"id":432,"name":"Minecraft","slug":"minecraft"}],"pagination":{"index":0,"pageSize":50,"resultCount":1,"totalCount":1}}"#,
            },
            MockResponse {
                request_contains: "GET /v1/categories?gameId=432 ",
                status: "200 OK",
                body: r#"{"data":[{"id":6,"name":"Mods","slug":"mc-mods","isClass":true,"classId":null},{"id":421,"name":"API and Library","slug":"api-and-library","isClass":false,"classId":6}]}"#,
            },
            MockResponse {
                request_contains: "GET /v1/mods/search?gameId=432&classId=6&index=0&pageSize=50&searchFilter=sodium&gameVersion=1.21.1&modLoaderType=4 ",
                status: "200 OK",
                body: r#"{"data":[{"id":394468,"name":"Sodium","slug":"sodium","summary":"Renderer","downloadCount":42,"categories":[{"id":421,"name":"API and Library","slug":"api-and-library","isClass":false,"classId":6}],"authors":[{"name":"jellysquid"}],"latestFiles":[{"id":1,"modId":394468,"isAvailable":true,"displayName":"Sodium 1","fileName":"sodium.jar","hashes":[{"value":"abc","algo":1}],"fileDate":"2026-01-01T00:00:00Z","downloadUrl":"https://example.invalid/sodium.jar","gameVersions":["1.21.1","Fabric"],"sortableGameVersions":[{"gameVersionName":"1.21.1"}],"dependencies":[],"fileFingerprint":123}],"latestFilesIndexes":[{"gameVersion":"1.21.1","fileId":1,"filename":"sodium.jar","releaseType":1,"modLoader":4}],"isAvailable":true}],"pagination":{"index":0,"pageSize":5,"resultCount":1,"totalCount":1}}"#,
            },
        ])
        .await;
        let provider =
            CurseForgeProvider::with_base_url("test-key", "orbit-test", &base_url).unwrap();

        let results = provider
            .search("sodium", Some("1.21.1"), Some("fabric"), 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "sodium");
        assert_eq!(results[0].mc_versions, vec!["1.21.1"]);
        stop_mock_server(server).await;
    }

    #[tokio::test]
    async fn resolves_missing_file_url_through_download_endpoint() {
        let project = r#"{"id":123,"name":"Example","slug":"example","summary":"Example","downloadCount":1,"categories":[],"authors":[],"latestFiles":[],"latestFilesIndexes":[],"isAvailable":true}"#;
        let file = r#"{"id":456,"modId":123,"isAvailable":true,"displayName":"Example 2","fileName":"example.jar","hashes":[{"value":"deadbeef","algo":1}],"fileDate":"2026-02-01T00:00:00Z","downloadUrl":null,"gameVersions":["1.21.1","NeoForge"],"sortableGameVersions":[{"gameVersionName":"1.21.1"}],"dependencies":[],"fileFingerprint":987}"#;
        let project_response: &'static str =
            Box::leak(format!(r#"{{"data":{project}}}"#).into_boxed_str());
        let files_response: &'static str = Box::leak(
            format!(
                r#"{{"data":[{file}],"pagination":{{"index":0,"pageSize":50,"resultCount":1,"totalCount":1}}}}"#
            )
            .into_boxed_str(),
        );
        let (base_url, server) = mock_server(vec![
            MockResponse {
                request_contains: "GET /v1/mods/123 ",
                status: "200 OK",
                body: project_response,
            },
            MockResponse {
                request_contains: "GET /v1/mods/123/files?index=0&pageSize=50&gameVersion=1.21.1&modLoaderType=6 ",
                status: "200 OK",
                body: files_response,
            },
            MockResponse {
                request_contains: "GET /v1/mods/123/files/456/download-url ",
                status: "200 OK",
                body: r#"{"data":"https://example.invalid/example.jar"}"#,
            },
        ])
        .await;
        let provider =
            CurseForgeProvider::with_base_url("test-key", "orbit-test", &base_url).unwrap();

        let versions = provider
            .get_versions("123", Some("1.21.1"), Some("neoforge"))
            .await
            .unwrap();

        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].sha1, "deadbeef");
        assert_eq!(
            versions[0].download_url,
            "https://example.invalid/example.jar"
        );
        assert_eq!(versions[0].version_id().as_deref(), Some("456"));
        stop_mock_server(server).await;
    }

    #[tokio::test]
    async fn rejects_a_partial_candidate_queue_when_one_file_is_blocked() {
        let project = r#"{"id":123,"name":"Example","slug":"example","summary":"Example","downloadCount":1,"categories":[],"authors":[],"latestFiles":[],"latestFilesIndexes":[],"isAvailable":true}"#;
        let available = r#"{"id":455,"modId":123,"isAvailable":true,"displayName":"Example 1","fileName":"example-1.jar","hashes":[{"value":"cafebabe","algo":1}],"fileDate":"2026-01-01T00:00:00Z","downloadUrl":"https://example.invalid/example-1.jar","gameVersions":["1.21.1","Fabric"],"sortableGameVersions":[{"gameVersionName":"1.21.1"}],"dependencies":[],"fileFingerprint":986}"#;
        let blocked = r#"{"id":456,"modId":123,"isAvailable":true,"displayName":"Example 2","fileName":"example-2.jar","hashes":[{"value":"deadbeef","algo":1}],"fileDate":"2026-02-01T00:00:00Z","downloadUrl":null,"gameVersions":["1.21.1","Fabric"],"sortableGameVersions":[{"gameVersionName":"1.21.1"}],"dependencies":[],"fileFingerprint":987}"#;
        let project_response: &'static str =
            Box::leak(format!(r#"{{"data":{project}}}"#).into_boxed_str());
        let files_response: &'static str = Box::leak(
            format!(
                r#"{{"data":[{available},{blocked}],"pagination":{{"index":0,"pageSize":50,"resultCount":2,"totalCount":2}}}}"#
            )
            .into_boxed_str(),
        );
        let (base_url, server) = mock_server(vec![
            MockResponse {
                request_contains: "GET /v1/mods/123 ",
                status: "200 OK",
                body: project_response,
            },
            MockResponse {
                request_contains: "GET /v1/mods/123/files?index=0&pageSize=50&gameVersion=1.21.1&modLoaderType=4 ",
                status: "200 OK",
                body: files_response,
            },
            MockResponse {
                request_contains: "GET /v1/mods/123/files/456/download-url ",
                status: "404 Not Found",
                body: r#"{"error":"download unavailable"}"#,
            },
        ])
        .await;
        let provider =
            CurseForgeProvider::with_base_url("test-key", "orbit-test", &base_url).unwrap();

        let error = provider
            .get_versions("123", Some("1.21.1"), Some("fabric"))
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("complete candidate queue")
                && error.to_string().contains("example-2.jar"),
            "unexpected error: {error}"
        );
        stop_mock_server(server).await;
    }

    #[tokio::test]
    async fn returns_an_empty_version_set_when_no_files_match() {
        let (base_url, server) = mock_server(vec![
            MockResponse {
                request_contains: "GET /v1/mods/123 ",
                status: "200 OK",
                body: r#"{"data":{"id":123,"name":"Example","slug":"example","summary":"Example","downloadCount":1,"categories":[],"authors":[],"latestFiles":[],"latestFilesIndexes":[],"isAvailable":true}}"#,
            },
            MockResponse {
                request_contains: "GET /v1/mods/123/files?index=0&pageSize=50&gameVersion=1.21.1&modLoaderType=4 ",
                status: "200 OK",
                body: r#"{"data":[],"pagination":{"index":0,"pageSize":50,"resultCount":0,"totalCount":0}}"#,
            },
        ])
        .await;
        let provider =
            CurseForgeProvider::with_base_url("test-key", "orbit-test", &base_url).unwrap();

        let versions = provider
            .get_versions("123", Some("1.21.1"), Some("fabric"))
            .await
            .unwrap();

        assert!(versions.is_empty());
        stop_mock_server(server).await;
    }

    #[tokio::test]
    async fn identifies_local_artifacts_with_the_fingerprint_endpoint() {
        let (base_url, server) = mock_server(vec![
            MockResponse {
                request_contains: "GET /v1/games?index=0&pageSize=50 ",
                status: "200 OK",
                body: r#"{"data":[{"id":432,"slug":"minecraft"}],"pagination":{"index":0,"pageSize":50,"resultCount":1,"totalCount":1}}"#,
            },
            MockResponse {
                request_contains: "GET /v1/categories?gameId=432 ",
                status: "200 OK",
                body: r#"{"data":[{"id":6,"name":"Mods","slug":"mc-mods","isClass":true,"classId":null}]}"#,
            },
            MockResponse {
                request_contains: "POST /v1/fingerprints/432 ",
                status: "200 OK",
                body: r#"{"data":{"exactMatches":[{"id":456,"file":{"id":456,"modId":123,"isAvailable":true,"displayName":"Example 2","fileName":"example.jar","hashes":[{"value":"deadbeef","algo":1}],"fileDate":"2026-02-01T00:00:00Z","downloadUrl":null,"gameVersions":["1.21.1","Fabric"],"sortableGameVersions":[{"gameVersionName":"1.21.1"}],"dependencies":[],"fileFingerprint":987},"latestFiles":[]}],"exactFingerprints":[987],"partialMatches":[],"unmatchedFingerprints":[]}}"#,
            },
            MockResponse {
                request_contains: "POST /v1/mods ",
                status: "200 OK",
                body: r#"{"data":[{"id":123,"name":"Example","slug":"example","summary":"Example","downloadCount":1,"categories":[],"authors":[],"latestFiles":[],"latestFilesIndexes":[],"isAvailable":true}]}"#,
            },
        ])
        .await;
        let provider =
            CurseForgeProvider::with_base_url("test-key", "orbit-test", &base_url).unwrap();

        let identified = provider
            .identify_artifacts(&[ArtifactFingerprint {
                sha1: String::new(),
                sha512: String::new(),
                curseforge: 987,
            }])
            .await
            .unwrap();

        assert_eq!(identified.len(), 1);
        assert_eq!(identified[0].slug, "example");
        assert!(identified[0].download_url.is_empty());
        assert_eq!(
            identified[0]
                .curseforge
                .as_ref()
                .map(|metadata| metadata.fingerprint),
            Some(987)
        );
        stop_mock_server(server).await;
    }
}
