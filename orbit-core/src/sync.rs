//! Reconciles the manifest, lockfile, and local `mods/` directory without downloading JARs.
//!
//! Local artifacts are batch-identified by every available provider before
//! reconciliation. Provider metadata is used only to recover source locators;
//! package identity and dependency metadata still come exclusively from JARs.

use std::collections::HashSet;
use std::path::Path;

use crate::error::OrbitError;
use crate::identification::{IdentifiedMod, identify_mods};
use crate::lockfile::LockMeta;
use crate::manifest::DependencySpec;
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
    providers: &[Box<dyn crate::providers::ModProvider>],
    dry_run: bool,
    interaction: InstallInteraction,
) -> Result<SyncReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    // Reconciliation must observe the launcher as it exists now. The manifest
    // is only the previous snapshot used to describe changes; none of its
    // versions or paths may constrain discovery.
    let discovered_platform = crate::platform_detection::rediscover_current_platform(instance_dir)?;
    let platform_snapshot = discovered_platform.snapshot(instance_dir)?;
    let platform_changes =
        describe_platform_changes(&manifest.inner, &discovered_platform, &platform_snapshot);
    crate::platform_detection::apply_to_manifest(
        &mut manifest.inner,
        &discovered_platform,
        platform_snapshot,
    );
    let mut lockfile = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest.inner.project.mc_version.clone(),
            modloader: manifest.inner.project.modloader.clone(),
            modloader_version: manifest.inner.project.modloader_version.clone(),
        },
    )?;
    let refreshed_lock_meta = LockMeta {
        mc_version: manifest.inner.project.mc_version.clone(),
        modloader: manifest.inner.project.modloader.clone(),
        modloader_version: manifest.inner.project.modloader_version.clone(),
    };
    let lock_metadata_changed = lockfile.inner.meta.mc_version != refreshed_lock_meta.mc_version
        || lockfile.inner.meta.modloader != refreshed_lock_meta.modloader
        || lockfile.inner.meta.modloader_version != refreshed_lock_meta.modloader_version;
    lockfile.inner.meta = refreshed_lock_meta;
    let scanned = crate::init::scan_mods_dir(instance_dir, discovered_platform.loader)?;
    let mut identified = identify_mods(&scanned, providers).await?;
    let mut superseded_managed_remotes = std::collections::HashMap::<
        String,
        std::collections::HashSet<crate::manifest::PackageRemote>,
    >::new();
    for package in &identified {
        if package
            .remotes
            .iter()
            .any(|remote| remote.provider() != "file")
        {
            superseded_managed_remotes
                .entry(package.package_id())
                .or_default()
                .insert(crate::source_store::managed_remote(&package.sha512));
        }
    }
    if !dry_run {
        crate::identification::preserve_local_sources(instance_dir, &mut identified)?;
    }
    let mut local_entries: Vec<_> = identified
        .iter()
        .map(IdentifiedMod::to_package_entry)
        .collect();
    let mut discovered_remotes =
        std::collections::HashMap::<String, Vec<crate::manifest::PackageRemote>>::new();
    for entry in &local_entries {
        discovered_remotes
            .entry(entry.mod_id.clone())
            .or_default()
            .extend(entry.remotes.iter().cloned());
    }
    for remotes in discovered_remotes.values_mut() {
        remotes.sort();
        remotes.dedup();
    }
    for entry in &mut local_entries {
        entry.remotes = discovered_remotes[&entry.mod_id].clone();
        if let Some(requirement) = manifest.inner.dependencies.get(&entry.mod_id) {
            let superseded = superseded_managed_remotes.get(&entry.mod_id);
            entry.remotes.extend(
                requirement
                    .remotes
                    .iter()
                    .filter(|remote| !superseded.is_some_and(|items| items.contains(*remote)))
                    .cloned(),
            );
            entry.remotes.sort();
            entry.remotes.dedup();
        }
    }

    let InstallInteraction {
        select_package: _,
        select_resolution,
        confirm_install,
        progress: _,
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
        let requirement = manifest
            .inner
            .dependencies
            .entry(entry.mod_id.clone())
            .or_insert_with(|| DependencySpec {
                version: entry.version.clone(),
                optional: false,
                env: None,
                exclude: Vec::new(),
                remotes: entry.remotes.clone(),
            });
        requirement.remotes = entry.remotes.clone();
    }
    lockfile.inner.packages = selected_entries;
    lockfile
        .inner
        .packages
        .sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    manifest.save()?;
    lockfile.save()?;
    if let Err(error) =
        crate::source_store::prune_unreferenced(instance_dir, &manifest.inner, &lockfile.inner)
    {
        report.warnings.push(format!(
            "could not prune unreferenced managed local package sources: {error}"
        ));
    }
    Ok(report)
}

