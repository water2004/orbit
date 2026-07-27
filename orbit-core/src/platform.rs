//! Strict consumption of the platform snapshot recorded in `orbit.toml`.
//!
//! Discovery belongs to `platform_detection` and is only invoked by `init` and
//! `sync`. This module resolves exactly the recorded files, validates their
//! hashes and metadata, and fails when the snapshot no longer matches disk.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::loader::LoaderKind;
use crate::manifest::{OrbitManifest, PlatformArtifact};
use crate::metadata::mojang::McVersion;
use crate::resolver::types::PlatformCandidate;

#[derive(Debug, Clone)]
pub(crate) struct Platform {
    pub minecraft_version: McVersion,
    pub minecraft_jar: PathBuf,
    pub loader: LoaderKind,
    pub loader_version: String,
    pub loader_jar: PathBuf,
    pub runtime_jars: Vec<PathBuf>,
    pub loader_package: Option<PlatformCandidate>,
    pub physical_environment: crate::metadata::Environment,
}

impl Platform {
    /// Loads the exact platform snapshot. This function never searches for a
    /// replacement artifact and never mutates the manifest.
    pub(crate) fn load(instance_dir: &Path, manifest: &OrbitManifest) -> Result<Self, OrbitError> {
        let minecraft_jar = resolve_artifact(
            instance_dir,
            "platform.minecraft_jar",
            &manifest.platform.minecraft_jar,
        )?;
        let loader_jar = resolve_artifact(
            instance_dir,
            "platform.loader_jar",
            &manifest.platform.loader_jar,
        )?;
        if minecraft_jar == loader_jar
            || manifest
                .platform
                .minecraft_jar
                .sha256
                .eq_ignore_ascii_case(&manifest.platform.loader_jar.sha256)
        {
            return Err(snapshot_error(
                "platform.minecraft_jar and platform.loader_jar are not distinct artifacts",
            ));
        }

        let mut runtime_jars = Vec::with_capacity(manifest.platform.runtime_jars.len());
        let mut runtime_paths = BTreeSet::new();
        let mut runtime_hashes = BTreeSet::new();
        for (index, artifact) in manifest.platform.runtime_jars.iter().enumerate() {
            let field = format!("platform.runtime_jars[{index}]");
            if artifact
                .sha256
                .eq_ignore_ascii_case(&manifest.platform.minecraft_jar.sha256)
                || artifact
                    .sha256
                    .eq_ignore_ascii_case(&manifest.platform.loader_jar.sha256)
            {
                return Err(snapshot_error(format!(
                    "{field} duplicates content from a primary platform artifact"
                )));
            }
            let path = resolve_artifact(instance_dir, &field, artifact)?;
            if path == minecraft_jar || path == loader_jar {
                return Err(snapshot_error(format!(
                    "{field} duplicates a primary platform artifact: '{}'",
                    path.display()
                )));
            }
            if !runtime_paths.insert(path.clone()) {
                return Err(snapshot_error(format!(
                    "{field} duplicates runtime path '{}'",
                    path.display()
                )));
            }
            if !runtime_hashes.insert(artifact.sha256.to_ascii_lowercase()) {
                return Err(snapshot_error(format!(
                    "{field} duplicates content already present in platform.runtime_jars"
                )));
            }
            runtime_jars.push(path);
        }

        let minecraft_version =
            crate::jar::read_minecraft_version(&minecraft_jar).map_err(|error| {
                snapshot_error(format!(
                    "platform.minecraft_jar '{}' is not a readable Minecraft JAR: {error}",
                    minecraft_jar.display()
                ))
            })?;
        if minecraft_version.id != manifest.project.mc_version {
            return Err(snapshot_error(format!(
                "platform.minecraft_jar '{}' declares Minecraft '{}', but project.mc_version is '{}'",
                minecraft_jar.display(),
                minecraft_version.id,
                manifest.project.mc_version
            )));
        }

        let loader = manifest
            .project
            .modloader
            .parse::<LoaderKind>()
            .map_err(snapshot_error)?;
        let loader_version = manifest.project.modloader_version.clone();
        let loader_package = load_loader_package(&loader_jar, loader, &loader_version)?;

        Ok(Self {
            minecraft_version,
            minecraft_jar,
            loader,
            loader_version,
            loader_jar,
            runtime_jars,
            loader_package,
            physical_environment: manifest.platform.physical_environment,
        })
    }
}

