//! Cross-runtime migration planning and export.
//!
//! A migration target is an already installed game instance. This keeps the
//! package manager independent from launcher metadata downloads while still
//! allowing it to build an exact target `orbit.toml` platform snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{LockMeta, OrbitLockfile};
use crate::progress::{ProgressEvent, ProgressReporter, emit as emit_progress};
use crate::providers::ModProvider;
use crate::resolver::types::{CandidateDiagnostic, PackageChange, PackageChangeKind};
use crate::workspace::{Lockfile, ManifestFile};

#[derive(Default)]
pub struct MigrationInteraction {
    pub select_resolution: Option<crate::resolver::types::ResolutionSelector>,
    pub progress: Option<ProgressReporter>,
}

#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub source_mc_version: String,
    pub target_mc_version: String,
    pub target_loader: String,
    pub target_loader_version: String,
    pub changes: Vec<PackageChange>,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
    pub selected_packages: usize,
    source_dir: PathBuf,
    target_dir: PathBuf,
    target_manifest: crate::manifest::OrbitManifest,
    target_lockfile: OrbitLockfile,
    _source_owner: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl MigrationPlan {
    pub fn target_dir(&self) -> &Path {
        &self.target_dir
    }

    pub fn target_manifest(&self) -> &crate::manifest::OrbitManifest {
        &self.target_manifest
    }

    pub fn target_lockfile(&self) -> &OrbitLockfile {
        &self.target_lockfile
    }
}

#[derive(Debug, Clone)]
pub struct MigrationExportReport {
    pub target_dir: PathBuf,
    pub packages: usize,
    pub config_files: usize,
    pub config_bytes: u64,
}

