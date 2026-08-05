//! 模组来源识别编排（批量 API 避免 N+1）。

use crate::error::OrbitError;
use crate::init::ScannedMod;
use crate::lockfile::{ArtifactSource, PackageEntry};
use crate::manifest::PackageRemote;
use crate::providers::{ArtifactFingerprint, ModProvider};

#[derive(Debug, Clone)]
pub struct IdentifiedMod {
    pub filename: String,
    pub enabled: bool,
    /// JAR loader 元数据声明的模组 ID
    pub mod_id: String,
    pub mod_name: String,
    /// JAR loader 元数据声明的版本
    pub version: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    pub remotes: Vec<PackageRemote>,
    pub artifact_sources: Vec<ArtifactSource>,
    pub dependencies: Vec<crate::metadata::DependencyExpression>,
    pub environment: crate::metadata::Environment,
    pub provides: Vec<crate::metadata::ProvidedMod>,
    pub language_loader: Option<crate::metadata::LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<crate::metadata::EmbeddedArtifact>,
    pub bundled: Vec<crate::lockfile::BundledMod>,
}

impl IdentifiedMod {
    pub(crate) fn package_id(&self) -> String {
        debug_assert!(!self.mod_id.is_empty());
        self.mod_id.clone()
    }

    pub(crate) fn to_package_entry(&self) -> PackageEntry {
        PackageEntry {
            mod_id: self.package_id(),
            version: self.version.clone(),
            sha1: self.sha1.clone(),
            sha256: self.sha256.clone(),
            sha512: self.sha512.clone(),
            filename: self.filename.clone(),
            remotes: self.remotes.clone(),
            artifact_sources: self.artifact_sources.clone(),
            dependencies: self.dependencies.clone(),
            environment: self.environment,
            provides: self.provides.clone(),
            language_loader: self.language_loader.clone(),
            embedded_artifacts: self.embedded_artifacts.clone(),
            bundled: self.bundled.clone(),
        }
    }
}

fn identified_sources(
    m: &ScannedMod,
    artifact: &crate::providers::RemoteArtifact,
) -> Option<(PackageRemote, ArtifactSource)> {
    if !artifact.matches_artifact(&ArtifactFingerprint {
        sha1: m.sha1.clone(),
        sha512: m.sha512.clone(),
        curseforge: m.curseforge_fingerprint,
    }) {
        return None;
    }
    Some((
        artifact.package_remote().ok()?,
        artifact.artifact_source().ok()?,
    ))
}

fn unidentified_local(m: &ScannedMod) -> IdentifiedMod {
    let path = format!("mods/{}", m.filename);
    IdentifiedMod {
        filename: m.filename.clone(),
        enabled: m.enabled,
        mod_id: m.mod_id.clone().unwrap_or_default(),
        mod_name: m.mod_name.clone().unwrap_or_default(),
        version: m.version.clone().unwrap_or_default(),
        sha1: m.sha1.clone(),
        sha256: m.sha256.clone(),
        sha512: m.sha512.clone(),
        remotes: vec![PackageRemote::File { path: path.clone() }],
        artifact_sources: vec![ArtifactSource::File { path }],
        dependencies: m.dependencies.clone(),
        environment: m.environment,
        provides: m.provides.clone(),
        language_loader: m.language_loader.clone(),
        embedded_artifacts: m.embedded_artifacts.clone(),
        bundled: m.bundled.clone(),
    }
}

pub async fn identify_mods(
    scanned: &[ScannedMod],
    providers: &[Box<dyn ModProvider>],
) -> Result<Vec<IdentifiedMod>, OrbitError> {
    let mut results: Vec<IdentifiedMod> = scanned.iter().map(unidentified_local).collect();
    let mut identified = vec![false; scanned.len()];

    for p in providers {
        let fingerprints: Vec<ArtifactFingerprint> = scanned
            .iter()
            .map(|artifact| ArtifactFingerprint {
                sha1: artifact.sha1.clone(),
                sha512: artifact.sha512.clone(),
                curseforge: artifact.curseforge_fingerprint,
            })
            .collect();
        let found = p.identify_artifacts(&fingerprints).await.map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "failed to identify local artifacts with {}: {error}",
                p.name()
            ))
        })?;
        for (index, scanned) in scanned.iter().enumerate() {
            for artifact in &found {
                let Some((remote, source)) = identified_sources(scanned, artifact) else {
                    continue;
                };
                if !identified[index] {
                    results[index].remotes.clear();
                    results[index].artifact_sources.clear();
                    identified[index] = true;
                }
                if !results[index].remotes.contains(&remote) {
                    results[index].remotes.push(remote);
                }
                if !results[index].artifact_sources.contains(&source) {
                    results[index].artifact_sources.push(source);
                }
            }
        }
    }

    for result in &mut results {
        result.remotes.sort();
        result
            .artifact_sources
            .sort_by_key(|source| format!("{source:?}"));
    }
    Ok(results)
}

pub(crate) fn preserve_local_sources(
    instance_dir: &std::path::Path,
    identified: &mut [IdentifiedMod],
) -> Result<(), OrbitError> {
    for package in identified {
        for remote in &mut package.remotes {
            let PackageRemote::File { path } = remote else {
                continue;
            };
            let source = {
                let path = std::path::Path::new(path);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    instance_dir.join(path)
                }
            };
            *remote = crate::source_store::preserve_if_instance_output(
                instance_dir,
                &source,
                &package.sha512,
            )?;
        }
        for source in &mut package.artifact_sources {
            let ArtifactSource::File { path } = source else {
                continue;
            };
            let file = {
                let path = std::path::Path::new(path);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    instance_dir.join(path)
                }
            };
            let remote = crate::source_store::preserve_if_instance_output(
                instance_dir,
                &file,
                &package.sha512,
            )?;
            *source = crate::source_store::managed_artifact_source(&remote);
        }
        package.remotes.sort();
        package.remotes.dedup();
        package
            .artifact_sources
            .sort_by_key(|source| format!("{source:?}"));
        package.artifact_sources.dedup();
    }
    Ok(())
}
