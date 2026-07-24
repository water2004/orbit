//! 过时检查编排层。
//!
//! BFS 下载候选 JAR → 解析 → PubGrub 离线求解。

use std::collections::{HashMap, HashSet};

use crate::error::OrbitError;
use crate::lockfile::{OrbitLockfile, PackageEntry};
use crate::manifest::OrbitManifest;
use crate::providers::{ModProvider, ResolvedMod};
use crate::resolver::types::{CandidateDiagnostic, CandidateVersion};

pub struct OutdatedMod {
    pub mod_id: String,
    pub current_version: String,
    pub new_version: String,
}

#[derive(Default)]
pub struct OutdatedReport {
    pub updates: Vec<OutdatedMod>,
    pub resolved: ResolvedCandidates,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

pub type ResolvedCandidateKey = (String, String);
pub type ResolvedCandidates = HashMap<ResolvedCandidateKey, ResolvedMod>;

#[derive(Default)]
pub struct CandidateDownload {
    pub candidates: HashMap<String, Vec<CandidateVersion>>,
    pub resolved: ResolvedCandidates,
    /// Provider 查询标识（slug 或 project_id）到 JAR 自声明 mod_id 的映射。
    pub source_packages: HashMap<String, String>,
}

/// BFS 下载 JAR 并构建候选图、候选元数据和 provider 标识映射。
/// 供 `install_mod` 和 `check_all_outdated` 共用。
pub async fn download_candidates_bfs(
    provider: &dyn ModProvider,
    seeds: &[String],
    lockfile: &OrbitLockfile,
    mc_version: &str,
    loader: &str,
) -> Result<CandidateDownload, OrbitError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut to_download: Vec<ResolvedMod> = Vec::new();
    let mut queue: Vec<String> = seeds.to_vec();

    while let Some(pid) = queue.pop() {
        if !seen.insert(pid.clone()) {
            continue;
        }
        let versions = provider
            .get_versions(&pid, Some(mc_version), Some(loader))
            .await?;
        for v in &versions {
            for dep in &v.dependencies {
                if dep.required
                    && let Some(ref pid) = dep.project_id
                    && !seen.contains(pid.as_str())
                {
                    queue.push(pid.clone());
                }
            }
            to_download.push(v.clone());
        }
    }
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
    let mut handles = Vec::new();

