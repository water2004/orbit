//! 过时检查编排层。
//!
//! 拉取版本列表 → 下载候选 JAR → 解析 fabric.mod.json → PubGrub 离线求解。

use std::collections::HashMap;

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;
use crate::manifest::OrbitManifest;
use crate::providers::ModProvider;

pub struct OutdatedMod {
    pub mod_id: String,
    pub current_version: String,
    pub new_version: String,
}

/// 检查所有已安装 modrinth mod 的可用更新。
pub async fn check_all_outdated(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
) -> Result<Vec<OutdatedMod>, OrbitError> {
    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;
    let provider = &providers[0];

    // 只处理有 modrinth 信息的条目
    let modrinth_entries: Vec<_> = lockfile.packages.iter()
        .filter(|e| e.modrinth.is_some())
        .collect();

    if modrinth_entries.is_empty() {
        eprintln!("  (no modrinth-sourced mods to check)");
        return Ok(vec![]);
    }

    let mut candidates: HashMap<String, Vec<(String, Vec<(String, String, bool)>)>> = HashMap::new();

    for (i, entry) in modrinth_entries.iter().enumerate() {
        let mr = entry.modrinth.as_ref().unwrap();

        eprintln!("  [{}/{}] {} — checking versions...", i + 1, modrinth_entries.len(), entry.mod_id);

        let mut versions = match provider.get_versions(&mr.project_id, Some(mc_version), Some(loader)).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("    ! API error: {e}");
                continue;
            }
        };
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));

        let current_date = versions.iter()
            .find(|v| v.modrinth.as_ref().map(|m| m.version_id.as_str()) == Some(mr.version_id.as_str()))
            .map(|v| v.date_published.clone());

        let Some(ref cd) = current_date else {
            eprintln!("    ! current version not found in API results, skipping");
            continue;
        };

        let newer: Vec<_> = versions.iter()
            .filter(|v| v.date_published > *cd)
            .collect();

        if newer.is_empty() {
            eprintln!("    up to date (current: {})", mr.version);
            continue;
        }

        eprintln!("    {} newer version(s) found, downloading JARs...", newer.len());

        let mut mod_candidates = Vec::new();
        for v in &newer {
            let ver_label = v.modrinth.as_ref().map(|m| m.version_number.as_str()).unwrap_or("?");
            match crate::jar::download_and_parse(&v.download_url, &v.sha512, loader).await {
                Ok(meta) => {
                    eprintln!("      {} → parsed (JAR version: {})", ver_label, meta.version);
                    mod_candidates.push((meta.version, meta.dependencies));
                }
                Err(e) => {
                    eprintln!("      {} → download/parse failed: {e}", ver_label);
                }
            }
        }
        if !mod_candidates.is_empty() {
            candidates.insert(entry.mod_id.clone(), mod_candidates);
        }
    }

    if candidates.is_empty() {
        eprintln!("\n  all mods up to date.");
        return Ok(vec![]);
    }

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
    Ok(results)
}
