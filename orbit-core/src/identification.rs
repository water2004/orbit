//! 模组来源识别编排（批量 API 避免 N+1）。

use crate::error::OrbitError;
use crate::init::ScannedMod;
use crate::lockfile::{CurseForgeInfo, ModrinthInfo};
use crate::providers::{ArtifactFingerprint, ModProvider};

#[derive(Debug, Clone)]
pub enum IdentifiedSource {
    Platform(IdentifiedPlatform),
    File { path: String },
}

#[derive(Debug, Clone)]
pub enum IdentifiedPlatform {
    Modrinth(ModrinthInfo),
    CurseForge(CurseForgeInfo),
}

impl IdentifiedPlatform {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Modrinth(_) => "modrinth",
            Self::CurseForge(_) => "curseforge",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IdentifiedMod {
    pub filename: String,
    /// JAR loader 元数据声明的模组 ID
    pub mod_id: String,
    pub mod_name: String,
    /// JAR loader 元数据声明的版本
    pub version: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    pub source: IdentifiedSource,
    pub dependencies: Vec<crate::metadata::DependencyExpression>,
    pub environment: crate::metadata::Environment,
    pub provides: Vec<crate::metadata::ProvidedMod>,
    pub language_loader: Option<crate::metadata::LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<crate::metadata::EmbeddedArtifact>,
    pub bundled: Vec<crate::lockfile::BundledMod>,
}

fn build_identified(
    m: &ScannedMod,
    artifact: &crate::providers::RemoteArtifact,
) -> Option<IdentifiedMod> {
    let slug = artifact.slug.clone();
    let source = if let Some(metadata) = &artifact.modrinth {
        IdentifiedPlatform::Modrinth(ModrinthInfo {
            project_id: metadata.project_id.clone(),
            version_id: metadata.version_id.clone(),
            slug,
            download_url: artifact.download_url.clone(),
        })
    } else if let Some(metadata) = &artifact.curseforge {
        IdentifiedPlatform::CurseForge(CurseForgeInfo {
            project_id: metadata.project_id,
            file_id: metadata.file_id,
            slug,
            download_url: artifact.download_url.clone(),
        })
    } else {
        return None;
    };
    Some(IdentifiedMod {
        filename: m.filename.clone(),
        mod_id: m.mod_id.clone().unwrap_or_default(),
        mod_name: m.mod_name.clone().unwrap_or_default(),
        version: m.version.clone().unwrap_or_default(),
        sha1: m.sha1.clone(),
        sha256: m.sha256.clone(),
        sha512: m.sha512.clone(),
        source: IdentifiedSource::Platform(source),
        dependencies: m.dependencies.clone(),
        environment: m.environment,
        provides: m.provides.clone(),
        language_loader: m.language_loader.clone(),
        embedded_artifacts: m.embedded_artifacts.clone(),
        bundled: m.bundled.clone(),
    })
}

pub async fn identify_mods(
    scanned: &[ScannedMod],
    providers: &[Box<dyn ModProvider>],
) -> Result<Vec<IdentifiedMod>, OrbitError> {
    let mut results: Vec<Option<IdentifiedMod>> = scanned.iter().map(|_| None).collect();
    let mut unrecognized: Vec<usize> = (0..scanned.len()).collect();

    for p in providers {
        if unrecognized.is_empty() {
            break;
        }

        let artifacts: Vec<ArtifactFingerprint> = unrecognized
            .iter()
            .map(|&i| ArtifactFingerprint {
                sha1: scanned[i].sha1.clone(),
                sha512: scanned[i].sha512.clone(),
                curseforge: scanned[i].curseforge_fingerprint,
            })
            .collect();
        let found = p.identify_artifacts(&artifacts).await.map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "failed to identify local artifacts with {}: {error}",
                p.name()
            ))
        })?;
        let mut still_unrecognized = Vec::new();
        for &idx in &unrecognized {
            let m = &scanned[idx];
            let artifact = ArtifactFingerprint {
                sha1: m.sha1.clone(),
                sha512: m.sha512.clone(),
                curseforge: m.curseforge_fingerprint,
            };
            if let Some(resolved) = found
                .iter()
                .find(|resolved| resolved.matches_artifact(&artifact))
                && let Some(identified) = build_identified(m, resolved)
            {
                results[idx] = Some(identified);
            } else {
                still_unrecognized.push(idx);
            }
        }
        unrecognized = still_unrecognized;
    }

    let mut final_results = Vec::new();
    for (i, m) in scanned.iter().enumerate() {
        if let Some(ident) = results[i].take() {
            final_results.push(ident);
        } else {
            final_results.push(IdentifiedMod {
                filename: m.filename.clone(),
                mod_id: m.mod_id.clone().unwrap_or_default(),
                mod_name: m.mod_name.clone().unwrap_or_default(),
                version: m.version.clone().unwrap_or_default(),
                sha1: m.sha1.clone(),
                sha256: m.sha256.clone(),
                sha512: m.sha512.clone(),
                source: IdentifiedSource::File {
                    path: format!("mods/{}", m.filename),
                },
                dependencies: m.dependencies.clone(),
                environment: m.environment,
                provides: m.provides.clone(),
                language_loader: m.language_loader.clone(),
                embedded_artifacts: m.embedded_artifacts.clone(),
                bundled: m.bundled.clone(),
            });
        }
    }
    Ok(final_results)
}
