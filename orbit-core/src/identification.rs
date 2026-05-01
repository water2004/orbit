//! 模组来源识别编排。

use crate::error::OrbitError;
use crate::init::ScannedMod;
use crate::providers::ModProvider;

#[derive(Debug, Clone)]
pub enum IdentifiedSource {
    Platform { platform: String, slug: String },
    File { path: String },
}

pub struct IdentifiedMod {
    pub filename: String,
    pub mod_id: String,
    pub mod_name: String,
    pub version: String,
    pub sha256: String,
    pub source: IdentifiedSource,
    /// 依赖列表: (mod_id, version_constraint, required)
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
    let mut results = vec![];

    for m in scanned {
        let (source, deps) = identify_one(m, providers, ctx).await;
        results.push(IdentifiedMod {
            filename: m.filename.clone(),
            mod_id: m.mod_id.clone().unwrap_or_default(),
            mod_name: m.mod_name.clone().unwrap_or_default(),
            version: m.version.clone().unwrap_or_default(),
            sha256: m.sha256.clone(),
            source,
            deps,
        });
    }

    Ok(results)
}

async fn identify_one(
    m: &ScannedMod,
    providers: &[Box<dyn ModProvider>],
    ctx: &IdentificationContext,
) -> (IdentifiedSource, Vec<(String, String, bool)>) {
    // Step 1: SHA-512 哈希反查
    for p in providers {
        match p.get_version_by_hash(&m.sha512).await {
            Ok(Some(resolved)) => {
                // 从 Version.dependencies 获取带 required 标记的依赖
                let deps: Vec<(String, String, bool)> = resolved.dependencies.iter().map(|d| {
                    (d.slug.clone().unwrap_or_else(|| d.name.clone()), String::new(), d.required)
                }).collect();
                eprintln!("    ✓ identified as {}/{} v{} (hash match, {} deps)", p.name(), resolved.mod_id, resolved.version, deps.len());
                return (
                    IdentifiedSource::Platform { platform: p.name().to_string(), slug: resolved.mod_id },
                    deps,
                );
            }
            _ => continue,
        }
    }

    // Step 2: slug + 版本交叉校验
    if let Some(ref mod_id) = m.mod_id {
        for p in providers {
            if let Ok(versions) = p.get_versions(mod_id, Some(&ctx.mc_version), Some(&ctx.loader)).await {
                let matched = m.version.as_ref().and_then(|ver| {
                    versions.iter().find(|v| v.version == *ver)
                });
                if let Some(v) = matched {
                    let deps: Vec<(String, String, bool)> = v.dependencies.iter().map(|d| {
                        (d.slug.clone().unwrap_or_else(|| d.name.clone()), String::new(), d.required)
                    }).collect();
                    eprintln!("    ✓ identified as {}/{} v{} (version match, {} deps)", p.name(), mod_id, v.version, deps.len());
                    return (
                        IdentifiedSource::Platform { platform: p.name().to_string(), slug: mod_id.clone() },
                        deps,
                    );
                }
            }
        }
    }

    // Step 3: File 兜底，依赖来自 JAR
    eprintln!("    ? unrecognized → recording as file ({} jar deps)", m.jar_deps.len());
    (IdentifiedSource::File { path: format!("mods/{}", m.filename) }, m.jar_deps.clone())
}
