//! 过时检查编排层。
//!
//! BFS 下载候选 JAR → 解析 → PubGrub 离线求解。

use std::collections::HashMap;

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;
use crate::manifest::{OrbitManifest, PackageRemote};
use crate::progress::{
    ArtifactProgressState, ProgressEvent, ProgressReporter, emit as emit_progress,
};
use crate::providers::{ModProvider, RemoteArtifact};
use crate::resolver::types::{
    CandidateCatalog, CandidateDiagnostic, PackageChange, PackageChangeKind, ResolutionReport,
    ResolutionSelector, ResolvedCandidates,
};

#[derive(Debug, Clone)]
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
    pub candidate_remotes: HashMap<String, Vec<PackageRemote>>,
    pub changes: Vec<PackageChange>,
    pub resolution: ResolutionReport,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct OutdatedInteraction {
    pub package: Option<String>,
    pub select_resolution: Option<ResolutionSelector>,
    pub progress: Option<ProgressReporter>,
}

#[derive(Debug)]
struct DiscoveredRemoteArtifact {
    artifact: RemoteArtifact,
    requested: bool,
}

async fn discover_artifact_closure(
    provider: &dyn ModProvider,
    initial_lookups: Vec<(String, bool)>,
    mc_version: &str,
    loader: &str,
    seen_lookups: &mut HashMap<String, bool>,
    seen_artifacts: &mut HashMap<String, usize>,
    progress: Option<&ProgressReporter>,
) -> Result<Vec<DiscoveredRemoteArtifact>, OrbitError> {
    let mut queue: Vec<_> = initial_lookups
        .into_iter()
        .map(|(lookup, requested)| (lookup, None::<String>, requested))
        .collect();
    let mut discovered: Vec<DiscoveredRemoteArtifact> = Vec::new();
    while let Some((lookup, referenced_by, requested)) = queue.pop() {
        match seen_lookups.entry(lookup.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(requested);
            }
            std::collections::hash_map::Entry::Occupied(mut entry)
                if requested && !*entry.get() =>
            {
                entry.insert(true);
            }
            std::collections::hash_map::Entry::Occupied(_) => continue,
        }
        emit_progress(
            progress,
            ProgressEvent::DiscoveringProject {
                provider: provider.name().to_string(),
                locator: lookup.clone(),
                pending_projects: queue.len(),
                artifacts_found: seen_artifacts.len(),
            },
        );
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
                    queue.push((project_id.clone(), Some(lookup.clone()), false));
                } else {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "{} project '{}' returned a dependency without a stable project ID",
                        provider.name(),
                        lookup
                    )));
                }
            }
            let artifact_key = format!(
                "{}:{}:{}:{}",
                artifact.provider,
                artifact.version_id().unwrap_or_default(),
                artifact.download_url,
                artifact.filename
            );
            if let Some(index) = seen_artifacts.get(&artifact_key).copied() {
                discovered[index].requested |= requested;
                continue;
            }
            seen_artifacts.insert(artifact_key, discovered.len());
            discovered.push(DiscoveredRemoteArtifact {
                artifact,
                requested,
            });
        }
    }
    Ok(discovered)
}