fn describe_platform_changes(
    manifest: &crate::manifest::OrbitManifest,
    discovered: &crate::platform_detection::DiscoveredPlatform,
    snapshot: &crate::manifest::PlatformSnapshot,
) -> Vec<PlatformChange> {
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
        discovered.loader.as_str(),
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
        &snapshot.minecraft_jar.path,
    );
    push_platform_change(
        &mut changes,
        "loader_jar",
        &manifest.platform.loader_jar.path,
        &snapshot.loader_jar.path,
    );
    if manifest.platform.minecraft_jar.sha256 != snapshot.minecraft_jar.sha256 {
        changes.push(PlatformChange {
            field: "minecraft_jar_content",
            previous: "recorded snapshot".to_string(),
            current: "changed on disk".to_string(),
        });
    }
    if manifest.platform.loader_jar.sha256 != snapshot.loader_jar.sha256 {
        changes.push(PlatformChange {
            field: "loader_jar_content",
            previous: "recorded snapshot".to_string(),
            current: "changed on disk".to_string(),
        });
    }
    let previous_runtime = runtime_paths(&manifest.platform.runtime_jars);
    let current_runtime = runtime_paths(&snapshot.runtime_jars);
    if previous_runtime != current_runtime {
        changes.push(PlatformChange {
            field: "runtime_jars",
            previous: previous_runtime.join(", "),
            current: current_runtime.join(", "),
        });
    } else if manifest.platform.runtime_jars != snapshot.runtime_jars {
        changes.push(PlatformChange {
            field: "runtime_jars_content",
            previous: "recorded snapshot".to_string(),
            current: "changed on disk".to_string(),
        });
    }
    if manifest.platform.physical_environment != snapshot.physical_environment {
        changes.push(PlatformChange {
            field: "physical_environment",
            previous: environment_name(manifest.platform.physical_environment).to_string(),
            current: environment_name(snapshot.physical_environment).to_string(),
        });
    }
    changes
}

fn runtime_paths(artifacts: &[crate::manifest::PlatformArtifact]) -> Vec<String> {
    if artifacts.is_empty() {
        return vec!["(none)".to_string()];
    }
    artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect()
}

