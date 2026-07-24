//! 过时检查编排层。
//!
//! BFS 下载候选 JAR → 解析 → PubGrub 离线求解。

use std::collections::{HashMap, HashSet};

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;
use crate::manifest::OrbitManifest;
use crate::providers::{ModProvider, ResolvedMod};
use crate::resolver::types::{CandidateDiagnostic, CandidateVersion, ImplantedCandidate};

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
        let versions = match provider
            .get_versions(&pid, Some(mc_version), Some(loader))
            .await
        {
            Ok(v) => v,
            Err(_) => continue,
        };
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
            let label = v
                .modrinth
                .as_ref()
                .map(|m| m.version_number.clone())
                .unwrap_or_default();
            match crate::jar::download_and_parse(&v.download_url, &v.filename, &v.sha512, &loader)
                .await
            {
                Ok(meta) => {
                    let key = if meta.mod_id.is_empty() {
                        lockfile_packages
                            .iter()
                            .find(|e| {
                                e.modrinth.as_ref().map(|m| m.slug.as_str()) == Some(&label)
                                    || e.modrinth.as_ref().map(|m| m.project_id.as_str())
                                        == Some(
                                            v.modrinth
                                                .as_ref()
                                                .map(|m| m.project_id.as_str())
                                                .unwrap_or(""),
                                        )
                            })
                            .map(|e| e.mod_id.clone())
                            .unwrap_or_default()
                    } else {
                        meta.mod_id.clone()
                    };
                    if key.is_empty() {
                        return None;
                    }
                    let imp_cands = meta
                        .implanted_mods
                        .into_iter()
                        .map(|im| crate::resolver::types::ImplantedCandidate {
                            mod_id: im.mod_id,
                            version: im.version,
                            deps: im.dependencies,
                        })
                        .collect();
                    Some((key, meta.version, meta.dependencies, imp_cands, v))
                }
                Err(_) => None,
            }
        }));
    }

    let mut download = CandidateDownload::default();
    for handle in handles {
        if let Ok(Some((package, version, dependencies, implanted, resolved))) = handle.await {
            record_candidate(
                &mut download,
                package,
                version,
                dependencies,
                implanted,
                resolved,
            );
        }
    }
    Ok(download)
}

fn record_candidate(
    download: &mut CandidateDownload,
    package: String,
    version: String,
    dependencies: Vec<(String, String, bool)>,
    implanted: Vec<ImplantedCandidate>,
    resolved: ResolvedMod,
) {
    download
        .source_packages
        .insert(resolved.slug.clone(), package.clone());
    download
        .source_packages
        .insert(resolved.mod_id.clone(), package.clone());
    if let Some(modrinth) = &resolved.modrinth {
        download
            .source_packages
            .insert(modrinth.project_id.clone(), package.clone());
    }
    download
        .resolved
        .insert((package.clone(), version.clone()), resolved);
    download
        .candidates
        .entry(package)
        .or_default()
        .push(CandidateVersion {
            jar_version: version,
            deps: dependencies,
            implanted,
        });
}

/// 检查所有已安装 modrinth mod 的可用更新。
pub async fn check_all_outdated(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
) -> Result<OutdatedReport, OrbitError> {
    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;
    let provider = &providers[0];

    let modrinth_entries: Vec<_> = lockfile
        .packages
        .iter()
        .filter(|e| e.modrinth.is_some())
        .collect();

    if modrinth_entries.is_empty() {
        return Ok(OutdatedReport::default());
    }

    // 1. Find outdated mods
    let mut seeds: Vec<String> = Vec::new();
    for entry in modrinth_entries {
        let mr = entry.modrinth.as_ref().unwrap();
        let mut versions = match provider
            .get_versions(&mr.project_id, Some(mc_version), Some(loader))
            .await
        {
            Ok(v) => v,
            Err(_) => continue,
        };
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
        let current_date = versions
            .iter()
            .find(|v| {
                v.modrinth.as_ref().map(|m| m.version_id.as_str()) == Some(mr.version_id.as_str())
            })
            .map(|v| v.date_published.clone());
        let Some(ref cd) = current_date else {
            continue;
        };
        let newer: Vec<_> = versions.iter().filter(|v| v.date_published > *cd).collect();
        if !newer.is_empty() {
            seeds.push(mr.project_id.clone());
        }
    }

    if seeds.is_empty() {
        return Ok(OutdatedReport::default());
    }

    // 2. BFS download
    let CandidateDownload {
        mut candidates,
        resolved,
        ..
    } = download_candidates_bfs(provider.as_ref(), &seeds, lockfile, mc_version, loader).await?;
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
                .and_then(|e| e.modrinth.as_ref().map(|m| m.version.clone()))
                .unwrap_or_default(),
            new_version,
            mod_id,
        })
        .collect();
    updates.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    Ok(OutdatedReport {
        updates,
        resolved,
        diagnostics: resolution.diagnostics,
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
            "1".to_string(),
            Vec::new(),
            Vec::new(),
            resolved("source-a", "project-a"),
        );
        record_candidate(
            &mut download,
            "actual-b".to_string(),
            "1".to_string(),
            Vec::new(),
            Vec::new(),
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
