//! Package remote management.

use std::path::Path;

use crate::error::OrbitError;
use crate::lockfile::{LockMeta, OrbitLockfile};
use crate::manifest::PackageRemote;
use crate::progress::ProgressReporter;
use crate::providers::ModProvider;
use crate::workspace::{Lockfile, ManifestFile};

#[derive(Debug, Clone)]
pub struct RemoteReport {
    pub package: String,
    pub remotes: Vec<PackageRemote>,
    pub changed: bool,
}

pub async fn add_package_remote(
    instance_dir: &Path,
    package: &str,
    remote: PackageRemote,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    dry_run: bool,
    progress: Option<ProgressReporter>,
) -> Result<RemoteReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    if !manifest.inner.packages.contains_key(package) {
        return Err(OrbitError::ModNotFound(package.to_string()));
    }
    let remote = match remote {
        PackageRemote::File { path } => {
            let source = {
                let path = Path::new(&path);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    instance_dir.join(path)
                }
            };
            let source = std::fs::canonicalize(&source).map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "cannot open local remote {}: {error}",
                    source.display()
                ))
            })?;
            if !source.is_file() {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "local remote is not a file: {}",
                    source.display()
                )));
            }
            PackageRemote::File {
                path: source.to_string_lossy().into_owned(),
            }
        }
        remote => remote,
    };
    let requirement = manifest
        .inner
        .packages
        .get(package)
        .expect("package existence was checked above");
    if requirement.remotes.contains(&remote) {
        return Ok(RemoteReport {
            package: package.to_string(),
            remotes: requirement.remotes.clone(),
            changed: false,
        });
    }

    let empty_lock = OrbitLockfile {
        meta: LockMeta {
            mc_version: manifest.inner.project.mc_version.clone(),
            modloader: manifest.inner.project.modloader.clone(),
            modloader_version: manifest.inner.project.modloader_version.clone(),
        },
        packages: Vec::new(),
    };
    let catalog = crate::outdated::download_candidate_catalog(
        crate::outdated::CandidateDiscoveryInput {
            instance_dir,
            providers,
            additional_remotes: &[],
            lockfile: &empty_lock,
            mc_version: &manifest.inner.project.mc_version,
            loader: manifest.inner.project.loader_kind()?,
            jar_cache,
            progress,
        },
        std::slice::from_ref(&remote),
    )
    .await?;
    if !catalog.requested_packages.contains(package) {
        return Err(OrbitError::Conflict(format!(
            "remote '{}' returned JARs declaring [{}], not package '{package}'",
            remote.display_locator(),
            catalog
                .requested_packages
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let mut remote = catalog
        .requested_remotes_for_package(package)
        .into_iter()
        .find(|candidate| candidate.provider() == remote.provider())
        .unwrap_or(remote);
    if !dry_run && let PackageRemote::File { path } = &remote {
        let source = Path::new(path);
        let sha512 = crate::jar::compute_sha512(source)?;
        remote = crate::source_store::preserve_if_instance_output(instance_dir, source, &sha512)?;
    }

    let requirement = manifest
        .inner
        .packages
        .get_mut(package)
        .expect("package was checked above");
    if requirement.remotes.contains(&remote) {
        return Ok(RemoteReport {
            package: package.to_string(),
            remotes: requirement.remotes.clone(),
            changed: false,
        });
    }
    requirement.remotes.push(remote.clone());
    requirement.remotes.sort();
    requirement.remotes.dedup();
    let remotes = requirement.remotes.clone();
    if !dry_run {
        let mut lockfile = open_optional_lock(instance_dir)?;
        if let Some(entry) = lockfile.as_mut().and_then(|lockfile| {
            lockfile
                .inner
                .packages
                .iter_mut()
                .find(|entry| entry.mod_id == package)
        }) {
            if !entry.remotes.contains(&remote) {
                entry.remotes.push(remote.clone());
                entry.remotes.sort();
                entry.remotes.dedup();
            }
            let selected_id = format!("sha512:{}", entry.sha512);
            if let Some(selected) = catalog.resolved.get(&selected_id) {
                for source in &selected.sources {
                    let source = match (&remote, source) {
                        (
                            PackageRemote::File { path },
                            crate::lockfile::ArtifactSource::File { .. },
                        ) => crate::lockfile::ArtifactSource::File { path: path.clone() },
                        _ => source.clone(),
                    };
                    if !entry.artifact_sources.contains(&source) {
                        entry.artifact_sources.push(source);
                    }
                }
                entry
                    .artifact_sources
                    .sort_by_key(|source| format!("{source:?}"));
            }
        }
        manifest.inner.validate()?;
        if let Some(lockfile) = &lockfile {
            lockfile.inner.validate()?;
        }
        manifest.save()?;
        if let Some(lockfile) = lockfile {
            lockfile.save()?;
        }
    }
    Ok(RemoteReport {
        package: package.to_string(),
        remotes,
        changed: true,
    })
}

pub fn remove_package_remote(
    instance_dir: &Path,
    package: &str,
    remote: &PackageRemote,
    dry_run: bool,
) -> Result<RemoteReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    let requirement = manifest
        .inner
        .packages
        .get_mut(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let Some(index) = requirement
        .remotes
        .iter()
        .position(|candidate| candidate == remote)
    else {
        return Err(OrbitError::ModNotFound(remote.display_locator()));
    };
    if requirement.remotes.len() == 1 {
        return Err(OrbitError::Conflict(format!(
            "cannot remove the last remote from package '{package}'"
        )));
    }
    requirement.remotes.remove(index);
    let remotes = requirement.remotes.clone();
    if !dry_run {
        let mut lockfile = open_optional_lock(instance_dir)?;
        if let Some(entry) = lockfile.as_mut().and_then(|lockfile| {
            lockfile
                .inner
                .packages
                .iter_mut()
                .find(|entry| entry.mod_id == package)
        }) {
            entry.remotes.retain(|candidate| candidate != remote);
        }
        manifest.inner.validate()?;
        if let Some(lockfile) = &lockfile {
            lockfile.inner.validate()?;
        }
        manifest.save()?;
        if let Some(lockfile) = lockfile {
            lockfile.save()?;
        }
    }
    Ok(RemoteReport {
        package: package.to_string(),
        remotes,
        changed: true,
    })
}

fn open_optional_lock(instance_dir: &Path) -> Result<Option<Lockfile>, OrbitError> {
    match Lockfile::open(instance_dir) {
        Ok(lockfile) => Ok(Some(lockfile)),
        Err(OrbitError::LockfileNotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn list_package_remotes(
    instance_dir: &Path,
    package: &str,
) -> Result<RemoteReport, OrbitError> {
    let manifest = ManifestFile::open(instance_dir)?;
    let requirement = manifest
        .inner
        .packages
        .get(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    Ok(RemoteReport {
        package: package.to_string(),
        remotes: requirement.remotes.clone(),
        changed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        OrbitManifest, PackageSpec, PlatformArtifact, PlatformSnapshot, ProjectMeta, ResolverConfig,
    };

    fn manifest(remote: PackageRemote) -> OrbitManifest {
        OrbitManifest {
            project: ProjectMeta {
                name: "test".to_string(),
                mc_version: "1.20.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16.10".to_string(),
                description: None,
                authors: None,
                version: None,
            },
            platform: PlatformSnapshot {
                minecraft_jar: PlatformArtifact {
                    path: "minecraft.jar".to_string(),
                    sha256: "test".to_string(),
                },
                loader_jar: PlatformArtifact {
                    path: "loader.jar".to_string(),
                    sha256: "test".to_string(),
                },
                runtime_jars: Vec::new(),
                physical_environment: crate::metadata::Environment::Client,
            },
            resolver: ResolverConfig::default(),
            packages: indexmap::IndexMap::from([(
                "sodium".to_string(),
                PackageSpec::new("*", vec![remote]),
            )]),
            groups: indexmap::IndexMap::new(),
        }
    }

    #[test]
    fn last_remote_cannot_be_removed() {
        let directory = tempfile::tempdir().unwrap();
        let remote = PackageRemote::Modrinth {
            project_id: "AANobbMI".to_string(),
        };
        ManifestFile::new(directory.path(), manifest(remote.clone()))
            .save()
            .unwrap();

        let error = remove_package_remote(directory.path(), "sodium", &remote, false).unwrap_err();

        assert!(error.to_string().contains("last remote"));
        assert_eq!(
            list_package_remotes(directory.path(), "sodium")
                .unwrap()
                .remotes,
            vec![remote]
        );
    }

    #[test]
    fn removing_a_discovery_remote_preserves_the_locked_exact_source() {
        let directory = tempfile::tempdir().unwrap();
        let modrinth = PackageRemote::Modrinth {
            project_id: "AANobbMI".to_string(),
        };
        let curseforge = PackageRemote::Curseforge { project_id: 394468 };
        let mut manifest = manifest(modrinth.clone());
        manifest.packages["sodium"].remotes.push(curseforge.clone());
        ManifestFile::new(directory.path(), manifest)
            .save()
            .unwrap();
        Lockfile::new(
            directory.path(),
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1.20.1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "0.16.10".to_string(),
                },
                packages: vec![crate::lockfile::PackageEntry {
                    mod_id: "sodium".to_string(),
                    version: "0.5.8".to_string(),
                    sha1: "sha1".to_string(),
                    sha256: "sha256".to_string(),
                    sha512: "sha512".to_string(),
                    filename: "sodium.jar".to_string(),
                    remotes: vec![modrinth.clone(), curseforge.clone()],
                    artifact_sources: vec![crate::lockfile::ArtifactSource::Modrinth {
                        project_id: "AANobbMI".to_string(),
                        version_id: "release".to_string(),
                        download_url: "https://cdn.modrinth.com/sodium.jar".to_string(),
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

        remove_package_remote(directory.path(), "sodium", &modrinth, false).unwrap();

        let manifest = ManifestFile::open(directory.path()).unwrap();
        assert_eq!(
            manifest.inner.packages["sodium"].remotes,
            vec![curseforge.clone()]
        );
        let lock = Lockfile::open(directory.path()).unwrap();
        assert_eq!(lock.inner.packages[0].remotes, vec![curseforge]);
        assert!(matches!(
            lock.inner.packages[0].artifact_sources[0],
            crate::lockfile::ArtifactSource::Modrinth { .. }
        ));
    }
}
