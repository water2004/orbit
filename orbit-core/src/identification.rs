//! 模组来源识别编排。
//!
//! 对 `init` 扫描到的每个模组，依次尝试各平台的 `ModProvider` 进行识别。
//! 不硬编码任何平台名，完全通过 trait 驱动。

use crate::error::OrbitError;
use crate::init::ScannedMod;
use crate::providers::ModProvider;

/// 识别后的来源
#[derive(Debug, Clone)]
pub enum IdentifiedSource {
    /// 平台模组 — 已通过 API 验证
    Platform { platform: String, slug: String },
    /// 未识别 — 以 file 类型记录
    File { path: String },
}

/// 识别后的模组
pub struct IdentifiedMod {
    pub filename: String,
    pub mod_id: String,
    pub mod_name: String,
    pub version: String,
    pub sha256: String,
    pub source: IdentifiedSource,
}

/// 识别所需的实例上下文
pub struct IdentificationContext {
    pub mc_version: String,
    pub loader: String,
}

/// 对扫到的模组逐个识别来源。
///
/// `providers` 的顺序决定优先级——先试 Modrinth，再 CurseForge。
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
    // Step 1: SHA-256 哈希反查（最高置信度）
    for p in providers {
        match p.get_version_by_hash(&m.sha256).await {
            Ok(Some(_resolved)) => {
                return IdentifiedSource::Platform {
                    platform: p.name().to_string(),
                    slug: m.mod_id.clone().unwrap_or_else(|| m.filename.clone()),
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
                let matched = m.version.as_ref().map_or(false, |ver| {
                    versions.iter().any(|v| v.version == *ver)
                });
                if matched {
                    return IdentifiedSource::Platform {
                        platform: p.name().to_string(),
                        slug: mod_id.clone(),
                    };
                }
            }
        }
    }

    // Step 3: 兜底
    IdentifiedSource::File {
        path: format!("mods/{}", m.filename),
    }
}