/// Build the one authoritative migration plan used by both `migrate check`
/// and `migrate export`.
pub async fn plan_migration(
    source_dir: &Path,
    target_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    interaction: MigrationInteraction,
) -> Result<MigrationPlan, OrbitError> {
    let source_dir = canonical_directory(source_dir, "source instance")?;
    let target_dir = canonical_directory(target_dir, "target instance")?;
    if source_dir == target_dir {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "migration source and target must be different instances"
        )));
    }

    let source_manifest = ManifestFile::open(&source_dir)?;
    let source_lock = Lockfile::open(&source_dir)?;
    let target_platform = crate::platform_detection::rediscover_current_platform(&target_dir)?;
    let target_snapshot = target_platform.snapshot(&target_dir)?;
    let target_meta = LockMeta {
        mc_version: target_platform.minecraft_version.id.clone(),
        modloader: target_platform.loader.to_string(),
        modloader_version: target_platform.loader_version.clone(),
    };
    let empty_target_lock = OrbitLockfile {
        meta: target_meta.clone(),
        packages: Vec::new(),
    };
    let mut target_manifest = source_manifest.inner.clone();
    crate::platform_detection::apply_to_manifest(
        &mut target_manifest,
        &target_platform,
        target_snapshot,
    );

    let manifest_remotes: Vec<_> = source_manifest
        .inner
        .dependencies
        .values()
        .flat_map(|dependency| dependency.remotes.iter().cloned())
        .collect();
    let mut catalog = crate::outdated::download_candidate_catalog(
        crate::outdated::CandidateDiscoveryInput {
            instance_dir: &source_dir,
            providers,
            additional_remotes: &manifest_remotes,
            lockfile: &source_lock.inner,
            mc_version: &target_meta.mc_version,
            loader: target_platform.loader,
            jar_cache,
            progress: interaction.progress.clone(),
        },
        &[],
    )
    .await?;
    catalog.loader_package = target_platform.loader_package;

    emit_progress(
        interaction.progress.as_ref(),
        ProgressEvent::ResolutionStarted {
            packages: catalog.candidates.len(),
            candidates: catalog.candidates.values().map(Vec::len).sum(),
        },
    );
    let portfolio = crate::resolver::resolve_candidate_portfolio_with_progress(
        &target_manifest,
        &empty_target_lock,
        &catalog,
        interaction.progress.clone(),
    )
    .await
    .map_err(|error| OrbitError::Conflict(error.to_string()))?;
    emit_progress(
        interaction.progress.as_ref(),
        ProgressEvent::ResolutionFinished {
            solutions: portfolio.alternatives.len(),
        },
    );
    let resolution = crate::resolver::select_resolution(portfolio, interaction.select_resolution)
        .map_err(OrbitError::Conflict)?;

    let mut packages = Vec::with_capacity(resolution.selected_candidates.len());
    for (package, candidate_id) in &resolution.selected_candidates {
        let version = resolution.selected_versions.get(package).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "migration solution omitted the selected version for '{package}'"
            ))
        })?;
        let artifact = catalog.resolved.get(candidate_id).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "migration solution selected '{package}' without a materializable artifact"
            ))
        })?;
        let candidate = catalog
            .candidates
            .get(package)
            .and_then(|candidates| {
                candidates
                    .iter()
                    .find(|candidate| candidate.id == *candidate_id)
            })
            .ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "migration solution selected unknown candidate metadata for '{package}'"
                ))
            })?;
        let remotes = crate::installer::package_remotes(
            package,
            &source_manifest.inner,
            &source_lock.inner,
            &catalog,
        );
        packages.push(crate::installer::lock_entry_from_candidate(
            package,
            version,
            artifact,
            remotes,
            Some(candidate),
        ));
    }
    packages.sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    let mut target_lockfile = OrbitLockfile {
        meta: target_meta.clone(),
        packages,
    };
    crate::installer::normalize_selected_file_remotes(&mut target_lockfile);
    crate::installer::reconcile_manifest_to_lock(&mut target_manifest, &target_lockfile);
    target_manifest.validate()?;
    target_lockfile.validate()?;

    let changes = migration_changes(
        &source_lock.inner,
        &target_lockfile,
        target_platform.loader,
        &catalog,
        &resolution.selected_candidates,
    );
    Ok(MigrationPlan {
        source_mc_version: source_manifest.inner.project.mc_version,
        target_mc_version: target_meta.mc_version,
        target_loader: target_meta.modloader,
        target_loader_version: target_meta.modloader_version,
        changes,
        diagnostics: resolution.diagnostics,
        warnings: resolution.warnings,
        selected_packages: target_lockfile.packages.len(),
        source_dir,
        target_dir,
        target_manifest,
        target_lockfile,
        _source_owner: None,
    })
}

pub async fn plan_migration_from_portable(
    source: crate::archive::PortableInstance,
    target_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    interaction: MigrationInteraction,
) -> Result<MigrationPlan, OrbitError> {
    let owner = source.owner();
    let mut plan =
        plan_migration(source.path(), target_dir, providers, jar_cache, interaction).await?;
    plan._source_owner = Some(owner);
    Ok(plan)
}

/// Export a previously selected plan into the installed target instance.
/// Package JARs are deliberately not copied; `orbit install` owns exact
/// materialization from the emitted lockfile.
pub fn export_migration(
    plan: &MigrationPlan,
    dry_run: bool,
) -> Result<MigrationExportReport, OrbitError> {
    validate_target_is_unchanged(plan)?;
    let config_sources = crate::archive::portable_config_sources(&plan.source_dir)?;
    let config_files = config_sources.len();
    let config_bytes = config_sources.iter().map(|source| source.bytes).sum();
    let report = MigrationExportReport {
        target_dir: plan.target_dir.clone(),
        packages: plan.target_lockfile.packages.len(),
        config_files,
        config_bytes,
    };
    if dry_run {
        return Ok(report);
    }

    let mut destinations = vec![
        plan.target_dir.join("orbit.toml"),
        plan.target_dir.join("orbit.lock"),
    ];
    destinations.extend(
        config_sources
            .iter()
            .map(|source| plan.target_dir.join(&source.relative)),
    );
    if let Some(existing) = destinations.iter().find(|path| path.exists()) {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "migration export refuses to overwrite existing target content: {}",
            existing.display()
        )));
    }

    let staging = plan
        .target_dir
        .join(format!(".orbit-migration-staging-{}", std::process::id()));
    if staging.exists() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "migration staging directory already exists: {}",
            staging.display()
        )));
    }
    std::fs::create_dir(&staging)?;
    let result = stage_and_commit(plan, &config_sources, &staging);
    if result.is_err() && staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result?;
    Ok(report)
}

