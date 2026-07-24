//! Reconciles the manifest, lockfile, and local `mods/` directory without downloading JARs.

use std::collections::HashSet;
use std::path::Path;

use crate::error::OrbitError;
use crate::identification::{IdentifiedMod, identify_mods};
use crate::lockfile::LockMeta;
use crate::manifest::DependencySpec;
use crate::providers::ModProvider;
use crate::workspace::{Lockfile, ManifestFile};
use crate::{InstallInteraction, RemovedPackage};

#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub missing: Vec<String>,
    pub unlocked: Vec<String>,
    pub removed: Vec<RemovedPackage>,
    pub diagnostics: Vec<crate::resolver::types::CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

pub async fn sync_instance(
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    dry_run: bool,
    interaction: InstallInteraction,
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
    let local_entries: Vec<_> = identified
        .iter()
        .map(IdentifiedMod::to_package_entry)
        .collect();

    let InstallInteraction {
        select_resolution,
        confirm_install,
    } = interaction;
    let selection = match crate::package_reconciliation::select_local_packages(
        &manifest.inner,
        &local_entries,
        select_resolution,
    )
    .await
    {
        Ok(selection) => selection,
        Err(error) => {
            let discovered: HashSet<_> = local_entries
                .iter()
                .map(|entry| entry.mod_id.as_str())
                .collect();
            let mut report = SyncReport::default();
            for package in manifest.inner.dependencies.keys() {
                if discovered.contains(package.as_str()) {
                    continue;
                }
                if lockfile.inner.find(package).is_some() {
                    report.missing.push(package.clone());
                } else {
                    report.unlocked.push(package.clone());
                }
            }
            if report.missing.is_empty() && report.unlocked.is_empty() {
                return Err(OrbitError::Conflict(error));
            }
            report.missing.sort();
            report.unlocked.sort();
            return Ok(report);
        }
    };
    if confirm_install.is_some_and(|confirm| {
        !confirm(&crate::package_reconciliation::confirmation_report(
            &selection,
        ))
    }) {
        return Ok(SyncReport::default());
    }
    let crate::package_reconciliation::LocalPackageSelection {
        selected_entries,
        removed,
        resolution,
    } = selection;

    let mut report = SyncReport {
        removed: removed.clone(),
        diagnostics: resolution.diagnostics.clone(),
        warnings: resolution.warnings.clone(),
        ..SyncReport::default()
    };
    for entry in &selected_entries {
        if !manifest.inner.dependencies.contains_key(&entry.mod_id) {
            report.added.push(entry.mod_id.clone());
        }
        match lockfile.inner.find(&entry.mod_id) {
            Some(locked)
                if locked.sha256 != entry.sha256
                    || locked.filename != entry.filename
                    || locked.version != entry.version =>
            {
                report.changed.push(entry.mod_id.clone());
            }
            None if manifest.inner.dependencies.contains_key(&entry.mod_id) => {
                report.unlocked.push(entry.mod_id.clone());
            }
            _ => {}
        }
    }
    report.added.sort();
    report.added.dedup();
    report.changed.sort();
    report.changed.dedup();
    report.unlocked.sort();
    report.unlocked.dedup();

    if dry_run {
        return Ok(report);
    }

    crate::package_reconciliation::remove_unselected_packages(instance_dir, &removed)?;
    for entry in &selected_entries {
        manifest
            .inner
            .dependencies
            .entry(entry.mod_id.clone())
            .or_insert_with(|| DependencySpec::Short(entry.version.clone()));
    }
    lockfile.inner.packages = selected_entries;
    lockfile
        .inner
        .packages
        .sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    manifest.save()?;
    lockfile.save()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{FileInfo, OrbitLockfile, PackageEntry};
    use crate::manifest::{OrbitManifest, ProjectMeta, ResolverConfig};
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use zip::write::SimpleFileOptions;

    fn test_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("orbit-sync-test-{name}-{}", std::process::id()))
    }

    fn write_fabric_jar(path: &Path, version: &str) {
        let file = std::fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("fabric.mod.json", SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"{{"schemaVersion":1,"id":"alpha","version":"{version}","name":"Alpha"}}"#
        )
        .unwrap();
        archive.finish().unwrap();
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

        let report = sync_instance(&directory, &[], true, InstallInteraction::default())
            .await
            .unwrap();

        assert_eq!(report.missing, vec!["missing"]);
        assert_eq!(report.unlocked, vec!["unlocked"]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn duplicate_package_versions_are_confirmed_and_removed_as_top_level_packages() {
        let directory = test_dir("duplicates");
        let mods = directory.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        write_fabric_jar(&mods.join("a-1.jar"), "1");
        write_fabric_jar(&mods.join("a-2.jar"), "2");
        assert_eq!(
            crate::jar::read_mod_metadata(&mods.join("a-1.jar"), "fabric")
                .unwrap()
                .mod_id,
            "alpha"
        );
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
[dependencies]
alpha = "*"
"#,
        )
        .unwrap();
        ManifestFile::new(&directory, manifest).save().unwrap();
        let scanned = crate::init::scan_mods_dir(&directory, "fabric").unwrap();
        assert_eq!(
            scanned
                .iter()
                .filter_map(|package| package.mod_id.as_deref())
                .collect::<Vec<_>>(),
            vec!["alpha", "alpha"]
        );

        let preview = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&preview);
        let aborted = sync_instance(
            &directory,
            &[],
            false,
            InstallInteraction {
                select_resolution: None,
                confirm_install: Some(Box::new(move |report| {
                    assert!(!report.removed.is_empty(), "{report:?}");
                    *captured.lock().unwrap() = report.removed.clone();
                    false
                })),
            },
        )
        .await
        .unwrap();
        assert!(aborted.removed.is_empty());
        assert_eq!(preview.lock().unwrap()[0].filename, "a-1.jar");
        assert!(mods.join("a-1.jar").exists());

        let applied = sync_instance(
            &directory,
            &[],
            false,
            InstallInteraction {
                select_resolution: None,
                confirm_install: Some(Box::new(|_| true)),
            },
        )
        .await
        .unwrap();
        assert_eq!(applied.removed[0].filename, "a-1.jar");
        assert!(!mods.join("a-1.jar").exists());
        assert!(mods.join("a-2.jar").exists());
        assert_eq!(
            Lockfile::open(&directory)
                .unwrap()
                .inner
                .find("alpha")
                .unwrap()
                .version,
            "2"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