fn resolve_artifact(
    instance_dir: &Path,
    field: &str,
    artifact: &PlatformArtifact,
) -> Result<PathBuf, OrbitError> {
    if artifact.path.trim().is_empty() {
        return Err(snapshot_error(format!("{field}.path is empty")));
    }
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(snapshot_error(format!(
            "{field}.sha256 is not a 64-digit hexadecimal SHA-256"
        )));
    }

    let configured = Path::new(&artifact.path);
    let path = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        instance_dir.join(configured)
    };
    let path = path.canonicalize().map_err(|error| {
        snapshot_error(format!(
            "{field}.path '{}' cannot be resolved: {error}",
            artifact.path
        ))
    })?;
    if !path.is_file() {
        return Err(snapshot_error(format!(
            "{field}.path '{}' is not a file",
            path.display()
        )));
    }

    let actual = crate::jar::compute_sha256(&path).map_err(|error| {
        snapshot_error(format!(
            "cannot verify {field} at '{}': {error}",
            path.display()
        ))
    })?;
    if !actual.eq_ignore_ascii_case(&artifact.sha256) {
        return Err(snapshot_error(format!(
            "{field} content does not match the snapshot for '{}'",
            path.display()
        )));
    }
    Ok(path)
}

fn load_loader_package(
    loader_jar: &Path,
    loader: LoaderKind,
    loader_version: &str,
) -> Result<Option<PlatformCandidate>, OrbitError> {
    match crate::jar::read_mod_metadata_if_present(loader_jar, loader.as_str()) {
        Ok(Some(metadata)) => {
            let expected_mod_id = loader.semantics().canonical_package;
            if metadata.mod_id != expected_mod_id {
                return Err(snapshot_error(format!(
                    "platform.loader_jar '{}' declares mod_id '{}', expected '{}'",
                    loader_jar.display(),
                    metadata.mod_id,
                    expected_mod_id
                )));
            }
            if crate::versions::Version::parse(&metadata.version, loader.as_str())
                != crate::versions::Version::parse(loader_version, loader.as_str())
            {
                return Err(snapshot_error(format!(
                    "platform.loader_jar '{}' declares version '{}', but project.modloader_version is '{}'",
                    loader_jar.display(),
                    metadata.version,
                    loader_version
                )));
            }
            Ok(Some(PlatformCandidate::from_jar_metadata(metadata)))
        }
        Ok(None) if matches!(loader, LoaderKind::Fabric | LoaderKind::Quilt) => {
            Err(snapshot_error(format!(
                "platform.loader_jar '{}' contains no {loader} loader metadata",
                loader_jar.display()
            )))
        }
        Ok(None) if matches!(loader, LoaderKind::Forge | LoaderKind::NeoForge) => Ok(None),
        Ok(None) => unreachable!(),
        Err(error) => Err(snapshot_error(format!(
            "cannot parse platform.loader_jar '{}': {error}",
            loader_jar.display()
        ))),
    }
}

fn snapshot_error(message: impl std::fmt::Display) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(
        "invalid platform snapshot in orbit.toml: {message}; run 'orbit sync' to rediscover the instance"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{OrbitManifest, ProjectMeta, ResolverConfig};

    fn manifest(instance_dir: &Path) -> OrbitManifest {
        let discovered = crate::platform_detection::discover_platform_for_init(
            instance_dir,
            "1.21.1",
            "fabric",
            "0.16.10",
        )
        .unwrap();
        OrbitManifest {
            project: ProjectMeta {
                name: "test".to_string(),
                mc_version: discovered.minecraft_version.id.clone(),
                modloader: discovered.loader.to_string(),
                modloader_version: discovered.loader_version.clone(),
                description: None,
                authors: None,
                version: None,
            },
            platform: discovered.snapshot(instance_dir).unwrap(),
            resolver: ResolverConfig::default(),
            dependencies: Default::default(),
            groups: Default::default(),
            overrides: Default::default(),
        }
    }

    #[test]
    fn loads_only_the_exact_recorded_platform_files() {
        let directory = tempfile::tempdir().unwrap();
        crate::platform_detection::test_support::write_platform(
            directory.path(),
            "1.21.1",
            "fabric",
            "0.16.10",
        );
        let manifest = manifest(directory.path());
        let recorded = directory.path().join(&manifest.platform.minecraft_jar.path);
        let replacement = directory.path().join("renamed-client.jar");
        std::fs::rename(&recorded, &replacement).unwrap();

        let error = Platform::load(directory.path(), &manifest)
            .unwrap_err()
            .to_string();

        assert!(error.contains("platform.minecraft_jar.path"));
        assert!(error.contains("run 'orbit sync'"));
        assert!(!error.contains(replacement.to_string_lossy().as_ref()));
    }

    #[test]
    fn rejects_changed_content_at_the_recorded_path() {
        let directory = tempfile::tempdir().unwrap();
        crate::platform_detection::test_support::write_platform(
            directory.path(),
            "1.21.1",
            "fabric",
            "0.16.10",
        );
        let manifest = manifest(directory.path());
        let loader = directory.path().join(&manifest.platform.loader_jar.path);
        std::fs::write(&loader, b"different bytes").unwrap();

        let error = Platform::load(directory.path(), &manifest)
            .unwrap_err()
            .to_string();

        assert!(error.contains("platform.loader_jar content does not match"));
        assert!(error.contains("run 'orbit sync'"));
    }
}