async fn download_artifact_queue(
    jobs: Vec<(
        crate::providers::ArtifactDownloadClient,
        DiscoveredRemoteArtifact,
    )>,
    loader: &str,
    jar_cache: &crate::jar_cache::JarCache,
    progress: Option<ProgressReporter>,
) -> Result<Vec<(crate::jar::InspectedJar, RemoteArtifact, bool)>, OrbitError> {
    let total = jobs.len();
    emit_progress(
        progress.as_ref(),
        ProgressEvent::CandidateDownloadStarted { total },
    );
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(jobs.len());
    for (downloader, discovered) in jobs {
        let artifact = discovered.artifact;
        let requested = discovered.requested;
        let loader = loader.to_string();
        let cache = jar_cache.clone();
        let semaphore = semaphore.clone();
        let completed = completed.clone();
        let progress = progress.clone();
        handles.push(tokio::spawn(async move {
            let _permit = semaphore.acquire_owned().await.map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "candidate download queue was closed: {error}"
                ))
            })?;
            let filename = artifact.filename.clone();
            emit_progress(
                progress.as_ref(),
                ProgressEvent::CandidateArtifact {
                    completed: completed.load(std::sync::atomic::Ordering::Relaxed),
                    total,
                    filename: filename.clone(),
                    state: ArtifactProgressState::Started,
                },
            );
            let result = crate::jar::download_and_inspect(
                &cache,
                &downloader,
                &artifact.download_url,
                &artifact.filename,
                &artifact.sha1,
                &artifact.sha512,
                &loader,
            )
            .await;
            let completed = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            emit_progress(
                progress.as_ref(),
                ProgressEvent::CandidateArtifact {
                    completed,
                    total,
                    filename,
                    state: if result.is_ok() {
                        ArtifactProgressState::Finished
                    } else {
                        ArtifactProgressState::Failed
                    },
                },
            );
            result.map(|inspected| (inspected, artifact, requested))
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
    emit_progress(
        progress.as_ref(),
        ProgressEvent::CandidateDownloadFinished { total },
    );
    Ok(parsed)
}

/// Build the complete candidate universe needed by an add/upgrade transaction.
///
/// A transaction is allowed to move already-installed packages to other
/// versions. Consequently the requested provider project alone is not a
/// complete solver input: every online project already represented by the
/// lockfile must contribute all of its matching artifacts as well. Discovery
/// finishes for every project before the combined queue is downloaded.
pub(crate) struct CandidateDiscoveryInput<'a> {
    pub instance_dir: &'a std::path::Path,
    pub providers: &'a [Box<dyn ModProvider>],
    pub additional_remotes: &'a [PackageRemote],
    pub lockfile: &'a OrbitLockfile,
    pub mc_version: &'a str,
    pub loader: &'a str,
    pub jar_cache: &'a crate::jar_cache::JarCache,
    pub progress: Option<ProgressReporter>,
}

pub(crate) async fn download_candidate_catalog(
    input: CandidateDiscoveryInput<'_>,
    requested_remotes: &[PackageRemote],
) -> Result<CandidateCatalog, OrbitError> {
    emit_progress(input.progress.as_ref(), ProgressEvent::DiscoveryStarted);
    let jobs = discover_transaction_artifact_queue(&input, requested_remotes).await?;
    let (jobs, local) = jobs;
    let mut catalog = CandidateCatalog::default();
    for (inspected, path, filename, requested) in local {
        catalog.record_local(inspected, path, filename, requested)?;
    }
    for (inspected, artifact, requested) in
        download_artifact_queue(jobs, input.loader, input.jar_cache, input.progress).await?
    {
        catalog.record(inspected.metadata.clone(), artifact, &inspected, requested)?;
    }
    Ok(catalog)
}

async fn discover_transaction_artifact_queue(
    input: &CandidateDiscoveryInput<'_>,
    requested_remotes: &[PackageRemote],
) -> Result<
    (
        Vec<(
            crate::providers::ArtifactDownloadClient,
            DiscoveredRemoteArtifact,
        )>,
        Vec<(crate::jar::InspectedJar, String, String, bool)>,
    ),
    OrbitError,
