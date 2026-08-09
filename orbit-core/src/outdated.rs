//! 过时检查编排层。
//!
//! BFS 下载候选 JAR → 解析 → PubGrub 离线求解。

use std::collections::HashMap;

use crate::error::OrbitError;
use crate::loader::LoaderKind;
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
    pub loader: LoaderKind,
    pub java_feature: u32,
    pub storage: crate::version_repository::CandidateStorage<'a>,
    pub progress: Option<ProgressReporter>,
}

pub(crate) async fn download_candidate_catalog(
    input: CandidateDiscoveryInput<'_>,
    requested_remotes: &[PackageRemote],
) -> Result<CandidateCatalog, OrbitError> {
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

    let scope = input
        .storage
        .version_repository()
        .scope(input.mc_version, input.loader)?;
    let mut remote_seeds = Vec::new();
    let mut local = Vec::new();
    for (remote, requested) in all_remotes {
        match remote {
            PackageRemote::File { path } => {
                let resolved = resolve_file_remote(input.instance_dir, &path);
                let sha512 = crate::jar::compute_sha512(&resolved)?;
                let inspected = match scope.find_jar(&sha512, "")? {
                    Some(inspected) => inspected,
                    None => {
                        let inspected = crate::jar::inspect_path(&resolved, input.loader)?;
                        scope.store_jar(&inspected)?;
                        inspected
                    }
                };
                let filename =
                    local_candidate_filename(&path, &resolved, &inspected, input.lockfile)?;
                local.push((inspected, path, filename, requested));
            }
            remote => remote_seeds.push((remote, requested)),
        }
    }
    refresh_remote_repository(&input, &scope, &remote_seeds).await?;
    let mut catalog = scope.build_catalog(&remote_seeds, input.java_feature)?;
    for (inspected, path, filename, requested) in local {
        catalog.record_local(inspected, path, filename, requested)?;
    }
    Ok(catalog)
}