fn migration_changes(
    source: &OrbitLockfile,
    target: &OrbitLockfile,
    loader: crate::loader::LoaderKind,
    catalog: &crate::resolver::types::CandidateCatalog,
    selected_candidates: &BTreeMap<String, String>,
) -> Vec<PackageChange> {
    let mut packages = BTreeSet::new();
    packages.extend(source.packages.iter().map(|entry| entry.mod_id.clone()));
    packages.extend(target.packages.iter().map(|entry| entry.mod_id.clone()));
    let mut changes = Vec::new();
    for package in packages {
        let current = source.find(&package);
        let selected = target.find(&package);
        if current.is_some_and(|current| {
            selected.is_some_and(|selected| current.sha512 == selected.sha512)
        }) {
            continue;
        }
        let kind = match (current, selected) {
            (None, Some(_)) => PackageChangeKind::Install,
            (Some(_), None) => PackageChangeKind::Remove,
            (Some(current), Some(selected)) => {
                let current_version = crate::versions::Version::parse(&current.version, loader);
                let selected_version = crate::versions::Version::parse(&selected.version, loader);
                if selected_version > current_version {
                    PackageChangeKind::Upgrade
                } else if selected_version < current_version {
                    PackageChangeKind::Downgrade
                } else {
                    PackageChangeKind::Replace
                }
            }
            (None, None) => continue,
        };
        let selected_description = selected_candidates
            .get(&package)
            .and_then(|candidate_id| {
                catalog.candidates.get(&package).and_then(|candidates| {
                    candidates
                        .iter()
                        .find(|candidate| candidate.id == *candidate_id)
                })
            })
            .map(|candidate| candidate.display_description());
        changes.push(PackageChange {
            package,
            current_version: current.map(|entry| entry.version.clone()),
            selected_version: selected.map(|entry| entry.version.clone()),
            filename: None,
            selected_filename: None,
            selected_description,
            kind,
        });
    }
    changes
}

fn validate_target_is_unchanged(plan: &MigrationPlan) -> Result<(), OrbitError> {
    let current = crate::platform_detection::rediscover_current_platform(&plan.target_dir)?;
    let snapshot = current.snapshot(&plan.target_dir)?;
    if current.minecraft_version.id != plan.target_manifest.project.mc_version
        || current.loader.as_str() != plan.target_manifest.project.modloader
        || current.loader_version != plan.target_manifest.project.modloader_version
        || snapshot != plan.target_manifest.platform
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "target runtime changed after migration planning; run migration planning again"
        )));
    }
    Ok(())
}

