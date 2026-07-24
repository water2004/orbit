//! Reconciles the manifest, lockfile, and local `mods/` directory without downloading JARs.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::OrbitError;
use crate::identification::{IdentifiedMod, IdentifiedPlatform, IdentifiedSource, identify_mods};
use crate::lockfile::{CurseForgeInfo, FileInfo, LockMeta, ModrinthInfo, PackageEntry};
use crate::manifest::DependencySpec;
use crate::providers::ModProvider;
use crate::workspace::{Lockfile, ManifestFile};

#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub missing: Vec<String>,
    pub unlocked: Vec<String>,
}

pub async fn sync_instance(
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    dry_run: bool,
) -> Result<SyncReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    let mut lockfile = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest.inner.project.mc_version.clone(),
            modloader: manifest.inner.project.modloader.clone(),
            modloader_version: manifest.inner.project.modloader_version.clone(),
        },
    );
    let scanned = crate::init::scan_mods_dir(instance_dir, &manifest.inner.project.modloader)?;
    let identified = identify_mods(&scanned, providers).await?;

    let by_filename: HashMap<_, _> = identified
        .iter()
        .map(|identified| (identified.filename.as_str(), identified))
        .collect();
    let by_package: HashMap<_, _> = identified
        .iter()
        .filter_map(|identified| {
            let package = package_id(identified);
            (!package.is_empty()).then_some((package, identified))
        })
        .collect();
    let mut represented_files = HashSet::new();
    let mut report = SyncReport::default();

    for package in manifest.inner.dependencies.keys() {
        let Some(entry) = lockfile.inner.find(package) else {
            report.unlocked.push(package.clone());
            if let Some(local) = by_package.get(package.as_str()) {
                represented_files.insert(local.filename.clone());
            }
            continue;
        };
        let local = by_filename
            .get(entry.filename.as_str())
            .copied()
            .or_else(|| by_package.get(package.as_str()).copied());
        let Some(local) = local else {
            report.missing.push(package.clone());
            continue;
        };
        represented_files.insert(local.filename.clone());
        if !local.sha256.is_empty() && entry.sha256 != local.sha256 {
            report.changed.push(package.clone());
        }
    }

    let locked_files: HashSet<_> = lockfile
        .inner
        .packages
        .iter()
        .map(|entry| entry.filename.as_str())
        .filter(|filename| !filename.is_empty())
        .collect();
    for local in &identified {
        let package = package_id(local);
        if package.is_empty()
            || represented_files.contains(&local.filename)
            || locked_files.contains(local.filename.as_str())
            || manifest.inner.dependencies.contains_key(&package)
        {
            continue;
        }
        report.added.push(package);
    }

    report.added.sort();
    report.changed.sort();
    report.missing.sort();
    report.unlocked.sort();

    if !dry_run {
        apply_changes(&mut manifest, &mut lockfile, &identified, &report)?;
    }
    Ok(report)
}

fn apply_changes(
    manifest: &mut ManifestFile,
    lockfile: &mut Lockfile,
    identified: &[IdentifiedMod],
    report: &SyncReport,
) -> Result<(), OrbitError> {
    for package in &report.changed {
        let Some(local) = identified
            .iter()
            .find(|identified| package_id(identified) == *package)
        else {
            continue;
        };
        if let Some(entry) = lockfile
            .inner
            .packages
            .iter_mut()
            .find(|entry| entry.mod_id == *package)
        {
            *entry = package_entry(local);
        }
    }
    for package in &report.added {
        let Some(local) = identified
            .iter()
            .find(|identified| package_id(identified) == *package)
        else {
            continue;
        };
        manifest
            .inner
            .dependencies
            .entry(package.clone())
            .or_insert_with(|| {
                DependencySpec::Short(if local.version.is_empty() {
                    "*".to_string()
                } else {
                    local.version.clone()
                })
            });
        lockfile
            .inner
            .packages
            .retain(|entry| entry.mod_id != *package);
        lockfile.inner.packages.push(package_entry(local));
    }
    if !report.added.is_empty() || !report.changed.is_empty() {
        lockfile
            .inner
            .packages
            .sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
        manifest.save()?;
        lockfile.save()?;
    }
    Ok(())
}

fn package_entry(local: &IdentifiedMod) -> PackageEntry {
    let mod_id = package_id(local);
    let common = |provider: String,
                  modrinth: Option<ModrinthInfo>,
                  curseforge: Option<CurseForgeInfo>,
                  file: Option<FileInfo>| PackageEntry {
        mod_id: mod_id.clone(),
        version: local.version.clone(),
        sha1: local.sha1.clone(),
        sha256: local.sha256.clone(),
        sha512: local.sha512.clone(),
        filename: local.filename.clone(),
        provider,
        modrinth,
        curseforge,
        file,
        dependencies: local.dependencies.clone(),
        environment: local.environment,
        provides: local.provides.clone(),
        language_loader: local.language_loader.clone(),
        embedded_artifacts: local.embedded_artifacts.clone(),
        bundled: local.bundled.clone(),
    };
    match &local.source {
        IdentifiedSource::Platform(IdentifiedPlatform::Modrinth(metadata)) => {
            common("modrinth".to_string(), Some(metadata.clone()), None, None)
        }
        IdentifiedSource::Platform(IdentifiedPlatform::CurseForge(metadata)) => {
            common("curseforge".to_string(), None, Some(metadata.clone()), None)
        }
        IdentifiedSource::File { .. } => common(
            "file".to_string(),
            None,
            None,
            Some(FileInfo {
                path: format!("mods/{}", local.filename),
            }),
        ),
    }
}

fn package_id(local: &IdentifiedMod) -> String {
    if !local.mod_id.is_empty() {
        local.mod_id.clone()
    } else if !local.mod_name.is_empty() {
        local.mod_name.clone()
    } else {
        local
            .filename
            .strip_suffix(".jar")
            .unwrap_or(&local.filename)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::OrbitLockfile;
    use crate::manifest::{OrbitManifest, ProjectMeta, ResolverConfig};

    fn test_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("orbit-sync-test-{name}-{}", std::process::id()))
    }

    #[tokio::test]
    async fn reports_missing_and_unlocked_manifest_dependencies() {
        let directory = test_dir("states");
        std::fs::create_dir_all(&directory).unwrap();
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
            resolver: ResolverConfig::default(),
            dependencies: indexmap::IndexMap::from([
                (
                    "missing".to_string(),
                    DependencySpec::Short("*".to_string()),
                ),
                (
                    "unlocked".to_string(),
                    DependencySpec::Short("*".to_string()),
                ),
            ]),
            groups: indexmap::IndexMap::new(),
            overrides: indexmap::IndexMap::new(),
        };
        ManifestFile::new(&directory, manifest).save().unwrap();
        Lockfile::new(
            &directory,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![PackageEntry {
                    mod_id: "missing".to_string(),
                    version: "1".to_string(),
                    sha1: String::new(),
                    sha256: "hash".to_string(),
                    sha512: String::new(),
                    filename: "missing.jar".to_string(),
                    provider: "file".to_string(),
                    modrinth: None,
                    curseforge: None,
                    file: Some(FileInfo {
                        path: "mods/missing.jar".to_string(),
                    }),
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

        let report = sync_instance(&directory, &[], true).await.unwrap();

        assert_eq!(report.missing, vec!["missing"]);
        assert_eq!(report.unlocked, vec!["unlocked"]);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