async fn refresh_remote_repository(
    input: &CandidateDiscoveryInput<'_>,
    scope: &crate::version_repository::RepositoryScope,
    seeds: &[(PackageRemote, bool)],
) -> Result<(), OrbitError> {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};

    let providers = input
        .providers
        .iter()
        .map(|provider| (provider.name(), provider.as_ref()))
        .collect::<HashMap<_, _>>();
    let mut queue = VecDeque::new();
    for (remote, _) in seeds {
        if let Some((provider, project_id)) = crate::version_repository::remote_project(remote) {
            queue.push_back((provider.to_string(), project_id, None::<String>));
        }
    }
    let mut visited = BTreeSet::new();
    let mut checked_projects = 0usize;
    let mut refreshed_projects = 0usize;
    let mut reused_projects = 0usize;
    let mut indexed_artifacts = 0usize;
    let mut pending_updates = Vec::new();
    let initial_total = queue
        .iter()
        .map(|(provider, project_id, _)| (provider, project_id))
        .collect::<BTreeSet<_>>()
        .len();
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::RepositoryIndexStarted {
            minecraft: input.mc_version.to_string(),
            loader: input.loader.to_string(),
            total: initial_total,
        },
    );
    while !queue.is_empty() {
        let mut wave = BTreeMap::<String, Vec<(String, Option<String>)>>::new();
        while let Some((provider, project_id, referenced_by)) = queue.pop_front() {
            if visited.insert((provider.clone(), project_id.clone())) {
                wave.entry(provider)
                    .or_default()
                    .push((project_id, referenced_by));
            }
        }
        for (provider_name, projects) in wave {
            let provider = providers.get(provider_name.as_str()).ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "package remotes require the unconfigured provider '{provider_name}'"
                ))
            })?;
            let project_ids = projects
                .iter()
                .map(|(project_id, _)| project_id.clone())
                .collect::<Vec<_>>();
            let states = provider.project_states(&project_ids).await?;
            let states = states
                .into_iter()
                .map(|state| (state.project_id.clone(), state))
                .collect::<HashMap<_, _>>();
            for (project_id, referenced_by) in projects {
                let state = states.get(&project_id).ok_or_else(|| {
                    let context = referenced_by
                        .as_ref()
                        .map(|parent| format!(" referenced by project '{parent}'"))
                        .unwrap_or_default();
                    OrbitError::Other(anyhow::anyhow!(
                        "{provider_name} batch project lookup did not return project '{project_id}'{context}"
                    ))
                })?;
                if state.marker.trim().is_empty() {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "{provider_name} project '{project_id}' returned an empty project change marker"
                    )));
                }
                let changed = scope
                    .project_marker(&provider_name, &project_id)?
                    .as_deref()
                    != Some(state.marker.as_str());
                let artifacts = if changed {
                    refreshed_projects += 1;
                    provider
                        .get_versions(
                            &project_id,
                            Some(input.mc_version),
                            Some(input.loader.as_str()),
                        )
                        .await
                        .map_err(|error| match error {
                            OrbitError::ModNotFound(_) if referenced_by.is_some() => {
                                OrbitError::Other(anyhow::anyhow!(
                                    "{provider_name} project '{}' references missing project '{project_id}'",
                                    referenced_by.unwrap_or_default()
                                ))
                            }
                            other => other,
                        })?
                } else {
                    reused_projects += 1;
                    scope
                        .project_artifacts(&provider_name, &project_id)?
                        .into_iter()
                        .map(|stored| stored.artifact)
                        .collect()
                };
                let artifact_count = artifacts.len();
                indexed_artifacts += artifact_count;
                for artifact in &artifacts {
                    for related in &artifact.related_projects {
                        let related_id = related.project_id.clone().ok_or_else(|| {
                            OrbitError::Other(anyhow::anyhow!(
                                "{provider_name} project '{project_id}' returned a dependency without a stable project ID"
                            ))
                        })?;
                        queue.push_back((
                            provider_name.clone(),
                            related_id,
                            Some(project_id.clone()),
                        ));
                    }
                }
                if changed {
                    pending_updates.push(PendingProjectUpdate {
                        provider: provider_name.clone(),
                        project_id: project_id.clone(),
                        marker: state.marker.clone(),
                        artifacts,
                    });
                }
                checked_projects += 1;
                let total = visited.len()
                    + queue
                        .iter()
                        .filter(|(provider, project, _)| {
                            !visited.contains(&(provider.clone(), project.clone()))
                        })
                        .map(|(provider, project, _)| (provider, project))
                        .collect::<BTreeSet<_>>()
                        .len();
                emit_progress(
                    input.progress.as_ref(),
                    ProgressEvent::RepositoryProjectChecked {
                        completed: checked_projects,
                        total,
                        provider: provider_name.clone(),
                        project_id,
                        refreshed: changed,
                        artifacts: artifact_count,
                    },
                );
            }
        }
    }
    for update in materialize_repository_updates(input, scope, pending_updates).await? {
        scope.replace_project(
            &update.provider,
            &update.project_id,
            &update.marker,
            &update.artifacts,
        )?;
    }
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::RepositoryIndexFinished {
            completed: checked_projects,
            total: checked_projects,
            refreshed: refreshed_projects,
            reused: reused_projects,
            artifacts: indexed_artifacts,
        },
    );
    Ok(())
}

struct PendingProjectUpdate {
    provider: String,
    project_id: String,
    marker: String,
    artifacts: Vec<RemoteArtifact>,
}

struct MaterializedProjectUpdate {
    provider: String,
    project_id: String,
    marker: String,
    artifacts: Vec<crate::version_repository::StoredRemoteArtifact>,
}

