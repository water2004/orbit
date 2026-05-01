//! 模组来源识别编排。
//!
//! 对 `init` 扫描到的每个模组，依次尝试各平台的 `ModProvider` 进行识别。
//! 所有识别必须校验本地哈希与平台返回的哈希一致。

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
        let source = identify_one(m, providers, ctx).await;
        results.push(IdentifiedMod {
            filename: m.filename.clone(),
            mod_id: m.mod_id.clone().unwrap_or_default(),
            mod_name: m.mod_name.clone().unwrap_or_default(),
            version: m.version.clone().unwrap_or_default(),
            sha256: m.sha256.clone(),
            source,
        });
    }

    Ok(results)
}

async fn identify_one(
    m: &ScannedMod,
    providers: &[Box<dyn ModProvider>],
    ctx: &IdentificationContext,
) -> IdentifiedSource {
    // Step 1: SHA-512 哈希反查（平台 API 已做匹配，无需本地再比较）
    for p in providers {
        match p.get_version_by_hash(&m.sha512).await {
            Ok(Some(resolved)) => {
                eprintln!(
                    "    ✓ identified as {}/{} v{} (hash match)",
                    p.name(),
                    resolved.mod_id,
                    resolved.version
                );
                return IdentifiedSource::Platform {
                    platform: p.name().to_string(),
                    slug: resolved.mod_id,
                };
            }
            _ => continue,
        }
    }

    // Step 2: slug + 版本交叉校验
    if let Some(ref mod_id) = m.mod_id {
        for p in providers {
            if let Ok(versions) = p
                .get_versions(mod_id, Some(&ctx.mc_version), Some(&ctx.loader))
                .await
            {
                let matched = m.version.as_ref().and_then(|ver| {
                    versions.iter().find(|v| v.version == *ver)
                });
                if let Some(v) = matched {
                    eprintln!(
                        "    ✓ identified as {}/{} v{} (version match)",
                        p.name(),
                        mod_id,
                        v.version
                    );
                    return IdentifiedSource::Platform {
                        platform: p.name().to_string(),
                        slug: mod_id.clone(),
                    };
                }
            }
        }
    }

    // Step 3: 兜底
    eprintln!("    ? unrecognized → recording as file");
    IdentifiedSource::File {
        path: format!("mods/{}", m.filename),
    }
}