> {
    #[derive(Default)]
    struct ProviderDiscovery {
        seen_lookups: HashMap<String, bool>,
        seen_artifacts: HashMap<String, usize>,
    }

    let mut all_remotes = std::collections::BTreeMap::<PackageRemote, bool>::new();
    for remote in requested_remotes {
        all_remotes.insert(remote.clone(), true);
    }
    for remote in input.additional_remotes.iter().chain(
        input
            .lockfile
            .packages
            .iter()
            .flat_map(|entry| entry.remotes.iter()),
    ) {
        all_remotes.entry(remote.clone()).or_insert(false);
    }

    let mut states: HashMap<String, ProviderDiscovery> = HashMap::new();
    let mut jobs = Vec::new();
    let mut local = Vec::new();
    let mut seeds_by_provider: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for (remote, requested) in all_remotes {
        match remote {
            PackageRemote::File { path } => {
                let resolved = resolve_file_remote(input.instance_dir, &path);
                let inspected = crate::jar::inspect_path(&resolved, input.loader)?;
                let filename =
                    local_candidate_filename(&path, &resolved, &inspected, input.lockfile)?;
                local.push((inspected, path, filename, requested));
            }
            remote => {
                seeds_by_provider
                    .entry(remote.provider().to_string())
                    .or_default()
                    .push((remote.locator(), requested));
            }
        }
    }
    for provider in input.providers {
        let Some(seeds) = seeds_by_provider.remove(provider.name()) else {
            continue;
        };
        let state = states.entry(provider.name().to_string()).or_default();
        let artifacts = discover_artifact_closure(
            provider.as_ref(),
            seeds,
            input.mc_version,
            input.loader,
            &mut state.seen_lookups,
            &mut state.seen_artifacts,
            input.progress.as_ref(),
        )
        .await?;
        let downloader = provider.artifact_downloader().clone();
        jobs.extend(
            artifacts
                .into_iter()
                .map(|artifact| (downloader.clone(), artifact)),
        );
    }
    if let Some((provider, _)) = seeds_by_provider.into_iter().next() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "package remotes require the unconfigured provider '{provider}'"
        )));
    }
    if jobs.is_empty() && local.is_empty() && !requested_remotes.is_empty() {
        return Err(OrbitError::ModNotFound(
            requested_remotes
                .first()
                .map(PackageRemote::display_locator)
                .unwrap_or_default(),
        ));
    }
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::DiscoveryFinished {
            projects: states.values().map(|state| state.seen_lookups.len()).sum(),
            artifacts: jobs.len() + local.len(),
        },
    );
    Ok((jobs, local))
}

fn resolve_file_remote(instance_dir: &std::path::Path, path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        instance_dir.join(path)
    }
}

fn local_candidate_filename(
    remote_path: &str,
    resolved: &std::path::Path,
    inspected: &crate::jar::InspectedJar,
    lockfile: &OrbitLockfile,
) -> Result<String, OrbitError> {
    if remote_path
        .replace('\\', "/")
        .starts_with(".orbit/sources/")
    {
        if let Some(filename) = lockfile
            .packages
            .iter()
            .find(|entry| entry.sha512.eq_ignore_ascii_case(&inspected.sha512))
            .map(|entry| entry.filename.as_str())
            .filter(|filename| !filename.is_empty())
        {
            return Ok(filename.to_string());
        }
        return Ok(format!(
            "{}-{}.jar",
            safe_filename_component(&inspected.metadata.mod_id),
            safe_filename_component(&inspected.metadata.version)
        ));
    }
    resolved
        .file_name()
        .map(|filename| filename.to_string_lossy().into_owned())
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "local remote '{}' has no filename",
                resolved.display()
            ))
        })
}

fn safe_filename_component(value: &str) -> String {
    let mut component: String = value
        .chars()
        .take(96)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    while component.ends_with('.') || component.ends_with(' ') {
        component.pop();
    }
    if component.is_empty() {
        "package".to_string()
    } else {
        component
    }
}

