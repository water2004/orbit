//! 模组来源识别编排。
//!
//! 使用批量 API 避免 N+1 查询。

use std::collections::HashMap;
use crate::error::OrbitError;
use crate::init::ScannedMod;
use crate::providers::ModProvider;

#[derive(Debug, Clone)]
pub enum IdentifiedSource {
    Platform { platform: String, project_id: String, slug: String },
    File { path: String },
}

#[derive(Debug, Clone)]
pub struct IdentifiedMod {
    pub filename: String,
    pub mod_id: String,
    pub mod_name: String,
    /// Modrinth 等平台的发布版本名（如 fabric-26.1-6.7.1），用于 lock/install
    pub version: String,
    /// 来自 fabric.mod.json 的自声明版本（如 6.7.1），用于本地依赖校验
    pub local_version: String,
    pub sha256: String,
    pub source: IdentifiedSource,
    pub deps: Vec<(String, String, bool)>,
}

pub struct IdentificationContext {
    pub mc_version: String,
    pub loader: String,
}

pub async fn identify_mods(
    scanned: &[ScannedMod],
    providers: &[Box<dyn ModProvider>],
    ctx: &IdentificationContext,
) -> Result<Vec<IdentifiedMod>, OrbitError> {
    let mut results: Vec<Option<IdentifiedMod>> = scanned.iter().map(|_| None).collect();
    let mut unrecognized: Vec<usize> = (0..scanned.len()).collect();

    for p in providers {
        if unrecognized.is_empty() { break; }

        // ── 批量哈希反查 ──
        let hashes: Vec<String> = unrecognized.iter()
            .map(|&i| scanned[i].sha512.clone())
            .collect();

        if let Ok(found) = p.get_versions_by_hashes(&hashes).await {
            let hash_to_mod: HashMap<&str, &crate::providers::ResolvedMod> = found.iter()
                .map(|m| (m.sha512.as_str(), m))
                .collect();

            let mut still_unrecognized = Vec::new();
            for &idx in &unrecognized {
                let m = &scanned[idx];
                if let Some(resolved) = hash_to_mod.get(m.sha512.as_str()) {
                    // 优先使用 JAR 的 mod_id 作为 slug（与 Modrinth slug 一致），
                    // 避免用 project_id（如 "EsAfCjCV"）作为 slug。
                    let slug = m.mod_id.clone().unwrap_or_else(|| resolved.mod_id.clone());
                    let deps = m.jar_deps.clone();
                    eprintln!("    ✓ identified as {}/{} v{} (hash match, {} deps)", p.name(), slug, resolved.version, deps.len());
                    results[idx] = Some(IdentifiedMod {
                        filename: m.filename.clone(),
                        mod_id: m.mod_id.clone().unwrap_or_default(),
                        mod_name: m.mod_name.clone().unwrap_or_default(),
                        version: resolved.version.clone(),
                        local_version: m.version.clone().unwrap_or_else(|| resolved.version.clone()),
                        sha256: m.sha256.clone(),
                        source: IdentifiedSource::Platform { platform: p.name().to_string(), project_id: resolved.mod_id.clone(), slug },
                        deps,
                    });
                } else {
                    still_unrecognized.push(idx);
                }
            }
            unrecognized = still_unrecognized;
            continue;
        }

        // 批量失败 → 逐个回退
        let mut still_unrecognized = Vec::new();
        for &idx in &unrecognized {
            let m = &scanned[idx];
            match p.get_version_by_hash(&m.sha512).await {
                Ok(Some(resolved)) => {
                    let slug = m.mod_id.clone().unwrap_or_else(|| resolved.mod_id.clone());
                    eprintln!("    ✓ identified as {}/{} v{} (hash match)", p.name(), slug, resolved.version);
                    results[idx] = Some(IdentifiedMod {
                        filename: m.filename.clone(), mod_id: m.mod_id.clone().unwrap_or_default(),
                        mod_name: m.mod_name.clone().unwrap_or_default(), version: resolved.version.clone(),
                        local_version: m.version.clone().unwrap_or_else(|| resolved.version.clone()),
                        sha256: m.sha256.clone(),
                        source: IdentifiedSource::Platform { platform: p.name().to_string(), project_id: resolved.mod_id.clone(), slug },
                        deps: m.jar_deps.clone(),
                    });
                }
                _ => {
                    // slug + 版本交叉校验
                    if let Some(ref mod_id) = m.mod_id {
                        if let Ok(versions) = p.get_versions(mod_id, Some(&ctx.mc_version), Some(&ctx.loader)).await {
                            let matched = m.version.as_ref().and_then(|ver| versions.iter().find(|v| v.version == *ver));
                            if let Some(v) = matched {
                                eprintln!("    ✓ identified as {}/{} v{} (version match)", p.name(), mod_id, v.version);
                                results[idx] = Some(IdentifiedMod {
                                    filename: m.filename.clone(), mod_id: mod_id.clone(),
                                    mod_name: m.mod_name.clone().unwrap_or_default(), version: v.version.clone(),
                                    local_version: m.version.clone().unwrap_or_else(|| v.version.clone()),
                                    sha256: m.sha256.clone(),
                                    source: IdentifiedSource::Platform { platform: p.name().to_string(), project_id: v.mod_id.clone(), slug: mod_id.clone() },
                                    deps: m.jar_deps.clone(),
                                });
                                continue;
                            }
                        }
                    }
                    still_unrecognized.push(idx);
                }
            }
        }
        unrecognized = still_unrecognized;
    }

    // 兜底：file 类型
    let mut final_results = Vec::new();
    for (i, m) in scanned.iter().enumerate() {
        if let Some(ident) = results[i].take() {
            final_results.push(ident);
        } else {
            eprintln!("    ? unrecognized → recording as file ({} jar deps)", m.jar_deps.len());
            final_results.push(IdentifiedMod {
                filename: m.filename.clone(), mod_id: m.mod_id.clone().unwrap_or_default(),
                mod_name: m.mod_name.clone().unwrap_or_default(),
                version: m.version.clone().unwrap_or_default(),
                local_version: m.version.clone().unwrap_or_default(),
                sha256: m.sha256.clone(),
                source: IdentifiedSource::File { path: format!("mods/{}", m.filename) },
                deps: m.jar_deps.clone(),
            });
        }
    }
    Ok(final_results)
}
