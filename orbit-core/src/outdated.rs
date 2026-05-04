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

    // mod_id → [(jar_version, deps)]，从新到旧
    let mut candidates: HashMap<String, Vec<(String, Vec<(String, String, bool)>)>> = HashMap::new();

    for entry in &lockfile.packages {
        let Some(mr) = &entry.modrinth else { continue; };

        let mut versions = match provider.get_versions(&mr.slug, Some(mc_version), Some(loader)).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));

        let current_date = versions.iter()
            .find(|v| v.modrinth.as_ref().map(|m| m.version_id.as_str()) == Some(mr.version_id.as_str()))
            .map(|v| v.date_published.clone());

        let Some(ref cd) = current_date else { continue; };

        // 收集所有更新的版本
        let newer: Vec<_> = versions.iter()
            .filter(|v| v.date_published > *cd)
            .collect();

        if newer.is_empty() { continue; }

        let mut mod_candidates = Vec::new();
        for v in &newer {
            match crate::jar::download_and_parse(&v.download_url, &v.sha512, loader).await {
                Ok(meta) => {
                    mod_candidates.push((meta.version, meta.dependencies));
                }
                Err(_) => continue,
            }
        }
        if !mod_candidates.is_empty() {
            candidates.insert(entry.mod_id.clone(), mod_candidates);
        }
    }

    if candidates.is_empty() {
        return Ok(vec![]);
    }

    // PubGrub 离线求解
    let upgrades = crate::resolver::resolve_with_candidates(manifest, lockfile, &candidates)
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