async fn materialize_repository_updates(
    input: &CandidateDiscoveryInput<'_>,
    scope: &crate::version_repository::RepositoryScope,
    updates: Vec<PendingProjectUpdate>,
) -> Result<Vec<MaterializedProjectUpdate>, OrbitError> {
    use futures_util::{StreamExt, stream};
    use std::collections::BTreeMap;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let providers = input
        .providers
        .iter()
        .map(|provider| (provider.name(), provider.artifact_downloader().clone()))
        .collect::<HashMap<_, _>>();
    let mut content = BTreeMap::<String, Vec<(usize, usize, RemoteArtifact)>>::new();
    for (update_index, update) in updates.iter().enumerate() {
        for (artifact_index, artifact) in update.artifacts.iter().cloned().enumerate() {
            content
                .entry(repository_artifact_content_key(&artifact))
                .or_default()
                .push((update_index, artifact_index, artifact));
        }
    }
    let total = content.len();
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::CandidateDownloadStarted { total },
    );
    let completed = Arc::new(AtomicUsize::new(0));
    let jobs = content
        .iter()
        .map(|(key, occurrences)| {
            let representative = occurrences[0].2.clone();
            let cached = scope.find_jar(&representative.sha512, &representative.sha1);
            (key.clone(), representative, cached)
        })
        .collect::<Vec<_>>();
    let results = stream::iter(jobs.into_iter().map(|(key, artifact, cached)| {
        let completed = completed.clone();
        let progress = input.progress.clone();
        let downloader = providers.get(artifact.provider.as_str()).cloned();
        async move {
            emit_progress(
                progress.as_ref(),
                ProgressEvent::CandidateArtifact {
                    completed: completed.load(Ordering::Relaxed),
                    total,
                    filename: artifact.filename.clone(),
                    state: ArtifactProgressState::Started,
                },
            );
            let (inspected, state) = match cached? {
                Some(inspected) => (inspected, ArtifactProgressState::AlreadyPresent),
                None => {
                    let downloader = downloader.ok_or_else(|| {
                        OrbitError::Other(anyhow::anyhow!(
                            "candidate artifact requires the unconfigured provider '{}'",
                            artifact.provider
                        ))
                    })?;
                    let inspected = crate::jar::download_and_inspect(
                        input.storage.jar_cache(),
                        &downloader,
                        &artifact.download_url,
                        &artifact.filename,
                        &artifact.sha1,
                        &artifact.sha512,
                        input.loader,
                    )
                    .await?;
                    (inspected, ArtifactProgressState::Finished)
                }
            };
            let completed = completed.fetch_add(1, Ordering::Relaxed) + 1;
            emit_progress(
                progress.as_ref(),
                ProgressEvent::CandidateArtifact {
                    completed,
                    total,
                    filename: artifact.filename,
                    state,
                },
            );
            Ok::<_, OrbitError>((key, inspected))
        }
    }))
    .buffer_unordered(total.max(1))
    .collect::<Vec<_>>()
    .await;
    let mut inspected_by_content = HashMap::new();
    for result in results {
        let (key, inspected) = result?;
        scope.store_jar(&inspected)?;
        inspected_by_content.insert(key, inspected);
    }
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::CandidateDownloadFinished { total },
    );
    let mut materialized = updates
        .into_iter()
        .map(|update| MaterializedProjectUpdate {
            provider: update.provider,
            project_id: update.project_id,
            marker: update.marker,
            artifacts: Vec::new(),
        })
        .collect::<Vec<_>>();
    for (key, occurrences) in content {
        let inspected = inspected_by_content.get(&key).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "candidate materialization omitted content '{key}'"
            ))
        })?;
        for (update_index, artifact_index, artifact) in occurrences {
            let artifacts = &mut materialized[update_index].artifacts;
            if artifacts.len() <= artifact_index {
                artifacts.resize_with(artifact_index + 1, || {
                    crate::version_repository::StoredRemoteArtifact {
                        artifact: artifact.clone(),
                        sha512: String::new(),
                    }
                });
            }
            artifacts[artifact_index] = crate::version_repository::StoredRemoteArtifact {
                artifact,
                sha512: inspected.sha512.clone(),
            };
        }
    }
    Ok(materialized)
}

