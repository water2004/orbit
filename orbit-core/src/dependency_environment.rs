//! Explicit managed-package environment filters in `orbit.toml`.

use std::path::Path;

use crate::error::OrbitError;
use crate::metadata::Environment;
use crate::workspace::{Lockfile, ManifestFile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEnvironmentReport {
    pub package: String,
    /// `None` means the selected JAR declaration remains authoritative.
    pub configured: Option<Environment>,
    /// Available when an explicit override or a selected lock entry exists.
    pub effective: Option<Environment>,
    pub dry_run: bool,
}

/// Set an explicit package filter, or pass `auto` to follow the selected JAR.
pub fn set_package_environment(
    instance_dir: &Path,
    package: &str,
    value: &str,
    dry_run: bool,
) -> Result<PackageEnvironmentReport, OrbitError> {
    let configured = if value == "auto" {
        None
    } else {
        Some(value.parse().map_err(|_| {
            OrbitError::Other(anyhow::anyhow!(
                "invalid package environment '{value}'; expected client, server, both, or auto"
            ))
        })?)
    };
    let mut manifest = ManifestFile::open(instance_dir)?;
    let requirement = manifest
        .inner
        .packages
        .get_mut(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    requirement.env = configured;

    let declared = if instance_dir.join("orbit.lock").exists() {
        Lockfile::open(instance_dir)?
            .inner
            .find(package)
            .map(|entry| entry.environment)
    } else {
        None
    };
    let effective = configured.or(declared);
    if !dry_run {
        manifest.save()?;
    }
    Ok(PackageEnvironmentReport {
        package: package.to_string(),
        configured,
        effective,
        dry_run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{ArtifactSource, LockMeta, OrbitLockfile, PackageEntry};
    use crate::manifest::{
        OrbitManifest, PackageRemote, PackageSpec, PlatformArtifact, PlatformSnapshot, ProjectMeta,
    };
    use indexmap::IndexMap;

    fn instance(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "orbit-environment-test-{name}-{}",
            std::process::id()
        ))
    }

    fn write_instance(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
        std::fs::create_dir_all(path).unwrap();
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
                physical_environment: Environment::Client,
            },
            resolver: Default::default(),
            packages: IndexMap::from([(
                "example".to_string(),
                PackageSpec::new(
                    "*",
                    vec![PackageRemote::File {
                        path: "example.jar".to_string(),
                    }],
                ),
            )]),
            groups: Default::default(),
        };
        ManifestFile::new(path, manifest).save().unwrap();
        Lockfile::new(
            path,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![PackageEntry {
                    mod_id: "example".to_string(),
                    version: "1".to_string(),
                    sha1: String::new(),
                    sha256: crate::jar::sha256_digest(b"example"),
                    sha512: crate::jar::sha512_digest(b"example"),
                    filename: "example.jar".to_string(),
                    remotes: vec![PackageRemote::File {
                        path: "example.jar".to_string(),
                    }],
                    artifact_sources: vec![ArtifactSource::File {
                        path: "example.jar".to_string(),
                    }],
                    dependencies: Vec::new(),
                    environment: Environment::Client,
                    provides: Vec::new(),
                    language_loader: None,
                    embedded_artifacts: Vec::new(),
                    bundled: Vec::new(),
                }],
            },
        )
        .save()
        .unwrap();
    }

    #[test]
    fn explicit_override_and_auto_roundtrip_without_changing_lock() {
        let path = instance("roundtrip");
        write_instance(&path);

        let explicit = set_package_environment(&path, "example", "server", false).unwrap();
        assert_eq!(explicit.configured, Some(Environment::Server));
        assert_eq!(explicit.effective, Some(Environment::Server));
        assert_eq!(
            ManifestFile::open(&path).unwrap().inner.packages["example"].env(),
            Some(Environment::Server)
        );

        let automatic = set_package_environment(&path, "example", "auto", false).unwrap();
        assert_eq!(automatic.configured, None);
        assert_eq!(automatic.effective, Some(Environment::Client));
        assert_eq!(
            ManifestFile::open(&path).unwrap().inner.packages["example"].env(),
            None
        );
        assert_eq!(
            Lockfile::open(&path)
                .unwrap()
                .inner
                .find("example")
                .unwrap()
                .environment,
            Environment::Client
        );
        std::fs::remove_dir_all(path).unwrap();
    }
}
