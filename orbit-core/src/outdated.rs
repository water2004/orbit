//! 过时检查编排层。
//!
//! BFS 下载候选 JAR → 解析 → PubGrub 离线求解。

use std::collections::{HashMap, HashSet};

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;
use crate::manifest::OrbitManifest;
use crate::providers::{ModProvider, RemoteArtifact};
use crate::resolver::types::{
    CandidateCatalog, CandidateDiagnostic, PackageChange, PackageChangeKind, ResolutionReport,
    ResolutionSelector, ResolvedCandidates,
};

pub struct OutdatedMod {
    pub mod_id: String,
    pub current_version: String,
    pub new_version: String,
    pub candidate_id: String,
}

#[derive(Default)]
pub struct OutdatedReport {
    pub updates: Vec<OutdatedMod>,
    pub resolved: ResolvedCandidates,
    pub changes: Vec<PackageChange>,
    pub resolution: ResolutionReport,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

/// Discover the complete remote project closure, then download that artifact
/// queue as one bounded batch. JAR dependencies are deliberately outside this
/// layer; only the solver consumes them after every artifact has been parsed.
/// 供 `install_mod` 和 `check_all_outdated` 共用。
pub async fn download_candidates_bfs(
    provider: &dyn ModProvider,
    seeds: &[String],
    mc_version: &str,
    loader: &str,
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<CandidateCatalog, OrbitError> {
    let mut seen_lookups: HashSet<String> = HashSet::new();
    let mut seen_artifacts: HashSet<String> = HashSet::new();
    let artifacts = discover_artifact_closure(
        provider,
        seeds.to_vec(),
        mc_version,
        loader,
        &mut seen_lookups,
        &mut seen_artifacts,
    )
    .await?;
    let downloader = provider.artifact_downloader().clone();
    let jobs = artifacts
        .into_iter()
        .map(|artifact| (downloader.clone(), artifact))
        .collect();
    let parsed = download_artifact_queue(jobs, loader, jar_cache).await?;
    let mut catalog = CandidateCatalog::default();
    for (metadata, artifact) in parsed {
        catalog.record(metadata, artifact)?;
    }
    Ok(catalog)
}

async fn discover_artifact_closure(
    provider: &dyn ModProvider,
    initial_lookups: Vec<String>,
    mc_version: &str,
    loader: &str,
    seen_lookups: &mut HashSet<String>,
    seen_artifacts: &mut HashSet<String>,
) -> Result<Vec<RemoteArtifact>, OrbitError> {
    let mut queue: Vec<_> = initial_lookups
        .into_iter()
        .map(|lookup| (lookup, None::<String>))
        .collect();
    let mut discovered = Vec::new();
    while let Some((lookup, referenced_by)) = queue.pop() {
        if !seen_lookups.insert(lookup.clone()) {
            continue;
        }
        let artifacts = match provider
            .get_versions(&lookup, Some(mc_version), Some(loader))
            .await
        {
            Ok(artifacts) => artifacts,
            Err(OrbitError::ModNotFound(_)) if referenced_by.is_some() => {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "{} project '{}' references missing project '{}'; the artifact closure is incomplete",
                    provider.name(),
                    referenced_by.unwrap_or_default(),
                    lookup
                )));
            }
            Err(error) => return Err(error),
        };

        for artifact in artifacts {
            for related in &artifact.related_projects {
                if let Some(project_id) = related.project_id.as_ref() {
                    queue.push((project_id.clone(), Some(lookup.clone())));
                } else if let Some(slug) = related.slug.as_ref() {
                    queue.push((slug.clone(), Some(lookup.clone())));
                }
            }
            let artifact_key = format!(
                "{}:{}:{}:{}",
                artifact.provider,
                artifact.version_id().unwrap_or_default(),
                artifact.download_url,
                artifact.filename
            );
            if !seen_artifacts.insert(artifact_key) {
                continue;
            }
            discovered.push(artifact);
        }
    }
    Ok(discovered)
}