/// 检查所有已安装平台模组的可用更新。
pub async fn check_all_outdated(
    instance_dir: &std::path::Path,
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
    selector: Option<ResolutionSelector>,
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<OutdatedReport, OrbitError> {
    check_all_outdated_with_progress(
        instance_dir,
        manifest,
        lockfile,
        providers,
        selector,
        jar_cache,
        None,
    )
    .await
}

/// Variant of [`check_all_outdated`] that reports candidate discovery,
/// download, parsing, and resolution progress.
pub async fn check_all_outdated_with_progress(
    instance_dir: &std::path::Path,
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
    selector: Option<ResolutionSelector>,
    jar_cache: &crate::jar_cache::JarCache,
    progress: Option<ProgressReporter>,
) -> Result<OutdatedReport, OrbitError> {
    check_outdated_with_progress(OutdatedCheck {
        instance_dir,
        manifest,
        lockfile,
        providers,
        requested_package: None,
        selector,
        jar_cache,
        progress,
    })
    .await
}

/// Check upgrades with an optional logical-package scope and UI callbacks.
///
/// Even when `interaction.package` is set, the complete instance candidate
/// universe is resolved because a feasible result may require coordinated
/// upgrades or downgrades elsewhere.
pub async fn check_outdated_with_interaction(
    instance_dir: &std::path::Path,
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    interaction: OutdatedInteraction,
) -> Result<OutdatedReport, OrbitError> {
    let OutdatedInteraction {
        package,
        select_resolution,
        progress,
    } = interaction;
    check_outdated_with_progress(OutdatedCheck {
        instance_dir,
        manifest,
        lockfile,
        providers,
        requested_package: package.as_deref(),
        selector: select_resolution,
        jar_cache,
        progress,
    })
    .await
}

struct OutdatedCheck<'a> {
    instance_dir: &'a std::path::Path,
    manifest: &'a OrbitManifest,
    lockfile: &'a OrbitLockfile,
    providers: &'a [Box<dyn ModProvider>],
    requested_package: Option<&'a str>,
    selector: Option<ResolutionSelector>,
    jar_cache: &'a crate::jar_cache::JarCache,
    progress: Option<ProgressReporter>,
}

async fn check_outdated_with_progress(
    input: OutdatedCheck<'_>,
) -> Result<OutdatedReport, OrbitError> {
    let OutdatedCheck {
        instance_dir,
        manifest,
        lockfile,
        providers,
        requested_package,
        selector,
        jar_cache,
        progress,
    } = input;
    let platform = crate::platform::Platform::load(instance_dir, manifest)?;
    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;

    let manifest_remotes: Vec<_> = manifest
        .dependencies
        .values()
        .flat_map(|dependency| dependency.remotes.iter().cloned())
        .collect();
    let mut catalog = download_candidate_catalog(
        CandidateDiscoveryInput {
            instance_dir,
            providers,
            additional_remotes: &manifest_remotes,
            lockfile,
            mc_version,
            loader,
            jar_cache,
            progress: progress.clone(),
        },
        &[],
    )
    .await?;
    catalog.loader_package = platform.loader_package;
    let discovery_diagnostics =
        missing_candidate_diagnostics(lockfile, &catalog, requested_package, mc_version, loader);
    if catalog.candidates.is_empty() {
        let candidate_remotes = catalog.package_remotes();
        return Ok(OutdatedReport {
            resolved: catalog.resolved,
            candidate_remotes,
            diagnostics: discovery_diagnostics,
            ..OutdatedReport::default()
        });
    }

    // 3. Resolve
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionStarted {
            packages: catalog.candidates.len(),
            candidates: catalog.candidates.values().map(Vec::len).sum(),
        },
    );
    let portfolio = crate::resolver::resolve_candidate_portfolio_with_progress(
        manifest,
        lockfile,
        &catalog,
        progress.clone(),
    )
    .await
    .map_err(|e| OrbitError::Other(anyhow::anyhow!("{e}")))?;
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionFinished {
            solutions: portfolio
                .alternatives
                .iter()
                .filter(|alternative| {
                    alternative.changes.iter().any(|change| {
                        change.kind == PackageChangeKind::Upgrade
                            && requested_package.is_none_or(|package| change.package == package)
                    })
                })
                .count(),
        },
    );
    let mut resolution =
        crate::resolver::select_upgrade_resolution(portfolio, requested_package, selector)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("{e}")))?;
    resolution.diagnostics.extend(discovery_diagnostics);
    resolution.diagnostics =
        crate::resolver::normalize_candidate_diagnostics(resolution.diagnostics);

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
        candidate_remotes: catalog.package_remotes(),
        resolved: catalog.resolved,
        changes: resolution.changes.clone(),
        diagnostics: resolution.diagnostics.clone(),
        warnings: resolution.warnings.clone(),
        resolution,
    })
}