fn stage_and_commit(
    plan: &MigrationPlan,
    config_sources: &[crate::archive::PortableFile],
    staging: &Path,
) -> Result<(), OrbitError> {
    ManifestFile::new(staging, plan.target_manifest.clone()).save()?;
    Lockfile::new(staging, plan.target_lockfile.clone()).save()?;
    for source in config_sources {
        let destination = staging.join(&source.relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source.source, destination)?;
    }

    let mut roots = BTreeSet::new();
    for source in config_sources {
        if let Some(root) = source.relative.components().next() {
            roots.insert(PathBuf::from(root.as_os_str()));
        }
    }
    let mut committed = Vec::new();
    for relative in roots
        .into_iter()
        .chain([PathBuf::from("orbit.toml"), PathBuf::from("orbit.lock")])
    {
        let source = staging.join(&relative);
        if !source.exists() {
            continue;
        }
        let destination = plan.target_dir.join(&relative);
        if let Err(error) = std::fs::rename(&source, &destination) {
            for previous in committed.iter().rev() {
                let _ = std::fs::rename(plan.target_dir.join(previous), staging.join(previous));
            }
            return Err(OrbitError::Io(error));
        }
        committed.push(relative);
    }
    std::fs::remove_dir(staging)?;
    Ok(())
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, OrbitError> {
    let canonical = path.canonicalize().map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "cannot resolve {label} '{}': {error}",
            path.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "{label} is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{OrbitManifest, ProjectMeta, ResolverConfig};

    fn write_empty_orbit_instance(directory: &Path, minecraft: &str, loader_version: &str) {
        crate::platform_detection::test_support::write_platform(
            directory,
            minecraft,
            "fabric",
            loader_version,
        );
        let discovered = crate::platform_detection::discover_platform_for_init(
            directory,
            minecraft,
            "fabric",
            loader_version,
        )
        .unwrap();
        let manifest = OrbitManifest {
            project: ProjectMeta {
                name: "migration-test".to_string(),
                mc_version: minecraft.to_string(),
                modloader: "fabric".to_string(),
                modloader_version: loader_version.to_string(),
                description: None,
                authors: None,
                version: None,
            },
            platform: discovered.snapshot(directory).unwrap(),
            resolver: ResolverConfig::default(),
            dependencies: Default::default(),
            groups: Default::default(),
            overrides: Default::default(),
        };
        ManifestFile::new(directory, manifest).save().unwrap();
        Lockfile::new(
            directory,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: minecraft.to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: loader_version.to_string(),
                },
                packages: Vec::new(),
            },
        )
        .save()
        .unwrap();
    }

    #[tokio::test]
    async fn check_and_export_share_an_exact_target_runtime_plan() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        write_empty_orbit_instance(&source, "1", "1");
        crate::platform_detection::test_support::write_platform(&target, "2", "fabric", "2");
        std::fs::create_dir_all(source.join("config")).unwrap();
        std::fs::write(source.join("config/example.toml"), "enabled = true\n").unwrap();

        let cache = crate::jar_cache::JarCache::open(root.path().join("cache")).unwrap();
        let pack = root.path().join("source.zip");
        crate::archive::export_instance(&source, &pack, None, "zip", false, None).unwrap();
        let portable = crate::archive::extract_portable_instance(&pack).unwrap();
        let plan = plan_migration_from_portable(
            portable,
            &target,
            &[],
            &cache,
            MigrationInteraction::default(),
        )
        .await
        .unwrap();

        assert_eq!(plan.source_mc_version, "1");
        assert_eq!(plan.target_mc_version, "2");
        assert_eq!(plan.selected_packages, 0);
        let report = export_migration(&plan, false).unwrap();
        assert_eq!(report.config_files, 1);
        assert!(target.join("orbit.toml").is_file());
        assert!(target.join("orbit.lock").is_file());
        assert_eq!(
            std::fs::read_to_string(target.join("config/example.toml")).unwrap(),
            "enabled = true\n"
        );
        assert_eq!(
            ManifestFile::open(&target)
                .unwrap()
                .inner
                .project
                .mc_version,
            "2"
        );
    }

    #[tokio::test]
    async fn export_refuses_to_overwrite_target_configuration() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        write_empty_orbit_instance(&source, "1", "1");
        crate::platform_detection::test_support::write_platform(&target, "2", "fabric", "2");
        std::fs::create_dir_all(source.join("config")).unwrap();
        std::fs::write(source.join("config/example.toml"), "source").unwrap();
        std::fs::create_dir_all(target.join("config")).unwrap();
        std::fs::write(target.join("config/example.toml"), "target").unwrap();

        let cache = crate::jar_cache::JarCache::open(root.path().join("cache")).unwrap();
        let plan = plan_migration(
            &source,
            &target,
            &[],
            &cache,
            MigrationInteraction::default(),
        )
        .await
        .unwrap();
        let error = export_migration(&plan, false).unwrap_err();

        assert!(error.to_string().contains("refuses to overwrite"));
        assert_eq!(
            std::fs::read_to_string(target.join("config/example.toml")).unwrap(),
            "target"
        );
        assert!(!target.join("orbit.toml").exists());
    }
}