async fn download_artifact_queue(
    jobs: Vec<(crate::providers::ArtifactDownloadClient, RemoteArtifact)>,
    loader: &str,
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<Vec<(crate::jar::JarModMetadata, RemoteArtifact)>, OrbitError> {
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
    let mut handles = Vec::with_capacity(jobs.len());
    for (downloader, artifact) in jobs {
        let loader = loader.to_string();
        let cache = jar_cache.clone();
        let semaphore = semaphore.clone();
        handles.push(tokio::spawn(async move {
            let _permit = semaphore.acquire_owned().await.map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "candidate download queue was closed: {error}"
                ))
            })?;
            let metadata = crate::jar::download_and_parse(
                &cache,
                &downloader,
                &artifact.download_url,
                &artifact.filename,
                &artifact.sha1,
                &artifact.sha512,
                &loader,
            )
            .await?;
            Ok::<_, OrbitError>((metadata, artifact))
        }));
    }

    let mut parsed = Vec::with_capacity(handles.len());
    let mut first_error = None;
    for handle in handles {
        match handle.await {
            Ok(Ok(candidate)) => parsed.push(candidate),
            Ok(Err(error)) => {
                first_error.get_or_insert(error);
            }
            Err(error) => {
                first_error.get_or_insert_with(|| {
                    OrbitError::Other(anyhow::anyhow!("candidate download task failed: {error}"))
                });
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(parsed)
}

pub async fn download_candidates_with_fallback(
    providers: &[Box<dyn ModProvider>],
    seeds: &[String],
    mc_version: &str,
    loader: &str,
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<CandidateCatalog, OrbitError> {
    for provider in providers {
        match download_candidates_bfs(provider.as_ref(), seeds, mc_version, loader, jar_cache).await
        {
            Ok(download) if !download.candidates.is_empty() => return Ok(download),
            Ok(_) | Err(OrbitError::ModNotFound(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(OrbitError::ModNotFound(
        seeds.first().cloned().unwrap_or_default(),
    ))
}

pub(crate) async fn download_lockfile_candidate_catalog(
    providers: &[Box<dyn ModProvider>],
    lockfile: &OrbitLockfile,
    mc_version: &str,
    loader: &str,
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<CandidateCatalog, OrbitError> {
    let mut seeds_by_provider: HashMap<String, Vec<String>> = HashMap::new();
    for entry in lockfile
        .packages
        .iter()
        .filter(|entry| entry.provider != "file")
    {
        crate::providers::find_provider(providers, &entry.provider).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "lockfile contains {} packages but that provider is not configured",
                entry.provider
            ))
        })?;
        if let Some(project_id) = entry.source_project_id() {
            seeds_by_provider
                .entry(entry.provider.clone())
                .or_default()
                .push(project_id);
        }
    }

    let mut jobs = Vec::new();
    let mut catalog = CandidateCatalog::default();
    for provider in providers {
        let Some(seeds) = seeds_by_provider.remove(provider.name()) else {
            continue;
        };
        let mut seen_lookups = HashSet::new();
        let mut seen_artifacts = HashSet::new();
        let artifacts = discover_artifact_closure(
            provider.as_ref(),
            seeds,
            mc_version,
            loader,
            &mut seen_lookups,
            &mut seen_artifacts,
        )
        .await?;
        let downloader = provider.artifact_downloader().clone();
        jobs.extend(
            artifacts
                .into_iter()
                .map(|artifact| (downloader.clone(), artifact)),
        );
    }
    debug_assert!(seeds_by_provider.is_empty());
    for (metadata, artifact) in download_artifact_queue(jobs, loader, jar_cache).await? {
        catalog.record(metadata, artifact)?;
    }
    Ok(catalog)
}

/// 检查所有已安装平台模组的可用更新。
pub async fn check_all_outdated(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
    selector: Option<ResolutionSelector>,
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<OutdatedReport, OrbitError> {
    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;

    let catalog =
        download_lockfile_candidate_catalog(providers, lockfile, mc_version, loader, jar_cache)
            .await?;
    if catalog.candidates.is_empty() {
        return Ok(OutdatedReport::default());
    }

    // 3. Resolve
    let mut portfolio = crate::resolver::resolve_candidate_portfolio(manifest, lockfile, &catalog)
        .await
        .map_err(|e| OrbitError::Other(anyhow::anyhow!("{e}")))?;
    portfolio.alternatives.retain(ResolutionReport::has_upgrade);
    if portfolio.alternatives.is_empty() {
        return Ok(OutdatedReport {
            resolved: catalog.resolved,
            ..OutdatedReport::default()
        });
    }
    let resolution = crate::resolver::select_resolution(portfolio, selector)
        .map_err(|e| OrbitError::Other(anyhow::anyhow!("{e}")))?;

    let mut updates: Vec<OutdatedMod> = resolution
        .changes
        .iter()
        .filter(|change| change.kind == PackageChangeKind::Upgrade)
        .map(|change| OutdatedMod {
            candidate_id: resolution.selected_candidates[&change.package].clone(),
            current_version: change.current_version.clone().unwrap_or_default(),
            new_version: change.selected_version.clone().unwrap_or_default(),
            mod_id: change.package.clone(),
        })
        .collect();
    updates.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    Ok(OutdatedReport {
        updates,
        resolved: catalog.resolved,
        changes: resolution.changes.clone(),
        diagnostics: resolution.diagnostics.clone(),
        warnings: resolution.warnings.clone(),
        resolution,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        ArtifactDownloadClient, ModInfo, ModrinthResolvedInfo, RemoteProjectLocator,
        SearchResultItem,
    };
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct DiscoveryProvider {
        downloader: ArtifactDownloadClient,
        projects: HashMap<String, Vec<RemoteArtifact>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ModProvider for DiscoveryProvider {
        fn name(&self) -> &'static str {
            "modrinth"
        }

        fn artifact_downloader(&self) -> &ArtifactDownloadClient {
            &self.downloader
        }

        async fn search(
            &self,
            _query: &str,
            _mc_version: Option<&str>,
            _loader: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<SearchResultItem>, OrbitError> {
            Ok(Vec::new())
        }

        async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError> {
            Err(OrbitError::ModNotFound(slug.to_string()))
        }

        async fn get_versions(
            &self,
            slug: &str,
            _mc_version: Option<&str>,
            _loader: Option<&str>,
        ) -> Result<Vec<RemoteArtifact>, OrbitError> {
            self.calls.lock().unwrap().push(slug.to_string());
            self.projects
                .get(slug)
                .cloned()
                .ok_or_else(|| OrbitError::ModNotFound(slug.to_string()))
        }
    }

    fn artifact(source: &str, project_id: &str) -> RemoteArtifact {
        RemoteArtifact {
            sha1: String::new(),
            sha512: String::new(),
            slug: source.to_string(),
            provider: "modrinth".to_string(),
            modrinth: Some(ModrinthResolvedInfo {
                project_id: project_id.to_string(),
                version_id: format!("{project_id}-version"),
            }),
            curseforge: None,
            download_url: String::new(),
            filename: String::new(),
            related_projects: Vec::new(),
        }
    }

    fn related_artifact(
        source: &str,
        project_id: &str,
        version: &str,
        related_project: Option<&str>,
    ) -> RemoteArtifact {
        let mut artifact = artifact(source, project_id);
        artifact.filename = format!("{project_id}-{version}.jar");
        artifact.download_url = format!("https://example.invalid/{}", artifact.filename);
        artifact.modrinth.as_mut().unwrap().version_id = format!("{project_id}-{version}");
        artifact.related_projects = related_project
            .map(|project_id| {
                vec![RemoteProjectLocator {
                    slug: None,
                    project_id: Some(project_id.to_string()),
                }]
            })
            .unwrap_or_default();
        artifact
    }

    #[tokio::test]
    async fn discovery_recurses_projects_and_queues_every_matching_version_before_download() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = DiscoveryProvider {
            downloader: ArtifactDownloadClient::anonymous("orbit-test").unwrap(),
            projects: HashMap::from([
                (
                    "root".to_string(),
                    vec![
                        related_artifact("root", "root", "1", Some("child")),
                        related_artifact("root", "root", "2", Some("child")),
                    ],
                ),
                (
                    "child".to_string(),
                    vec![related_artifact("child", "child", "1", Some("grandchild"))],
                ),
                (
                    "grandchild".to_string(),
                    vec![related_artifact("grandchild", "grandchild", "1", None)],
                ),
            ]),
            calls: calls.clone(),
        };
        let mut seen_lookups = HashSet::new();
        let mut seen_artifacts = HashSet::new();

        let artifacts = discover_artifact_closure(
            &provider,
            vec!["root".to_string()],
            "1.21.1",
            "fabric",
            &mut seen_lookups,
            &mut seen_artifacts,
        )
        .await
        .unwrap();

        assert_eq!(artifacts.len(), 4);
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from([
                "root".to_string(),
                "child".to_string(),
                "grandchild".to_string(),
            ])
        );
    }

    #[tokio::test]
    async fn discovery_rejects_a_missing_related_project() {
        let provider = DiscoveryProvider {
            downloader: ArtifactDownloadClient::anonymous("orbit-test").unwrap(),
            projects: HashMap::from([(
                "root".to_string(),
                vec![related_artifact("root", "root", "1", Some("missing"))],
            )]),
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let mut seen_lookups = HashSet::new();
        let mut seen_artifacts = HashSet::new();

        let error = discover_artifact_closure(
            &provider,
            vec!["root".to_string()],
            "1.21.1",
            "fabric",
            &mut seen_lookups,
            &mut seen_artifacts,
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("artifact closure is incomplete"));
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn candidate_catalog_uses_actual_package_and_version_as_its_key() {
        let mut download = CandidateCatalog::default();

        download
            .record(
                crate::jar::JarModMetadata {
                    mod_id: "actual-a".to_string(),
                    name: "Actual A".to_string(),
                    version: "1".to_string(),
                    environment: Default::default(),
                    dependencies: Vec::new(),
                    provides: Vec::new(),
                    language_loader: None,
                    load_condition: crate::metadata::ModLoadCondition::IfPossible,
                    origin: crate::jar::JarModOrigin::Root,
                    embedded_jars: Vec::new(),
                    embedded_artifacts: Vec::new(),
                    bundled_mods: Vec::new(),
                },
                artifact("source-a", "project-a"),
            )
            .unwrap();
        download
            .record(
                crate::jar::JarModMetadata {
                    mod_id: "actual-b".to_string(),
                    name: "Actual B".to_string(),
                    version: "1".to_string(),
                    environment: Default::default(),
                    dependencies: Vec::new(),
                    provides: Vec::new(),
                    language_loader: None,
                    load_condition: crate::metadata::ModLoadCondition::IfPossible,
                    origin: crate::jar::JarModOrigin::Root,
                    embedded_jars: Vec::new(),
                    embedded_artifacts: Vec::new(),
                    bundled_mods: Vec::new(),
                },
                artifact("source-b", "project-b"),
            )
            .unwrap();

        assert_eq!(download.resolved.len(), 2);
        assert!(
            download
                .resolved
                .values()
                .any(|artifact| artifact.slug == "source-a")
        );
        assert!(
            download
                .resolved
                .values()
                .any(|artifact| artifact.slug == "source-b")
        );
        assert_eq!(
            download.source_packages[&("modrinth".to_string(), "source-a".to_string())],
            "actual-a"
        );
        assert_eq!(
            download.source_packages[&("modrinth".to_string(), "project-b".to_string())],
            "actual-b"
        );
    }

    #[test]
    fn candidate_catalog_preserves_duplicate_mod_versions_as_distinct_package_candidates() {
        fn metadata(
            dependencies: Vec<crate::metadata::ModDependency>,
        ) -> crate::jar::JarModMetadata {
            crate::jar::JarModMetadata {
                mod_id: "actual".to_string(),
                name: "Actual".to_string(),
                version: "1".to_string(),
                environment: Default::default(),
                dependencies: dependencies
                    .into_iter()
                    .map(crate::metadata::DependencyExpression::Only)
                    .collect(),
                provides: Vec::new(),
                language_loader: None,
                load_condition: crate::metadata::ModLoadCondition::IfPossible,
                origin: crate::jar::JarModOrigin::Root,
                embedded_jars: Vec::new(),
                embedded_artifacts: Vec::new(),
                bundled_mods: Vec::new(),
            }
        }

        let mut catalog = CandidateCatalog::default();
        catalog
            .record(metadata(Vec::new()), artifact("first", "project-a"))
            .unwrap();
        catalog
            .record(metadata(Vec::new()), artifact("mirror", "project-b"))
            .unwrap();

        assert_eq!(catalog.candidates["actual"].len(), 2);
        assert_eq!(catalog.resolved.len(), 2);
        assert_eq!(
            catalog.package_for_locator("mirror").unwrap().as_deref(),
            Some("actual")
        );

        catalog
            .record(
                metadata(vec![crate::metadata::ModDependency::required(
                    "different",
                    "*",
                )]),
                artifact("conflict", "project-c"),
            )
            .unwrap();
        assert_eq!(catalog.candidates["actual"].len(), 3);

        let error = catalog
            .record(
                metadata(vec![crate::metadata::ModDependency::required(
                    "same-artifact-different-metadata",
                    "*",
                )]),
                artifact("same-artifact", "project-c"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("different metadata"));
    }
}