    for v in &to_download {
        let v = v.clone();
        let loader = loader.to_string();
        let sem = semaphore.clone();
        let lockfile_packages = lockfile.packages.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let metadata = crate::jar::download_and_parse(
                &v.download_url,
                &v.filename,
                &v.sha1,
                &v.sha512,
                &loader,
            )
            .await?;
            let key = if metadata.mod_id.is_empty() {
                lockfile_packages
                    .iter()
                    .find(|entry| {
                        entry.source_slug() == Some(v.slug.as_str())
                            || entry.source_project_id() == v.project_id()
                    })
                    .map(|entry| entry.mod_id.clone())
                    .unwrap_or_default()
            } else {
                metadata.mod_id.clone()
            };
            if key.is_empty() {
                return Ok(None);
            }
            Ok(Some((
                key,
                CandidateVersion::from_jar_metadata(metadata),
                v,
            )))
        }));
    }

    let mut download = CandidateDownload::default();
    let mut first_error = None;
    for handle in handles {
        match handle.await {
            Ok(Ok(Some((package, candidate, resolved)))) => {
                record_candidate(&mut download, package, candidate, resolved);
            }
            Ok(Ok(None)) => {}
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
    if download.candidates.is_empty()
        && let Some(error) = first_error
    {
        return Err(error);
    }
    Ok(download)
}

pub async fn download_candidates_with_fallback(
    providers: &[Box<dyn ModProvider>],
    seeds: &[String],
    lockfile: &OrbitLockfile,
    mc_version: &str,
    loader: &str,
) -> Result<CandidateDownload, OrbitError> {
    for provider in providers {
        match download_candidates_bfs(provider.as_ref(), seeds, lockfile, mc_version, loader).await
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

fn record_candidate(
    download: &mut CandidateDownload,
    package: String,
    candidate: CandidateVersion,
    resolved: ResolvedMod,
) {
    download
        .source_packages
        .insert(resolved.slug.clone(), package.clone());
    download
        .source_packages
        .insert(resolved.mod_id.clone(), package.clone());
    if let Some(project_id) = resolved.project_id() {
        download.source_packages.insert(project_id, package.clone());
    }
    download
        .resolved
        .insert((package.clone(), candidate.jar_version.clone()), resolved);
    download
        .candidates
        .entry(package)
        .or_default()
        .push(candidate);
}

/// 检查所有已安装平台模组的可用更新。
pub async fn check_all_outdated(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
) -> Result<OutdatedReport, OrbitError> {
    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;

    let mut seeds_by_provider: HashMap<String, Vec<String>> = HashMap::new();
    for entry in lockfile
        .packages
        .iter()
        .filter(|entry| entry.provider != "file")
    {
        let provider =
            crate::providers::find_provider(providers, &entry.provider).ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "lockfile contains {} packages but that provider is not configured",
                    entry.provider
                ))
            })?;
        let Some(project_id) = entry.source_project_id() else {
            continue;
        };
        let Some(version_id) = entry.source_version_id() else {
            continue;
        };
        let mut versions = provider
            .get_versions(&project_id, Some(mc_version), Some(loader))
            .await?;
        versions.sort_by(|left, right| right.date_published.cmp(&left.date_published));
        let Some(current_date) = versions
            .iter()
            .find(|version| version.version_id().as_deref() == Some(version_id.as_str()))
            .map(|version| version.date_published.as_str())
        else {
            continue;
        };
        if versions
            .iter()
            .any(|version| version.date_published.as_str() > current_date)
        {
            seeds_by_provider
                .entry(entry.provider.clone())
                .or_default()
                .push(project_id);
        }
    }

    if seeds_by_provider.is_empty() {
        return Ok(OutdatedReport::default());
    }

    // 2. Download candidates from each package's original provider, then solve once.
    let mut candidates = HashMap::new();
    let mut resolved = HashMap::new();
    for (provider_name, seeds) in seeds_by_provider {
        let Some(provider) = crate::providers::find_provider(providers, &provider_name) else {
            continue;
        };
        let download =
            download_candidates_bfs(provider, &seeds, lockfile, mc_version, loader).await?;
        for (package, versions) in download.candidates {
            candidates
                .entry(package)
                .or_insert_with(Vec::new)
                .extend(versions);
        }
        resolved.extend(download.resolved);
    }
    if candidates.is_empty() {
        return Ok(OutdatedReport::default());
    }

    // 3. Resolve
    let resolution = crate::resolver::resolve_with_candidates_report(
        manifest,
        lockfile,
        &mut candidates,
        providers,
    )
    .await
    .map_err(|e| OrbitError::Other(anyhow::anyhow!("{e}")))?;

    let mut updates: Vec<OutdatedMod> = resolution
        .upgrades
        .into_iter()
        .map(|(mod_id, new_version)| OutdatedMod {
            current_version: lockfile
                .find(&mod_id)
                .and_then(PackageEntry::source_version)
                .unwrap_or_default()
                .to_string(),
            new_version,
            mod_id,
        })
        .collect();
    updates.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    Ok(OutdatedReport {
        updates,
        resolved,
        diagnostics: resolution.diagnostics,
        warnings: resolution.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ModrinthResolvedInfo;

    fn resolved(source: &str, project_id: &str) -> ResolvedMod {
        ResolvedMod {
            mod_id: source.to_string(),
            version: "provider-version".to_string(),
            sha1: String::new(),
            sha512: String::new(),
            slug: source.to_string(),
            provider: "modrinth".to_string(),
            modrinth: Some(ModrinthResolvedInfo {
                project_id: project_id.to_string(),
                version_id: format!("{project_id}-version"),
                version_number: "provider-version".to_string(),
            }),
            curseforge: None,
            date_published: String::new(),
            download_url: String::new(),
            filename: String::new(),
            dependencies: Vec::new(),
            client_side: None,
            server_side: None,
        }
    }

    #[test]
    fn candidate_catalog_uses_actual_package_and_version_as_its_key() {
        let mut download = CandidateDownload::default();

        record_candidate(
            &mut download,
            "actual-a".to_string(),
            CandidateVersion {
                jar_version: "1".to_string(),
                dependencies: Vec::new(),
                environment: Default::default(),
                provides: Vec::new(),
                language_loader: None,
                embedded_artifacts: Vec::new(),
                bundled: Vec::new(),
            },
            resolved("source-a", "project-a"),
        );
        record_candidate(
            &mut download,
            "actual-b".to_string(),
            CandidateVersion {
                jar_version: "1".to_string(),
                dependencies: Vec::new(),
                environment: Default::default(),
                provides: Vec::new(),
                language_loader: None,
                embedded_artifacts: Vec::new(),
                bundled: Vec::new(),
            },
            resolved("source-b", "project-b"),
        );

        assert_eq!(download.resolved.len(), 2);
        assert!(
            download
                .resolved
                .contains_key(&("actual-a".to_string(), "1".to_string()))
        );
        assert!(
            download
                .resolved
                .contains_key(&("actual-b".to_string(), "1".to_string()))
        );
        assert_eq!(download.source_packages["source-a"], "actual-a");
        assert_eq!(download.source_packages["project-b"], "actual-b");
    }
}
