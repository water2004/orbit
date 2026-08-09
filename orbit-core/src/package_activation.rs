//! Loader activation state for managed top-level package JARs.
//!
//! Disabled packages remain part of the manifest and exact lock. Their
//! top-level carrier is renamed from `<name>.jar` to `<name>.jar.disabled`, a
//! name that the supported Loaders do not discover as a mod JAR.

use std::path::Path;

use crate::error::OrbitError;
use crate::workspace::{Lockfile, ManifestFile};

const DISABLED_SUFFIX: &str = ".disabled";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageActivationReport {
    pub package: String,
    pub previous_enabled: bool,
    pub enabled: bool,
    pub changed: bool,
    pub dry_run: bool,
}

/// Return the Loader activation state represented by a managed filename.
pub(crate) fn mod_artifact_enabled(filename: &str) -> Option<bool> {
    if filename.ends_with(".jar") {
        Some(true)
    } else if filename.ends_with(".jar.disabled") {
        Some(false)
    } else {
        None
    }
}

/// Produce the canonical physical filename for a package activation state.
pub(crate) fn filename_for_activation(filename: &str, enabled: bool) -> Result<String, OrbitError> {
    let jar_filename = filename.strip_suffix(DISABLED_SUFFIX).unwrap_or(filename);
    if !jar_filename.ends_with(".jar") || Path::new(jar_filename).components().count() != 1 {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "managed package filename must end in '.jar' or '.jar.disabled': '{filename}'"
        )));
    }
    Ok(if enabled {
        jar_filename.to_string()
    } else {
        format!("{jar_filename}{DISABLED_SUFFIX}")
    })
}

/// Atomically converge one package's TOML state, lock filename, and physical
/// top-level carrier as far as ordinary filesystem operations allow.
pub fn set_package_activation(
    instance_dir: &Path,
    package: &str,
    enabled: bool,
    dry_run: bool,
) -> Result<PackageActivationReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    let mut lock = Lockfile::open(instance_dir)?;
    let specification = manifest
        .inner
        .packages
        .get(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let entry = lock
        .inner
        .find(package)
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "orbit.lock has no exact realization for package '{package}'; run 'orbit sync' or 'orbit fix' first"
            ))
        })?;
    let previous_enabled = mod_artifact_enabled(&entry.filename).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "locked package '{package}' has unsupported physical filename '{}'; run 'orbit sync'",
            entry.filename
        ))
    })?;
    let target_filename = filename_for_activation(&entry.filename, enabled)?;
    let changed = previous_enabled != enabled || specification.enabled != enabled;
    let report = PackageActivationReport {
        package: package.to_string(),
        previous_enabled,
        enabled,
        changed,
        dry_run,
    };
    let mods_dir = instance_dir.join("mods");
    let source = lock.find_jar_path(package, &mods_dir)?;
    let target = mods_dir.join(&target_filename);
    if source != target && target.exists() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "cannot {} package '{package}': target file already exists: {}",
            if enabled { "enable" } else { "disable" },
            target.display()
        )));
    }
    if dry_run || !changed {
        return Ok(report);
    }

    manifest
        .inner
        .packages
        .get_mut(package)
        .expect("package existence was checked")
        .enabled = enabled;
    lock.inner
        .packages
        .iter_mut()
        .find(|entry| entry.mod_id == package)
        .expect("lock package existence was checked")
        .filename = target_filename;

    if source != target {
        std::fs::rename(&source, &target).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "cannot {} package '{package}' by renaming '{}' to '{}': {error}",
                if enabled { "enable" } else { "disable" },
                source.display(),
                target.display()
            ))
        })?;
    }

    if let Err(error) = crate::workspace::save_workspace(&manifest, &lock) {
        if source != target
            && let Err(rollback) = std::fs::rename(&target, &source)
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "failed to persist package '{package}' activation state: {error}; restoring '{}' to '{}' also failed: {rollback}",
                target.display(),
                source.display()
            )));
        }
        return Err(error);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{ArtifactSource, LockMeta, OrbitLockfile, PackageEntry};
    use crate::manifest::{
        OrbitManifest, PackageRemote, PackageSpec, PlatformArtifact, PlatformSnapshot, ProjectMeta,
        ResolverConfig,
    };

    fn workspace() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let mods = directory.path().join("mods");
        std::fs::create_dir(&mods).unwrap();
        let bytes = b"package bytes";
        std::fs::write(mods.join("alpha.jar"), bytes).unwrap();
        let sha256 = crate::jar::sha256_digest(bytes);
        let sha512 = crate::jar::sha512_digest(bytes);
        let remote = PackageRemote::File {
            path: ".orbit/sources/alpha.jar".to_string(),
        };
        let manifest = OrbitManifest {
            project: ProjectMeta {
                name: "test".to_string(),
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
                description: None,
                authors: None,
                version: None,
            },
            platform: PlatformSnapshot {
                minecraft_jar: PlatformArtifact {
                    path: "minecraft.jar".to_string(),
                    sha256: "minecraft".to_string(),
                },
                loader_jar: PlatformArtifact {
                    path: "loader.jar".to_string(),
                    sha256: "loader".to_string(),
                },
                runtime_jars: Vec::new(),
                physical_environment: crate::metadata::Environment::Client,
            },
            resolver: ResolverConfig::default(),
            packages: indexmap::IndexMap::from([(
                "alpha".to_string(),
                PackageSpec::new("*", vec![remote.clone()]),
            )]),
            groups: indexmap::IndexMap::new(),
        };
        ManifestFile::new(directory.path(), manifest)
            .save()
            .unwrap();
        Lockfile::new(
            directory.path(),
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![PackageEntry {
                    mod_id: "alpha".to_string(),
                    version: "1".to_string(),
                    sha1: String::new(),
                    sha256,
                    sha512,
                    filename: "alpha.jar".to_string(),
                    remotes: vec![remote],
                    artifact_sources: vec![ArtifactSource::File {
                        path: ".orbit/sources/alpha.jar".to_string(),
                    }],
                    dependencies: Vec::new(),
                    environment: crate::metadata::Environment::Both,
                    provides: Vec::new(),
                    language_loader: None,
                    embedded_artifacts: Vec::new(),
                    bundled: Vec::new(),
                }],
            },
        )
        .save()
        .unwrap();
        directory
    }

    #[test]
    fn disable_and_enable_rename_the_carrier_and_persist_toml_and_lock() {
        let directory = workspace();

        let disabled = set_package_activation(directory.path(), "alpha", false, false).unwrap();
        assert!(disabled.changed);
        assert!(directory.path().join("mods/alpha.jar.disabled").is_file());
        assert!(!ManifestFile::open(directory.path()).unwrap().inner.packages["alpha"].enabled);
        assert_eq!(
            Lockfile::open(directory.path()).unwrap().inner.packages[0].filename,
            "alpha.jar.disabled"
        );

        let enabled = set_package_activation(directory.path(), "alpha", true, false).unwrap();
        assert!(enabled.changed);
        assert!(directory.path().join("mods/alpha.jar").is_file());
        assert!(ManifestFile::open(directory.path()).unwrap().inner.packages["alpha"].enabled);
    }

    #[test]
    fn dry_run_does_not_change_any_state() {
        let directory = workspace();
        let report = set_package_activation(directory.path(), "alpha", false, true).unwrap();
        assert!(report.changed);
        assert!(directory.path().join("mods/alpha.jar").is_file());
        assert!(ManifestFile::open(directory.path()).unwrap().inner.packages["alpha"].enabled);
    }
}