fn environment_name(environment: crate::metadata::Environment) -> &'static str {
    match environment {
        crate::metadata::Environment::Client => "client",
        crate::metadata::Environment::Server => "server",
        crate::metadata::Environment::Both => "unknown",
    }
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
    use crate::lockfile::{ArtifactSource, OrbitLockfile, PackageEntry};
    use crate::manifest::{OrbitManifest, PackageRemote, ProjectMeta, ResolverConfig};
    use crate::providers::{
        ArtifactDownloadClient, ArtifactFingerprint, ModInfo, ModProvider, ModrinthResolvedInfo,
        RemoteArtifact, SearchResultItem,
    };
    use async_trait::async_trait;
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

    struct IdentificationProvider {
        downloader: ArtifactDownloadClient,
        artifacts: Vec<RemoteArtifact>,
    }

    #[async_trait]
    impl ModProvider for IdentificationProvider {
        fn name(&self) -> &'static str {
            "modrinth"
        }

        fn artifact_downloader(&self) -> &ArtifactDownloadClient {
            &self.downloader
        }

        async fn search(
            &self,
            _query: &str,
            _mc_version: Option<&str>,
            _loader: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<SearchResultItem>, OrbitError> {
            Ok(Vec::new())
        }

        async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError> {
            Err(OrbitError::ModNotFound(slug.to_string()))
        }

        async fn identify_artifacts(
            &self,
            _artifacts: &[ArtifactFingerprint],
        ) -> Result<Vec<RemoteArtifact>, OrbitError> {
            Ok(self.artifacts.clone())
        }

        async fn get_versions(
            &self,
            slug: &str,
            _mc_version: Option<&str>,
            _loader: Option<&str>,
        ) -> Result<Vec<RemoteArtifact>, OrbitError> {
            Err(OrbitError::ModNotFound(slug.to_string()))
        }
    }

    #[tokio::test]
    async fn provider_identification_replaces_the_exact_managed_fallback_and_prunes_its_copy() {
        let directory = test_dir("identify-remotes");
        crate::platform_detection::test_support::write_platform(
            &directory, "1.20.1", "fabric", "0.16.10",
        );
        let mods = directory.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        write_fabric_jar(&mods.join("alpha.jar"), "1");
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
runtime_jars = []
physical_environment = "client"
[dependencies]
"#,
        )
        .unwrap();
        ManifestFile::new(&directory, manifest).save().unwrap();

        sync_instance(&directory, &[], false, InstallInteraction::default())
            .await
            .unwrap();
        let local_lock = Lockfile::open(&directory).unwrap();
        let local = local_lock.inner.find("alpha").unwrap();
        let sha512 = local.sha512.clone();
        let managed = directory
            .join(".orbit")
            .join("sources")
            .join(format!("{sha512}.jar"));
        assert!(managed.is_file());

        let provider: Box<dyn ModProvider> = Box::new(IdentificationProvider {
            downloader: ArtifactDownloadClient::anonymous("orbit-sync-test").unwrap(),
            artifacts: vec![RemoteArtifact {
                sha1: local.sha1.clone(),
                sha512,
                slug: "alpha".to_string(),
                provider: "modrinth".to_string(),
                modrinth: Some(ModrinthResolvedInfo {
                    project_id: "alpha-project".to_string(),
                    version_id: "alpha-version".to_string(),
                }),
                curseforge: None,
                download_url: "https://cdn.modrinth.com/data/alpha/versions/one/alpha.jar"
                    .to_string(),
                filename: "alpha.jar".to_string(),
                related_projects: Vec::new(),
            }],
        });

        sync_instance(
            &directory,
            &[provider],
            false,
            InstallInteraction::default(),
        )
        .await
        .unwrap();

        let manifest = ManifestFile::open(&directory).unwrap();
        assert_eq!(
            manifest.inner.dependencies["alpha"].remotes,
            vec![PackageRemote::Modrinth {
                project_id: "alpha-project".to_string()
            }]
        );
        let lock = Lockfile::open(&directory).unwrap();
        let alpha = lock.inner.find("alpha").unwrap();
        assert_eq!(alpha.remotes, manifest.inner.dependencies["alpha"].remotes);
        assert!(matches!(
            alpha.artifact_sources.as_slice(),
            [ArtifactSource::Modrinth { project_id, version_id, .. }]
                if project_id == "alpha-project" && version_id == "alpha-version"
        ));
        assert!(!managed.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn reports_missing_and_unlocked_manifest_dependencies() {
        let directory = test_dir("states");
        std::fs::create_dir_all(&directory).unwrap();
        crate::platform_detection::test_support::write_platform(&directory, "1", "fabric", "1");
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
            platform: crate::manifest::PlatformSnapshot {
                minecraft_jar: crate::manifest::PlatformArtifact {
                    path: "minecraft.jar".to_string(),
                    sha256: "test".to_string(),
                },
                loader_jar: crate::manifest::PlatformArtifact {
                    path: "loader.jar".to_string(),
                    sha256: "test".to_string(),
                },
                runtime_jars: Vec::new(),
                physical_environment: crate::metadata::Environment::Client,
            },
            resolver: ResolverConfig::default(),
            dependencies: indexmap::IndexMap::from([
                (
                    "missing".to_string(),
                    DependencySpec::new(
                        "*",
                        vec![crate::manifest::PackageRemote::File {
                            path: "sources/missing.jar".to_string(),
                        }],
                    ),
                ),
                (
                    "unlocked".to_string(),
                    DependencySpec::new(
                        "*",
                        vec![crate::manifest::PackageRemote::File {
                            path: "sources/unlocked.jar".to_string(),
                        }],
                    ),
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
                    sha512: "content-identity".to_string(),
                    filename: "missing.jar".to_string(),
                    remotes: vec![PackageRemote::File {
                        path: "mods/missing.jar".to_string(),
                    }],
                    artifact_sources: vec![ArtifactSource::File {
                        path: "mods/missing.jar".to_string(),
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
        crate::platform_detection::test_support::write_platform(
            &directory, "1.20.1", "fabric", "0.16.10",
        );
        let mods = directory.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        write_fabric_jar(&mods.join("a-1.jar"), "1");
        write_fabric_jar(&mods.join("a-2.jar"), "2");
        assert_eq!(
            crate::jar::read_mod_metadata(
                &mods.join("a-1.jar"),
                crate::loader::LoaderKind::Fabric,
            )
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
runtime_jars = []
physical_environment = "client"
[dependencies]
alpha = { version = "*", remotes = [{ type = "file", path = "alpha.jar" }] }
"#,
        )
        .unwrap();
        ManifestFile::new(&directory, manifest).save().unwrap();
        let scanned =
            crate::init::scan_mods_dir(&directory, crate::loader::LoaderKind::Fabric).unwrap();
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
                progress: None,
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
                progress: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(applied.removed[0].filename, "a-1.jar");
        assert!(!mods.join("a-1.jar").exists());
        assert!(mods.join("a-2.jar").exists());
        let refreshed = Lockfile::open(&directory).unwrap();
        let alpha = refreshed.inner.find("alpha").unwrap();
        assert_eq!(alpha.version, "2");
        assert_eq!(alpha.remotes.len(), 3);
        assert_eq!(
            ManifestFile::open(&directory).unwrap().inner.dependencies["alpha"].env(),
            None
        );
        assert_eq!(
            alpha
                .remotes
                .iter()
                .filter(|remote| { remote.display_locator() == "file:managed local source" })
                .count(),
            2
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn refreshes_loader_version_and_artifact_paths_from_a_fresh_scan() {
        let directory = test_dir("platform-refresh");
        crate::platform_detection::test_support::write_platform(
            &directory, "1.20.1", "fabric", "0.17.0",
        );
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
            platform: crate::manifest::PlatformSnapshot {
                minecraft_jar: crate::manifest::PlatformArtifact {
                    path: "old-minecraft.jar".to_string(),
                    sha256: "old".to_string(),
                },
                loader_jar: crate::manifest::PlatformArtifact {
                    path: "old-loader.jar".to_string(),
                    sha256: "old".to_string(),
                },
                runtime_jars: Vec::new(),
                physical_environment: crate::metadata::Environment::Client,
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

    #[tokio::test]
    async fn rediscovers_renamed_platform_jars_instead_of_following_manifest_paths() {
        let directory = test_dir("renamed-platform-files");
        crate::platform_detection::test_support::write_platform(
            &directory, "1.20.1", "fabric", "0.17.0",
        );
        let minecraft_jar = directory.join("1.20.1.jar");
        let renamed_minecraft_jar = directory.join("launcher-client-current.jar");
        std::fs::rename(&minecraft_jar, &renamed_minecraft_jar).unwrap();
        let loader_directory = directory
            .join("libraries")
            .join("net")
            .join("fabricmc")
            .join("fabric-loader")
            .join("0.17.0");
        let loader_jar = loader_directory.join("fabric-loader-0.17.0.jar");
        let renamed_loader_jar = loader_directory.join("launcher-loader-current.jar");
        std::fs::rename(&loader_jar, &renamed_loader_jar).unwrap();

        let manifest = OrbitManifest {
            project: ProjectMeta {
                name: "test".to_string(),
                mc_version: "1.20.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.17.0".to_string(),
                description: None,
                authors: None,
                version: None,
            },
            platform: crate::manifest::PlatformSnapshot {
                minecraft_jar: crate::manifest::PlatformArtifact {
                    path: "deleted-client-name.jar".to_string(),
                    sha256: "stale".to_string(),
                },
                loader_jar: crate::manifest::PlatformArtifact {
                    path: "deleted-loader-name.jar".to_string(),
                    sha256: "stale".to_string(),
                },
                runtime_jars: Vec::new(),
                physical_environment: crate::metadata::Environment::Client,
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

        assert!(report.platform_changes.iter().any(|change| {
            change.field == "minecraft_jar"
                && change.current.ends_with("launcher-client-current.jar")
        }));
        assert!(report.platform_changes.iter().any(|change| {
            change.field == "loader_jar" && change.current.ends_with("launcher-loader-current.jar")
        }));
        assert!(
            refreshed
                .inner
                .platform
                .minecraft_jar
                .path
                .ends_with("launcher-client-current.jar")
        );
        assert!(
            refreshed
                .inner
                .platform
                .loader_jar
                .path
                .ends_with("launcher-loader-current.jar")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