fn missing_candidate_diagnostics(
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
    requested_package: Option<&str>,
    mc_version: &str,
    loader: &str,
) -> Vec<CandidateDiagnostic> {
    let mut diagnostics: Vec<_> = lockfile
        .packages
        .iter()
        .filter(|entry| !entry.remotes.is_empty())
        .filter(|entry| {
            requested_package.is_none_or(|package| entry.mod_id == package)
                && !catalog.candidates.contains_key(&entry.mod_id)
        })
        .map(|entry| CandidateDiagnostic {
            package: entry.mod_id.clone(),
            selected_version: entry.version.clone(),
            candidate_version: "none".to_string(),
            kind: crate::resolver::types::CandidateDiagnosticKind::NoCompatibleCandidate,
            facts: vec![format!(
                "{} returned no JAR for Minecraft {mc_version} / {loader} declaring this mod ID",
                entry
                    .remotes
                    .iter()
                    .map(PackageRemote::display_locator)
                    .collect::<Vec<_>>()
                    .join(", ")
            )],
        })
        .collect();
    diagnostics.sort_by(|left, right| left.package.cmp(&right.package));
    diagnostics.dedup_by(|left, right| left.package == right.package);
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        ArtifactDownloadClient, CurseForgeResolvedInfo, ModInfo, ModrinthResolvedInfo,
        RemoteProjectLocator, SearchResultItem,
    };
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::io::Write;
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

    fn fabric_jar_bytes(mod_id: &str, version: &str) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        archive
            .start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"{{"schemaVersion":1,"id":"{mod_id}","version":"{version}","name":"Test"}}"#
        )
        .unwrap();
        archive.finish().unwrap().into_inner()
    }

    #[test]
    fn managed_source_identity_never_becomes_the_installed_filename() {
        let directory = tempfile::tempdir().unwrap();
        let source_dir = directory.path().join(".orbit/sources");
        std::fs::create_dir_all(&source_dir).unwrap();
        let bytes = fabric_jar_bytes("voxy", "1.0+mc");
        let sha512 = crate::jar::sha512_digest(&bytes);
        let source = source_dir.join(format!("{sha512}.jar"));
        std::fs::write(&source, bytes).unwrap();
        let inspected = crate::jar::inspect_path(&source, "fabric").unwrap();
        let lockfile = OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: "1.21.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16.10".to_string(),
            },
            packages: Vec::new(),
        };

        let filename = local_candidate_filename(
            &format!(".orbit/sources/{sha512}.jar"),
            &source,
            &inspected,
            &lockfile,
        )
        .unwrap();

        assert_eq!(filename, "voxy-1.0-mc.jar");
        assert!(!filename.contains(&sha512));
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
        let mut seen_lookups = HashMap::new();
        let mut seen_artifacts = HashMap::new();

        let artifacts = discover_artifact_closure(
            &provider,
            vec![("root".to_string(), true)],
            "1.21.1",
            "fabric",
            &mut seen_lookups,
            &mut seen_artifacts,
            None,
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
    async fn a_direct_seed_promotes_artifacts_already_seen_through_a_relation() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let shared = related_artifact("shared", "shared", "1", None);
        let provider = DiscoveryProvider {
            downloader: ArtifactDownloadClient::anonymous("orbit-test").unwrap(),
            projects: HashMap::from([("shared".to_string(), vec![shared])]),
            calls: calls.clone(),
        };
        let mut seen_lookups = HashMap::new();
        let mut seen_artifacts = HashMap::new();

        // LIFO traversal visits the transitive occurrence first. The later
        // direct occurrence must still promote the same bytes to "requested".
        let artifacts = discover_artifact_closure(
            &provider,
            vec![("shared".to_string(), true), ("shared".to_string(), false)],
            "1.21.1",
            "fabric",
            &mut seen_lookups,
            &mut seen_artifacts,
            None,
        )
        .await
        .unwrap();

        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].requested);
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|lookup| lookup.as_str() == "shared")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn cached_candidate_download_reports_start_item_and_completion() {
        let directory = tempfile::tempdir().unwrap();
        let cache = crate::jar_cache::JarCache::open(directory.path().join("cache")).unwrap();
        let bytes = fabric_jar_bytes("voxy", "1.0.0");
        let sha512 = crate::jar::sha512_digest(&bytes);
        cache.store_bytes(&bytes).unwrap();

        let mut candidate = related_artifact("voxy", "voxy-project", "1", None);
        candidate.sha512 = sha512;
        candidate.filename = "voxy.jar".to_string();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let reporter: ProgressReporter = Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });

        let parsed = download_artifact_queue(
            vec![(
                ArtifactDownloadClient::anonymous("orbit-test").unwrap(),
                DiscoveredRemoteArtifact {
                    artifact: candidate,
                    requested: true,
                },
            )],
            "fabric",
            &cache,
            Some(reporter),
        )
        .await
        .unwrap();

        assert_eq!(parsed[0].0.metadata.mod_id, "voxy");
        let events = events.lock().unwrap();
        assert_eq!(
            events.first(),
            Some(&ProgressEvent::CandidateDownloadStarted { total: 1 })
        );
        assert!(events.contains(&ProgressEvent::CandidateArtifact {
            completed: 0,
            total: 1,
            filename: "voxy.jar".to_string(),
            state: ArtifactProgressState::Started,
        }));
        assert!(events.contains(&ProgressEvent::CandidateArtifact {
            completed: 1,
            total: 1,
            filename: "voxy.jar".to_string(),
            state: ArtifactProgressState::Finished,
        }));
        assert_eq!(
            events.last(),
            Some(&ProgressEvent::CandidateDownloadFinished { total: 1 })
        );
    }

    #[tokio::test]
    async fn add_discovery_includes_every_version_of_existing_online_packages() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = DiscoveryProvider {
            downloader: ArtifactDownloadClient::anonymous("orbit-test").unwrap(),
            projects: HashMap::from([
                (
                    "requested".to_string(),
                    vec![related_artifact(
                        "requested",
                        "requested-project",
                        "1",
                        None,
                    )],
                ),
                (
                    "existing-project".to_string(),
                    vec![
                        related_artifact("existing", "existing-project", "old", None),
                        related_artifact("existing", "existing-project", "compatible", None),
                    ],
                ),
            ]),
            calls: calls.clone(),
        };
        let providers: Vec<Box<dyn ModProvider>> = vec![Box::new(provider)];
        let lockfile = OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: "26.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.19.2".to_string(),
            },
            packages: vec![crate::lockfile::PackageEntry {
                mod_id: "existing".to_string(),
                version: "old".to_string(),
                sha1: String::new(),
                sha256: "hash".to_string(),
                sha512: String::new(),
                filename: "existing-old.jar".to_string(),
                remotes: vec![PackageRemote::Modrinth {
                    project_id: "existing-project".to_string(),
                }],
                artifact_sources: vec![crate::lockfile::ArtifactSource::Modrinth {
                    project_id: "existing-project".to_string(),
                    version_id: "existing-old".to_string(),
                    download_url: String::new(),
                }],
                dependencies: Vec::new(),
                environment: crate::metadata::Environment::Both,
                provides: Vec::new(),
                language_loader: None,
                embedded_artifacts: Vec::new(),
                bundled: Vec::new(),
            }],
        };

        let jar_cache =
            crate::jar_cache::JarCache::open(std::path::PathBuf::from("unused-cache")).unwrap();
        let input = CandidateDiscoveryInput {
            instance_dir: std::path::Path::new("."),
            providers: &providers,
            additional_remotes: &[],
            lockfile: &lockfile,
            mc_version: "26.1",
            loader: "fabric",
            jar_cache: &jar_cache,
            progress: None,
        };
        let jobs = discover_transaction_artifact_queue(
            &input,
            &[PackageRemote::Modrinth {
                project_id: "requested".to_string(),
            }],
        )
        .await
        .unwrap();
        let filenames = jobs
            .0
            .into_iter()
            .map(|(_, artifact)| artifact.artifact.filename)
            .collect::<HashSet<_>>();

        assert_eq!(
            filenames,
            HashSet::from([
                "requested-project-1.jar".to_string(),
                "existing-project-old.jar".to_string(),
                "existing-project-compatible.jar".to_string(),
            ])
        );
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &["requested".to_string(), "existing-project".to_string()]
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
        let mut seen_lookups = HashMap::new();
        let mut seen_artifacts = HashMap::new();

        let error = discover_artifact_closure(
            &provider,
            vec![("root".to_string(), true)],
            "1.21.1",
            "fabric",
            &mut seen_lookups,
            &mut seen_artifacts,
            None,
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("artifact closure is incomplete"));
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn missing_jar_declared_package_is_reported_instead_of_called_up_to_date() {
        let entry = crate::lockfile::PackageEntry {
            mod_id: "missing-package".to_string(),
            version: "1.0.0".to_string(),
            sha1: String::new(),
            sha256: "hash".to_string(),
            sha512: String::new(),
            filename: "ignored-by-presentation.jar".to_string(),
            remotes: vec![PackageRemote::Modrinth {
                project_id: "project".to_string(),
            }],
            artifact_sources: vec![crate::lockfile::ArtifactSource::Modrinth {
                project_id: "project".to_string(),
                version_id: "version".to_string(),
                download_url: String::new(),
            }],
            dependencies: Vec::new(),
            environment: crate::metadata::Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        };
        let lockfile = OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: "26.1.2".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.19.2".to_string(),
            },
            packages: vec![entry],
        };

        let diagnostics = missing_candidate_diagnostics(
            &lockfile,
            &CandidateCatalog::default(),
            Some("missing-package"),
            "26.1.2",
            "fabric",
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].kind,
            crate::resolver::types::CandidateDiagnosticKind::NoCompatibleCandidate
        );
        assert!(diagnostics[0].facts[0].contains("declaring this mod ID"));
        assert!(!diagnostics[0].facts[0].contains(".jar"));
    }

    #[test]
    fn candidate_catalog_uses_actual_package_and_version_as_its_key() {
        let mut download = CandidateCatalog::default();

        download
            .record_test(
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
            .record_test(
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
        assert_eq!(
            download.remote_packages[&PackageRemote::Modrinth {
                project_id: "project-a".to_string()
            }],
            std::collections::BTreeSet::from(["actual-a".to_string()])
        );
        assert_eq!(
            download.remote_packages[&PackageRemote::Modrinth {
                project_id: "project-b".to_string()
            }],
            std::collections::BTreeSet::from(["actual-b".to_string()])
        );
    }

    #[test]
    fn candidate_catalog_partitions_project_artifacts_by_actual_mod_id() {
        let mut catalog = CandidateCatalog::default();
        for (mod_id, version) in [("gca-wrapper", "1.0.1"), ("gca_wrapper", "1.0.6")] {
            let mut remote = artifact("gca", "UHjbX5mk");
            remote.modrinth.as_mut().unwrap().version_id = version.to_string();
            catalog
                .record_test(
                    crate::jar::JarModMetadata {
                        mod_id: mod_id.to_string(),
                        name: "GCA wrapper".to_string(),
                        version: version.to_string(),
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
                    remote,
                )
                .unwrap();
        }

        assert_eq!(catalog.candidates["gca-wrapper"].len(), 1);
        assert_eq!(catalog.candidates["gca_wrapper"].len(), 1);
        assert_eq!(
            catalog.packages_for_remote(&PackageRemote::Modrinth {
                project_id: "UHjbX5mk".to_string()
            }),
            vec!["gca-wrapper", "gca_wrapper"]
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
            .record_test(metadata(Vec::new()), artifact("first", "project-a"))
            .unwrap();
        catalog
            .record_test(metadata(Vec::new()), artifact("mirror", "project-b"))
            .unwrap();

        assert_eq!(catalog.candidates["actual"].len(), 2);
        assert_eq!(catalog.resolved.len(), 2);
        assert_eq!(
            catalog.packages_for_remote(&PackageRemote::Modrinth {
                project_id: "project-b".to_string()
            }),
            vec!["actual"]
        );

        catalog
            .record_test(
                metadata(vec![crate::metadata::ModDependency::required(
                    "different",
                    "*",
                )]),
                artifact("conflict", "project-c"),
            )
            .unwrap();
        assert_eq!(catalog.candidates["actual"].len(), 3);

        let error = catalog
            .record_test(
                metadata(vec![crate::metadata::ModDependency::required(
                    "same-artifact-different-metadata",
                    "*",
                )]),
                artifact("same-artifact", "project-c"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("inconsistent JAR metadata"));
    }

    #[test]
    fn identical_content_from_different_providers_is_one_human_readable_candidate() {
        let metadata = crate::jar::JarModMetadata {
            mod_id: "actual".to_string(),
            name: "Actual".to_string(),
            version: "1".to_string(),
            environment: Default::default(),
            dependencies: vec![crate::metadata::ModDependency::required("sodium", ">=0.9").into()],
            provides: Vec::new(),
            language_loader: None,
            load_condition: crate::metadata::ModLoadCondition::IfPossible,
            origin: crate::jar::JarModOrigin::Root,
            embedded_jars: Vec::new(),
            embedded_artifacts: Vec::new(),
            bundled_mods: Vec::new(),
        };
        let inspected = crate::jar::InspectedJar {
            metadata: metadata.clone(),
            sha1: "same-sha1".to_string(),
            sha256: "same-sha256".to_string(),
            sha512: "same-sha512".to_string(),
        };
        let mut modrinth = artifact("release-a", "project-a");
        modrinth.sha512 = inspected.sha512.clone();
        let curseforge = crate::providers::RemoteArtifact {
            sha1: inspected.sha1.clone(),
            sha512: inspected.sha512.clone(),
            slug: "actual".to_string(),
            provider: "curseforge".to_string(),
            modrinth: None,
            curseforge: Some(CurseForgeResolvedInfo {
                project_id: 123,
                file_id: 456,
                fingerprint: 789,
            }),
            download_url: "https://edge.forgecdn.net/files/actual.jar".to_string(),
            filename: "actual-cf.jar".to_string(),
            related_projects: Vec::new(),
        };

        let mut catalog = CandidateCatalog::default();
        catalog
            .record(metadata.clone(), modrinth, &inspected, true)
            .unwrap();
        catalog
            .record(metadata, curseforge, &inspected, true)
            .unwrap();

        assert_eq!(catalog.candidates["actual"].len(), 1);
        assert_eq!(catalog.resolved.len(), 1);
        let candidate = &catalog.candidates["actual"][0];
        assert_eq!(candidate.display_sources.len(), 2);
        assert!(
            candidate
                .display_description()
                .contains("requires sodium >=0.9")
        );
        assert!(!candidate.display_description().contains("same-sha"));
        assert_eq!(catalog.resolved[&candidate.id].sources.len(), 2);
    }
}
