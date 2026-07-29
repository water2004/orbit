//! Explicit version policy for managed logical packages.

use std::path::Path;

use crate::error::OrbitError;
use crate::versions::Version;
use crate::workspace::{Lockfile, ManifestFile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConstraintReport {
    pub package: String,
    pub previous: String,
    pub current: String,
    pub selected_version: Option<String>,
    pub selected_satisfies: Option<bool>,
    pub changed: bool,
    pub dry_run: bool,
}

pub fn package_constraint(
    instance_dir: &Path,
    package: &str,
) -> Result<PackageConstraintReport, OrbitError> {
    change_package_constraint(instance_dir, package, None, false)
}

pub fn set_package_constraint(
    instance_dir: &Path,
    package: &str,
    constraint: &str,
    dry_run: bool,
) -> Result<PackageConstraintReport, OrbitError> {
    let constraint = constraint.trim();
    if constraint.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "package version constraint cannot be empty; use '*' to allow every version"
        )));
    }
    change_package_constraint(instance_dir, package, Some(constraint), dry_run)
}

fn change_package_constraint(
    instance_dir: &Path,
    package: &str,
    constraint: Option<&str>,
    dry_run: bool,
) -> Result<PackageConstraintReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    let loader = manifest.inner.project.loader_kind()?;
    let specification = manifest
        .inner
        .packages
        .get_mut(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let previous = specification.version.clone();
    let current = constraint.unwrap_or(&previous).to_string();
    let changed = previous != current;
    if constraint.is_some() {
        specification.version.clone_from(&current);
    }

    let selected_version = if instance_dir.join("orbit.lock").is_file() {
        Lockfile::open(instance_dir)?
            .inner
            .find(package)
            .map(|entry| entry.version.clone())
    } else {
        None
    };
    let selected_satisfies = selected_version.as_ref().map(|version| {
        Version::parse_constraint(&current, loader).contains(&Version::parse(version, loader))
    });
    if constraint.is_some() && changed && !dry_run {
        manifest.save()?;
    }

    Ok(PackageConstraintReport {
        package: package.to_string(),
        previous,
        current,
        selected_version,
        selected_satisfies,
        changed,
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
    use crate::metadata::Environment;
    use crate::workspace::{Lockfile, ManifestFile};
    use indexmap::IndexMap;

    fn instance() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let manifest = OrbitManifest {
            project: ProjectMeta {
                name: "test".to_string(),
                mc_version: "1.20.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16".to_string(),
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
            groups: IndexMap::new(),
        };
        ManifestFile::new(directory.path(), manifest)
            .save()
            .unwrap();
        Lockfile::new(
            directory.path(),
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1.20.1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "0.16".to_string(),
                },
                packages: vec![PackageEntry {
                    mod_id: "example".to_string(),
                    version: "1.2.3-beta".to_string(),
                    sha1: String::new(),
                    sha256: String::new(),
                    sha512: "example-content".to_string(),
                    filename: "example.jar".to_string(),
                    remotes: vec![PackageRemote::File {
                        path: "example.jar".to_string(),
                    }],
                    artifact_sources: vec![ArtifactSource::File {
                        path: "example.jar".to_string(),
                    }],
                    dependencies: Vec::new(),
                    environment: Environment::Both,
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
    fn setting_a_constraint_only_changes_manifest_intent() {
        let directory = instance();

        let report = set_package_constraint(directory.path(), "example", "=1.2.3", false).unwrap();

        assert!(report.changed);
        assert_eq!(report.selected_satisfies, Some(true));
        assert_eq!(
            ManifestFile::open(directory.path()).unwrap().inner.packages["example"].version,
            "=1.2.3"
        );
        assert_eq!(
            Lockfile::open(directory.path()).unwrap().inner.packages[0].version,
            "1.2.3-beta"
        );
    }

    #[test]
    fn explicit_suffix_reports_current_selection_as_outside_policy() {
        let directory = instance();

        let report =
            set_package_constraint(directory.path(), "example", "=1.2.3-alpha", true).unwrap();

        assert_eq!(report.selected_satisfies, Some(false));
        assert_eq!(
            ManifestFile::open(directory.path()).unwrap().inner.packages["example"].version,
            "*"
        );
    }
}