fn repository_artifact_content_key(artifact: &RemoteArtifact) -> String {
    if !artifact.sha512.is_empty() {
        format!("sha512:{}", artifact.sha512.to_ascii_lowercase())
    } else if !artifact.sha1.is_empty() {
        format!("sha1:{}", artifact.sha1.to_ascii_lowercase())
    } else {
        format!("url:{}:{}", artifact.provider, artifact.download_url)
    }
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
    storage: crate::version_repository::CandidateStorage<'_>,
) -> Result<OutdatedReport, OrbitError> {
    check_all_outdated_with_progress(
        instance_dir,
        manifest,
        lockfile,
        providers,
        selector,
        storage,
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
    storage: crate::version_repository::CandidateStorage<'_>,
    progress: Option<ProgressReporter>,
) -> Result<OutdatedReport, OrbitError> {
    check_outdated_with_progress(OutdatedCheck {
        instance_dir,
        manifest,
        lockfile,
        providers,
        requested_package: None,
        selector,
        storage,
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
    storage: crate::version_repository::CandidateStorage<'_>,
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
        storage,
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
    storage: crate::version_repository::CandidateStorage<'a>,
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
        storage,
        progress,
    } = input;
    let platform = crate::platform::Platform::load(instance_dir, manifest)?;
    let loader = platform.loader;
    let mc_version = &manifest.project.mc_version;

    let manifest_remotes: Vec<_> = manifest
        .packages
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
            java_feature: platform.minecraft_version.java_version,
            storage,
            progress: progress.clone(),
        },
        &[],
    )
    .await?;
    catalog.loader_package = platform.loader_package;
    let discovery_diagnostics = missing_candidate_diagnostics(
        lockfile,
        &catalog,
        requested_package,
        mc_version,
        loader.as_str(),
    );
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
        crate::resolver::select_upgrade_resolution(portfolio, requested_package, selector)?;
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
        RemoteProjectState, SearchResultItem,
    };
    use async_trait::async_trait;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    struct RepositoryProvider {
        downloader: ArtifactDownloadClient,
        state_calls: Arc<Mutex<Vec<Vec<String>>>>,
        version_calls: Arc<Mutex<Vec<VersionCall>>>,
        projects: HashMap<String, Vec<RemoteArtifact>>,
    }

    type VersionCall = (String, Option<String>, Option<String>);

    #[async_trait]
    impl ModProvider for RepositoryProvider {
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

        async fn get_mod_info(&self, project: &str) -> Result<ModInfo, OrbitError> {
            Err(OrbitError::ModNotFound(project.to_string()))
        }

        async fn project_states(
            &self,
            project_ids: &[String],
        ) -> Result<Vec<RemoteProjectState>, OrbitError> {
            self.state_calls.lock().unwrap().push(project_ids.to_vec());
            Ok(project_ids
                .iter()
                .map(|project_id| RemoteProjectState {
                    project_id: project_id.clone(),
                    marker: "unchanged-marker".to_string(),
                })
                .collect())
        }

        async fn get_versions(
            &self,
            project: &str,
            mc_version: Option<&str>,
            loader: Option<&str>,
        ) -> Result<Vec<RemoteArtifact>, OrbitError> {
            self.version_calls.lock().unwrap().push((
                project.to_string(),
                mc_version.map(str::to_string),
                loader.map(str::to_string),
            ));
            if self.projects.is_empty() {
                Ok(Vec::new())
            } else {
                self.projects
                    .get(project)
                    .cloned()
                    .ok_or_else(|| OrbitError::ModNotFound(project.to_string()))
            }
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

    fn inspected(mod_id: &str, version: &str, sha512: &str) -> crate::jar::InspectedJar {
        crate::jar::InspectedJar {
            metadata: crate::jar::JarModMetadata {
                mod_id: mod_id.to_string(),
                name: mod_id.to_string(),
                version: version.to_string(),
                environment: crate::metadata::Environment::Both,
                dependencies: Vec::new(),
                provides: Vec::new(),
                language_loader: None,
                load_condition: crate::metadata::ModLoadCondition::Always,
                origin: crate::jar::JarModOrigin::Root,
                embedded_jars: Vec::new(),
                embedded_artifacts: Vec::new(),
                bundled_mods: Vec::new(),
            },
            sha1: format!("sha1-{sha512}"),
            sha256: format!("sha256-{sha512}"),
            sha512: sha512.to_string(),
        }
    }

    fn repository_artifact(
        project_id: &str,
        version: &str,
        sha512: &str,
        related_project: Option<&str>,
    ) -> RemoteArtifact {
        let mut artifact = artifact(project_id, project_id);
        artifact.sha512 = sha512.to_string();
        artifact.filename = format!("{project_id}-{version}.jar");
        artifact.download_url = format!("https://example.invalid/{}", artifact.filename);
        artifact.modrinth.as_mut().unwrap().version_id = format!("{project_id}-{version}");
        artifact.related_projects = related_project
            .map(|project_id| {
                vec![crate::providers::RemoteProjectLocator {
                    slug: None,
                    project_id: Some(project_id.to_string()),
                }]
            })
            .unwrap_or_default();
        artifact
    }

    #[tokio::test]
    async fn repository_batches_markers_filters_exact_scope_and_reuses_unchanged_projects() {
        let directory = tempfile::tempdir().unwrap();
        let state_calls = Arc::new(Mutex::new(Vec::new()));
        let version_calls = Arc::new(Mutex::new(Vec::new()));
        let providers: Vec<Box<dyn ModProvider>> = vec![Box::new(RepositoryProvider {
            downloader: ArtifactDownloadClient::test_anonymous("orbit-test").unwrap(),
            state_calls: state_calls.clone(),
            version_calls: version_calls.clone(),
            projects: HashMap::new(),
        })];
        let jar_cache =
            crate::jar_cache::JarCache::open(directory.path().join("jar-cache")).unwrap();
        let version_repository =
            crate::version_repository::VersionRepository::open(directory.path().join("repository"))
                .unwrap();
        let lockfile = OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: "1.21.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16".to_string(),
            },
            packages: Vec::new(),
        };
        let requested = ["project-a", "project-b"].map(|project_id| PackageRemote::Modrinth {
            project_id: project_id.to_string(),
        });

        for _ in 0..2 {
            download_candidate_catalog(
                CandidateDiscoveryInput {
                    instance_dir: directory.path(),
                    providers: &providers,
                    additional_remotes: &[],
                    lockfile: &lockfile,
                    mc_version: "1.21.1",
                    loader: LoaderKind::Fabric,
                    java_feature: 21,
                    storage: crate::version_repository::CandidateStorage::new(
                        &jar_cache,
                        &version_repository,
                    ),
                    progress: None,
                },
                &requested,
            )
            .await
            .unwrap();
        }

        let state_calls = state_calls.lock().unwrap();
        assert_eq!(state_calls.len(), 2);
        assert_eq!(state_calls[0], vec!["project-a", "project-b"]);
        assert_eq!(state_calls[1], vec!["project-a", "project-b"]);
        let version_calls = version_calls.lock().unwrap();
        assert_eq!(version_calls.len(), 2);
        assert_eq!(
            version_calls[0],
            (
                "project-a".to_string(),
                Some("1.21.1".to_string()),
                Some("fabric".to_string())
            )
        );
        assert_eq!(
            version_calls[1],
            (
                "project-b".to_string(),
                Some("1.21.1".to_string()),
                Some("fabric".to_string())
            )
        );
    }

    #[tokio::test]
    async fn repository_discovers_the_recursive_closure_before_one_deduplicated_materialization() {
        let directory = tempfile::tempdir().unwrap();
        let version_repository =
            crate::version_repository::VersionRepository::open(directory.path().join("repository"))
                .unwrap();
        let scope = version_repository
            .scope("1.21.1", LoaderKind::Fabric)
            .unwrap();
        for jar in [
            inspected("root", "1", "root-1"),
            inspected("root", "2", "root-2"),
            inspected("child", "1", "child-1"),
            inspected("grandchild", "1", "grandchild-1"),
        ] {
            scope.store_jar(&jar).unwrap();
        }
        let state_calls = Arc::new(Mutex::new(Vec::new()));
        let version_calls = Arc::new(Mutex::new(Vec::new()));
        let providers: Vec<Box<dyn ModProvider>> = vec![Box::new(RepositoryProvider {
            downloader: ArtifactDownloadClient::test_anonymous("orbit-test").unwrap(),
            state_calls,
            version_calls: version_calls.clone(),
            projects: HashMap::from([
                (
                    "root".to_string(),
                    vec![
                        repository_artifact("root", "1", "root-1", Some("child")),
                        repository_artifact("root", "2", "root-2", Some("child")),
                    ],
                ),
                (
                    "child".to_string(),
                    vec![repository_artifact(
                        "child",
                        "1",
                        "child-1",
                        Some("grandchild"),
                    )],
                ),
                (
                    "grandchild".to_string(),
                    vec![repository_artifact("grandchild", "1", "grandchild-1", None)],
                ),
            ]),
        })];
        let jar_cache =
            crate::jar_cache::JarCache::open(directory.path().join("jar-cache")).unwrap();
        let lockfile = OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: "1.21.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16".to_string(),
            },
            packages: Vec::new(),
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let progress: ProgressReporter = Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });

        let catalog = download_candidate_catalog(
            CandidateDiscoveryInput {
                instance_dir: directory.path(),
                providers: &providers,
                additional_remotes: &[],
                lockfile: &lockfile,
                mc_version: "1.21.1",
                loader: LoaderKind::Fabric,
                java_feature: 21,
                storage: crate::version_repository::CandidateStorage::new(
                    &jar_cache,
                    &version_repository,
                ),
                progress: Some(progress),
            },
            &[PackageRemote::Modrinth {
                project_id: "root".to_string(),
            }],
        )
        .await
        .unwrap();

        assert_eq!(catalog.candidates["root"].len(), 2);
        assert_eq!(catalog.candidates["child"].len(), 1);
        assert_eq!(catalog.candidates["grandchild"].len(), 1);
        assert!(catalog.requested_packages.contains("root"));
        assert_eq!(version_calls.lock().unwrap().len(), 3);
        let events = events.lock().unwrap();
        assert!(events.contains(&ProgressEvent::CandidateDownloadStarted { total: 4 }));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    ProgressEvent::CandidateArtifact {
                        state: ArtifactProgressState::AlreadyPresent,
                        ..
                    }
                ))
                .count(),
            4
        );
    }

    #[tokio::test]
    async fn repository_rejects_a_missing_related_project_before_materialization() {
        let directory = tempfile::tempdir().unwrap();
        let version_repository =
            crate::version_repository::VersionRepository::open(directory.path().join("repository"))
                .unwrap();
        version_repository
            .scope("1.21.1", LoaderKind::Fabric)
            .unwrap()
            .store_jar(&inspected("root", "1", "root-1"))
            .unwrap();
        let providers: Vec<Box<dyn ModProvider>> = vec![Box::new(RepositoryProvider {
            downloader: ArtifactDownloadClient::test_anonymous("orbit-test").unwrap(),
            state_calls: Arc::new(Mutex::new(Vec::new())),
            version_calls: Arc::new(Mutex::new(Vec::new())),
            projects: HashMap::from([(
                "root".to_string(),
                vec![repository_artifact("root", "1", "root-1", Some("missing"))],
            )]),
        })];
        let jar_cache =
            crate::jar_cache::JarCache::open(directory.path().join("jar-cache")).unwrap();
        let lockfile = OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: "1.21.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16".to_string(),
            },
            packages: Vec::new(),
        };

        let error = download_candidate_catalog(
            CandidateDiscoveryInput {
                instance_dir: directory.path(),
                providers: &providers,
                additional_remotes: &[],
                lockfile: &lockfile,
                mc_version: "1.21.1",
                loader: LoaderKind::Fabric,
                java_feature: 21,
                storage: crate::version_repository::CandidateStorage::new(
                    &jar_cache,
                    &version_repository,
                ),
                progress: None,
            },
            &[PackageRemote::Modrinth {
                project_id: "root".to_string(),
            }],
        )
        .await
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("references missing project 'missing'")
        );
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
        let inspected = crate::jar::inspect_path(&source, LoaderKind::Fabric).unwrap();
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
        assert_eq!(candidate.display_sources, ["CurseForge", "Modrinth"]);
        assert!(
            candidate
                .display_description()
                .contains("1 dependency constraint")
        );
        assert!(!candidate.display_description().contains("project-a"));
        assert!(!candidate.display_description().contains("release-a"));
        assert!(!candidate.display_description().contains("same-sha"));
        assert_eq!(catalog.resolved[&candidate.id].sources.len(), 2);
    }
}
