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
    pub platform_changes: Vec<PlatformChange>,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub missing: Vec<String>,
    pub unlocked: Vec<String>,
    pub removed: Vec<RemovedPackage>,
    pub diagnostics: Vec<crate::resolver::types::CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformChange {
    pub field: &'static str,
    pub previous: String,
    pub current: String,
}

pub async fn sync_instance(
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    dry_run: bool,
    interaction: InstallInteraction,
) -> Result<SyncReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    let discovered_platform = crate::platform::discover_platform(instance_dir, None, None, None)?;
    let platform_changes =
        describe_platform_changes(&manifest.inner, &discovered_platform, instance_dir)?;
    crate::platform::apply_to_manifest(instance_dir, &mut manifest.inner, &discovered_platform)?;
    let mut lockfile = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest.inner.project.mc_version.clone(),
            modloader: manifest.inner.project.modloader.clone(),
            modloader_version: manifest.inner.project.modloader_version.clone(),
        },
    );
    let refreshed_lock_meta = LockMeta {
        mc_version: manifest.inner.project.mc_version.clone(),
        modloader: manifest.inner.project.modloader.clone(),
        modloader_version: manifest.inner.project.modloader_version.clone(),
    };
    let lock_metadata_changed = lockfile.inner.meta.mc_version != refreshed_lock_meta.mc_version
        || lockfile.inner.meta.modloader != refreshed_lock_meta.modloader
        || lockfile.inner.meta.modloader_version != refreshed_lock_meta.modloader_version;
    lockfile.inner.meta = refreshed_lock_meta;
    let scanned = crate::init::scan_mods_dir(instance_dir, &manifest.inner.project.modloader)?;
    let identified = identify_mods(&scanned, providers).await?;
    let local_entries: Vec<_> = identified
        .iter()
        .map(IdentifiedMod::to_package_entry)
        .collect();

    let InstallInteraction {
        select_package: _,
        select_resolution,
        confirm_install,
    } = interaction;
    let loader_package = discovered_platform.loader_package;
    let selection = match crate::package_reconciliation::select_local_packages(
        &manifest.inner,
        &local_entries,
        loader_package,
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
            let mut report = SyncReport {
                platform_changes,
                ..SyncReport::default()
            };
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
            if !dry_run && (!report.platform_changes.is_empty() || lock_metadata_changed) {
                manifest.save()?;
                lockfile.save()?;
            }
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
        platform_changes,
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

fn describe_platform_changes(
    manifest: &crate::manifest::OrbitManifest,
    discovered: &crate::platform::DiscoveredPlatform,
    instance_dir: &Path,
) -> Result<Vec<PlatformChange>, OrbitError> {
    let artifacts = discovered.artifacts(instance_dir)?;
    let mut changes = Vec::new();
    push_platform_change(
        &mut changes,
        "minecraft_version",
        &manifest.project.mc_version,
        &discovered.minecraft_version.id,
    );
    push_platform_change(
        &mut changes,
        "modloader",
        &manifest.project.modloader,
        &discovered.loader,
    );
    push_platform_change(
        &mut changes,
        "modloader_version",
        &manifest.project.modloader_version,
        &discovered.loader_version,
    );
    push_platform_change(
        &mut changes,
        "minecraft_jar",
        &manifest.platform.minecraft_jar.path,
        &artifacts.minecraft_jar.path,
    );
    push_platform_change(
        &mut changes,
        "loader_jar",
        &manifest.platform.loader_jar.path,
        &artifacts.loader_jar.path,
    );
    if manifest.platform.minecraft_jar.sha256 != artifacts.minecraft_jar.sha256 {
        changes.push(PlatformChange {
            field: "minecraft_jar_sha256",
            previous: manifest.platform.minecraft_jar.sha256.clone(),
            current: artifacts.minecraft_jar.sha256,
        });
    }
    if manifest.platform.loader_jar.sha256 != artifacts.loader_jar.sha256 {
        changes.push(PlatformChange {
            field: "loader_jar_sha256",
            previous: manifest.platform.loader_jar.sha256.clone(),
            current: artifacts.loader_jar.sha256,
        });
    }
    Ok(changes)
}

fn push_platform_change(
    changes: &mut Vec<PlatformChange>,
    field: &'static str,
    previous: &str,
    current: &str,
) {
    if previous != current {
        changes.push(PlatformChange {
            field,
            previous: previous.to_string(),
            current: current.to_string(),
        });
    }
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
        crate::platform::test_support::write_platform(&directory, "1", "fabric", "1");
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
            platform: crate::manifest::PlatformArtifacts {
                minecraft_jar: crate::manifest::PlatformArtifact {
                    path: "minecraft.jar".to_string(),
                    sha256: "test".to_string(),
                },
                loader_jar: crate::manifest::PlatformArtifact {
                    path: "loader.jar".to_string(),
                    sha256: "test".to_string(),
                },
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
        crate::platform::test_support::write_platform(&directory, "1.20.1", "fabric", "0.16.10");
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
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
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
                select_package: None,
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
                select_package: None,
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

    #[tokio::test]
    async fn refreshes_loader_version_and_artifact_paths_from_a_fresh_scan() {
        let directory = test_dir("platform-refresh");
        crate::platform::test_support::write_platform(&directory, "1.20.1", "fabric", "0.17.0");
        let manifest = OrbitManifest {
            project: ProjectMeta {
                name: "test".to_string(),
                mc_version: "1.20.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16.10".to_string(),
                description: None,
                authors: None,
                version: None,
            },
            platform: crate::manifest::PlatformArtifacts {
                minecraft_jar: crate::manifest::PlatformArtifact {
                    path: "old-minecraft.jar".to_string(),
                    sha256: "old".to_string(),
                },
                loader_jar: crate::manifest::PlatformArtifact {
                    path: "old-loader.jar".to_string(),
                    sha256: "old".to_string(),
                },
            },
            resolver: ResolverConfig::default(),
            dependencies: indexmap::IndexMap::new(),
            groups: indexmap::IndexMap::new(),
            overrides: indexmap::IndexMap::new(),
        };
        ManifestFile::new(&directory, manifest).save().unwrap();

        let report = sync_instance(&directory, &[], false, InstallInteraction::default())
            .await
            .unwrap();
        let refreshed = ManifestFile::open(&directory).unwrap();

        assert!(
            report
                .platform_changes
                .iter()
                .any(|change| change.field == "modloader_version" && change.current == "0.17.0")
        );
        assert_eq!(refreshed.inner.project.modloader_version, "0.17.0");
        assert!(
            refreshed
                .inner
                .platform
                .loader_jar
                .path
                .contains("fabric-loader/0.17.0")
        );
        assert_ne!(refreshed.inner.platform.loader_jar.sha256, "old");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
