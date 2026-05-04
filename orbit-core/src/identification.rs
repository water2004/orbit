//! 模组来源识别编排（批量 API 避免 N+1）。

use std::collections::HashMap;
use crate::error::OrbitError;
use crate::init::ScannedMod;
use crate::providers::ModProvider;

#[derive(Debug, Clone)]
pub enum IdentifiedSource {
    Platform { platform: String, project_id: String, version_id: String, slug: String },
    File { path: String },
}

#[derive(Debug, Clone)]
pub struct IdentifiedMod {
    pub filename: String,
    /// fabric.mod.json 的 `id`
    pub mod_id: String,
    pub mod_name: String,
    /// fabric.mod.json 的 `version`
    pub version: String,
    /// Modrinth version_number
    pub modrinth_version: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    pub source: IdentifiedSource,
    pub deps: Vec<(String, String, bool)>,
}

pub struct IdentificationContext {
    pub mc_version: String,
    pub loader: String,
}

fn build_identified(m: &ScannedMod, platform: &str, resolved: &crate::providers::ResolvedMod, version_match: bool) -> IdentifiedMod {
    let slug = m.mod_id.clone().unwrap_or_else(|| resolved.mod_id.clone());
    let jar_ver = m.version.clone().unwrap_or_default();
    IdentifiedMod {
        filename: m.filename.clone(),
        mod_id: m.mod_id.clone().unwrap_or_default(),
        mod_name: m.mod_name.clone().unwrap_or_default(),
        version: if jar_ver.is_empty() { resolved.version.clone() } else { jar_ver },
        modrinth_version: resolved.modrinth.as_ref().map(|m| m.version_number.clone()).unwrap_or_default(),
        sha1: m.sha1.clone(),
        sha256: m.sha256.clone(),
        sha512: m.sha512.clone(),
        source: IdentifiedSource::Platform {
            platform: platform.to_string(),
            project_id: resolved.modrinth.as_ref().map(|m| m.project_id.clone()).unwrap_or_default(),
            version_id: resolved.modrinth.as_ref().map(|m| m.version_id.clone()).unwrap_or_default(),
            slug,
        },
        deps: m.jar_deps.clone(),
    }
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

        let hashes: Vec<String> = unrecognized.iter().map(|&i| scanned[i].sha512.clone()).collect();
        if let Ok(found) = p.get_versions_by_hashes(&hashes).await {
            let hash_to_mod: HashMap<&str, &crate::providers::ResolvedMod> = found.iter()
                .map(|m| (m.sha512.as_str(), m)).collect();
            let mut still_unrecognized = Vec::new();
            for &idx in &unrecognized {
                let m = &scanned[idx];
                if let Some(resolved) = hash_to_mod.get(m.sha512.as_str()) {
                    eprintln!("    ✓ identified as {}/{} v{} (hash match)", p.name(), m.mod_id.as_deref().unwrap_or("?"), resolved.version);
                    results[idx] = Some(build_identified(m, p.name(), resolved, false));
                } else {
                    still_unrecognized.push(idx);
                }
            }
            unrecognized = still_unrecognized;
            continue;
        }

        let mut still_unrecognized = Vec::new();
        for &idx in &unrecognized {
            let m = &scanned[idx];
            match p.get_version_by_hash(&m.sha512).await {
                Ok(Some(resolved)) => {
                    eprintln!("    ✓ identified as {}/{} v{} (hash match)", p.name(), m.mod_id.as_deref().unwrap_or("?"), resolved.version);
                    results[idx] = Some(build_identified(m, p.name(), &resolved, false));
                }
                _ => {
                    if let Some(ref mod_id) = m.mod_id {
                        if let Ok(versions) = p.get_versions(mod_id, Some(&ctx.mc_version), Some(&ctx.loader)).await {
                            let matched = m.version.as_ref().and_then(|ver| versions.iter().find(|v| v.version == *ver));
                            if let Some(v) = matched {
                                eprintln!("    ✓ identified as {}/{} v{} (version match)", p.name(), mod_id, v.version);
                                results[idx] = Some(build_identified(m, p.name(), v, true));
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

    let mut final_results = Vec::new();
    for (i, m) in scanned.iter().enumerate() {
        if let Some(ident) = results[i].take() {
            final_results.push(ident);
        } else {
            eprintln!("    ? unrecognized → recording as file ({} jar deps)", m.jar_deps.len());
            final_results.push(IdentifiedMod {
                filename: m.filename.clone(),
                mod_id: m.mod_id.clone().unwrap_or_default(),
                mod_name: m.mod_name.clone().unwrap_or_default(),
                version: m.version.clone().unwrap_or_default(),
                modrinth_version: String::new(),
                sha1: m.sha1.clone(),
                sha256: m.sha256.clone(),
                sha512: m.sha512.clone(),
                source: IdentifiedSource::File { path: format!("mods/{}", m.filename) },
                deps: m.jar_deps.clone(),
            });
        }
    }
    Ok(final_results)
}
