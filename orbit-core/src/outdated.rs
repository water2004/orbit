//! 过时检查编排层。
//!
//! BFS 下载候选 JAR → 解析 → PubGrub 离线求解。

use std::collections::{HashMap, HashSet};

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;
use crate::manifest::OrbitManifest;
use crate::providers::ModProvider;
use crate::resolver::types::CandidateVersion;

pub struct OutdatedMod {
    pub mod_id: String,
    pub current_version: String,
    pub new_version: String,
}

/// BFS 下载 JAR 并构建 candidates + jar_ver_to_v。
/// 供 `install_mod` 和 `check_all_outdated` 共用。
pub async fn download_candidates_bfs(
    provider: &dyn ModProvider,
    seeds: &[String],
    lockfile: &OrbitLockfile,
    mc_version: &str,
    loader: &str,
) -> Result<(HashMap<String, Vec<CandidateVersion>>, HashMap<String, crate::providers::ResolvedMod>), OrbitError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut to_download: Vec<crate::providers::ResolvedMod> = Vec::new();
    let mut queue: Vec<String> = seeds.to_vec();

    while let Some(pid) = queue.pop() {
        if !seen.insert(pid.clone()) { continue; }
        let versions = match provider.get_versions(&pid, Some(mc_version), Some(loader)).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        for v in &versions {
            for dep in &v.dependencies {
                if dep.required {
                    if let Some(ref pid) = dep.project_id {
                        if !seen.contains(pid.as_str()) { queue.push(pid.clone()); }
                    }
                }
            }
            to_download.push(v.clone());
        }
    }
    eprintln!("  BFS query done: {} versions across {} projects", to_download.len(), seen.len());

    let mut jar_ver_to_v: HashMap<String, crate::providers::ResolvedMod> = HashMap::new();
    let mut candidates: HashMap<String, Vec<CandidateVersion>> = HashMap::new();

    for v in &to_download {
        let label = v.modrinth.as_ref().map(|m| m.version_number.as_str()).unwrap_or("?");
        match crate::jar::download_and_parse(&v.download_url, &v.sha512, loader).await {
            Ok(meta) => {
                let imp_cands = meta.implanted_mods.into_iter().map(|im| {
                    crate::resolver::types::ImplantedCandidate {
                        mod_id: im.mod_id, version: im.version, deps: im.dependencies,
                    }
                }).collect();
                // 用 lockfile 中的 mod_id 作为 key
                let key = if meta.mod_id.is_empty() {
                    lockfile.packages.iter()
                        .find(|e| e.modrinth.as_ref().map(|m| m.slug.as_str()) == Some(label)
                            || e.modrinth.as_ref().map(|m| m.project_id.as_str()) == Some(v.modrinth.as_ref().map(|m| m.project_id.as_str()).unwrap_or("")))
                        .map(|e| e.mod_id.clone())
                        .unwrap_or_default()
                } else {
                    meta.mod_id.clone()
                };
                if key.is_empty() { continue; }
                jar_ver_to_v.insert(meta.version.clone(), v.clone());
                candidates.entry(key).or_default().push(CandidateVersion {
                    jar_version: meta.version, deps: meta.dependencies, implanted: imp_cands,
                });
            }
            Err(_) => continue,
        }
    }
    Ok((candidates, jar_ver_to_v))
}

/// 检查所有已安装 modrinth mod 的可用更新。
pub async fn check_all_outdated(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
) -> Result<(Vec<OutdatedMod>, HashMap<String, crate::providers::ResolvedMod>), OrbitError> {
    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;
    let provider = &providers[0];

    let modrinth_entries: Vec<_> = lockfile.packages.iter()
        .filter(|e| e.modrinth.is_some())
        .collect();

    if modrinth_entries.is_empty() {
        eprintln!("  (no modrinth-sourced mods to check)");
        return Ok((vec![], HashMap::new()));
    }

    // 1. Find outdated mods
    let mut seeds: Vec<String> = Vec::new();
    for (i, entry) in modrinth_entries.iter().enumerate() {
        let mr = entry.modrinth.as_ref().unwrap();
        eprintln!("  [{}/{}] {} — checking versions...", i + 1, modrinth_entries.len(), entry.mod_id);
        let mut versions = match provider.get_versions(&mr.project_id, Some(mc_version), Some(loader)).await {
            Ok(v) => v,
            Err(e) => { eprintln!("    ! API error: {e}"); continue; }
        };
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
        let current_date = versions.iter()
            .find(|v| v.modrinth.as_ref().map(|m| m.version_id.as_str()) == Some(mr.version_id.as_str()))
            .map(|v| v.date_published.clone());
        let Some(ref cd) = current_date else {
            eprintln!("    ! current version not found in API results, skipping");
            continue;
        };
        let newer: Vec<_> = versions.iter().filter(|v| v.date_published > *cd).collect();
        if newer.is_empty() {
            eprintln!("    up to date (current: {})", mr.version);
        } else {
            eprintln!("    {} newer version(s) found", newer.len());
            seeds.push(mr.project_id.clone());
        }
    }

    if seeds.is_empty() {
        eprintln!("\n  all mods up to date.");
        return Ok((vec![], HashMap::new()));
    }

    // 2. BFS download
    let (mut candidates, jar_ver_to_v) = download_candidates_bfs(provider.as_ref(), &seeds, lockfile, mc_version, loader).await?;
    if candidates.is_empty() {
        return Ok((vec![], HashMap::new()));
    }

    // 3. Resolve
    eprintln!("\n  resolving dependency graph with {} candidate(s)...", candidates.len());
    let upgrades = crate::resolver::resolve_with_candidates(manifest, lockfile, &mut candidates, providers).await
        .map_err(|e| OrbitError::Other(anyhow::anyhow!("{e}")))?;

    let mut results: Vec<OutdatedMod> = upgrades.into_iter()
        .map(|(mod_id, new_version)| OutdatedMod {
            current_version: lockfile.find(&mod_id)
                .and_then(|e| e.modrinth.as_ref().map(|m| m.version.clone()))
                .unwrap_or_default(),
            new_version,
            mod_id,
        })
        .collect();
    results.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    Ok((results, jar_ver_to_v))
}
