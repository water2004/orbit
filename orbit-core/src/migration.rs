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

pub type MigrationFallbackConfirmation =
    Box<dyn FnOnce(&MigrationFallbackPrompt) -> Result<bool, String> + Send>;

#[derive(Default)]
pub struct MigrationInteraction {
    pub select_resolution: Option<crate::resolver::types::ResolutionSelector>,
    pub confirm_soft_fallback: Option<MigrationFallbackConfirmation>,
    pub progress: Option<ProgressReporter>,
}

#[derive(Debug, Clone, Default)]
pub struct MigrationOptions {
    /// The caller has already consented to package removal, so the package-
    /// preserving Pareto search may run without an additional interaction.
    pub allow_package_removals: bool,
}

#[derive(Debug, Clone)]
pub struct MigrationFallbackPrompt {
    pub strict_failure: String,
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
    local_sources: Vec<MigrationLocalSource>,
    _source_owner: Option<std::sync::Arc<tempfile::TempDir>>,
}

#[derive(Debug, Clone)]
struct MigrationLocalSource {
    source: PathBuf,
    relative: PathBuf,
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
    options: MigrationOptions,
    interaction: MigrationInteraction,
) -> Result<MigrationPlan, OrbitError> {
    plan_migration_inner(
        source_dir,
        target_dir,
        providers,
        jar_cache,
        options,
        interaction,
        false,
    )
    .await
}

async fn plan_migration_inner(
    source_dir: &Path,
    target_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    options: MigrationOptions,
    interaction: MigrationInteraction,
    portable_snapshot: bool,
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

    let (manifest_remotes, discovery_lock) = migration_discovery_state(
        &source_manifest.inner,
        &source_lock.inner,
        portable_snapshot,
    );
    let mut catalog = crate::outdated::download_candidate_catalog(
        crate::outdated::CandidateDiscoveryInput {
            instance_dir: &source_dir,
            providers,
            additional_remotes: &manifest_remotes,
            lockfile: &discovery_lock,
            mc_version: &target_meta.mc_version,
            loader: target_platform.loader,
            jar_cache,
            progress: interaction.progress.clone(),
        },
        &[],
    )
    .await?;
    catalog.loader_package = target_platform.loader_package;

    let MigrationInteraction {
        select_resolution,
        confirm_soft_fallback,
        progress,
    } = interaction;

    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionStarted {
            packages: catalog.candidates.len(),
            candidates: catalog.candidates.values().map(Vec::len).sum(),
        },
    );
    let mut portfolio = if options.allow_package_removals {
        crate::resolver::resolve_package_preserving_portfolio_with_progress(
            &target_manifest,
            &empty_target_lock,
            &catalog,
            progress.clone(),
        )
        .await
        .map_err(|error| OrbitError::Conflict(error.to_string()))?
    } else {
        match crate::resolver::resolve_required_package_portfolio_with_progress(
            &target_manifest,
            &empty_target_lock,
            &catalog,
            progress.clone(),
        )
        .await
        {
            Ok(portfolio) => portfolio,
            Err(crate::resolver::ResolutionFailure::Internal(error)) => {
                return Err(OrbitError::Conflict(error));
            }
            Err(crate::resolver::ResolutionFailure::NoSolution(strict_failure)) => {
                let prompt = MigrationFallbackPrompt {
                    strict_failure: strict_failure.clone(),
                };
                let accepted = match confirm_soft_fallback {
                    Some(confirm) => confirm(&prompt).map_err(OrbitError::Conflict)?,
                    None => false,
                };
                if !accepted {
                    return Err(OrbitError::Conflict(format!(
                        "strict migration is unavailable:\n{strict_failure}\n\nSearching for a package-removing migration was declined"
                    )));
                }
                crate::resolver::resolve_package_preserving_portfolio_with_progress(
                    &target_manifest,
                    &empty_target_lock,
                    &catalog,
                    progress.clone(),
                )
                .await
                .map_err(|soft_failure| {
                    OrbitError::Conflict(format!(
                        "strict migration is unavailable:\n{strict_failure}\n\nNo migration is possible even after allowing package removal:\n{soft_failure}"
                    ))
                })?
            }
        }
    };

    attach_migration_changes(
        &mut portfolio,
        &source_manifest.inner,
        &source_lock.inner,
        &target_meta,
        target_platform.loader,
        &catalog,
    )?;

    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionFinished {
            solutions: portfolio.alternatives.len(),
        },
    );
    let resolution = crate::resolver::select_resolution(portfolio, select_resolution)
        .map_err(OrbitError::Conflict)?;

    let mut target_lockfile = migration_lockfile(
        &resolution,
        &source_manifest.inner,
        &source_lock.inner,
        &target_meta,
        &catalog,
    )?;
    let local_sources = prepare_migration_local_sources(&source_dir, &mut target_lockfile)?;
    let diagnostics = migration_diagnostics(&resolution.changes, &resolution.diagnostics);
    crate::installer::reconcile_manifest_to_lock(&mut target_manifest, &target_lockfile);
    target_manifest.validate()?;
    target_lockfile.validate()?;

    Ok(MigrationPlan {
        source_mc_version: source_manifest.inner.project.mc_version,
        target_mc_version: target_meta.mc_version,
        target_loader: target_meta.modloader,
        target_loader_version: target_meta.modloader_version,
        changes: resolution.changes,
        diagnostics,
        warnings: resolution.warnings,
        selected_packages: target_lockfile.packages.len(),
        source_dir,
        target_dir,
        target_manifest,
        target_lockfile,
        local_sources,
        _source_owner: None,
    })
}

/// Build the target-version discovery projection of the source package state.
///
/// A portable Orbit archive adds `file = "mods/<installed>.jar"` so the exact
/// source runtime can be restored without a network. That restoration carrier
/// is not another target-version remote. When a package has a real provider
/// project, migration must enumerate that project for the target Minecraft
/// version and must not inject the archived source JAR into the target solver
/// graph. File-only packages keep their sole source and are still checked by
/// the normal JAR-declared Minecraft constraints in PubGrub.
fn migration_discovery_state(
    manifest: &crate::manifest::OrbitManifest,
    lockfile: &OrbitLockfile,
    portable_snapshot: bool,
) -> (Vec<crate::manifest::PackageRemote>, OrbitLockfile) {
    use crate::manifest::PackageRemote;

    let mut package_remotes = BTreeMap::<String, Vec<PackageRemote>>::new();
    for (package, specification) in &manifest.packages {
        package_remotes
            .entry(package.clone())
            .or_default()
            .extend(specification.remotes.iter().cloned());
    }
    for entry in &lockfile.packages {
        let remotes = package_remotes.entry(entry.mod_id.clone()).or_default();
        remotes.extend(entry.remotes.iter().cloned());
        remotes.extend(entry.artifact_sources.iter().map(|source| match source {
            crate::lockfile::ArtifactSource::File { path } => {
                PackageRemote::File { path: path.clone() }
            }
            crate::lockfile::ArtifactSource::Modrinth { project_id, .. } => {
                PackageRemote::Modrinth {
                    project_id: project_id.clone(),
                }
            }
            crate::lockfile::ArtifactSource::Curseforge { project_id, .. } => {
                PackageRemote::Curseforge {
                    project_id: *project_id,
                }
            }
        }));
    }

    let portable_carriers: BTreeMap<_, _> = lockfile
        .packages
        .iter()
        .map(|entry| {
            (
                entry.mod_id.clone(),
                format!("mods/{}", entry.filename).replace('\\', "/"),
            )
        })
        .collect();
    for (package, remotes) in &mut package_remotes {
        remotes.sort();
        remotes.dedup();
        if portable_snapshot && let Some(carrier) = portable_carriers.get(package) {
            let has_online = remotes
                .iter()
                .any(|remote| !matches!(remote, PackageRemote::File { .. }));
            remotes.retain(|remote| {
                !matches!(remote, PackageRemote::File { .. })
                    || (!has_online
                        && matches!(
                            remote,
                            PackageRemote::File { path }
                                if path.replace('\\', "/").eq_ignore_ascii_case(carrier)
                        ))
            });
        }
    }

    let mut projected_lock = lockfile.clone();
    for entry in &mut projected_lock.packages {
        entry.remotes = package_remotes
            .get(&entry.mod_id)
            .cloned()
            .unwrap_or_default();
    }
    let mut remotes: Vec<_> = package_remotes.into_values().flatten().collect();
    remotes.sort();
    remotes.dedup();
    (remotes, projected_lock)
}

/// Keep only diagnostics that explain an actual package removal in the chosen
/// migration.
///
/// Resolver diagnostics describe why *unselected candidates* lost. During a
/// cross-version migration that naturally includes source-runtime JARs and
/// their bundled modules being rejected by the target Minecraft version. A
/// successful replacement already explains that outcome in `changes`; showing
/// every rejected dependency as a migration error is both redundant and
/// misleading. A removed top-level package is different: its rejection is the
/// user-visible reason the soft migration had to drop it.
fn migration_diagnostics(
    changes: &[PackageChange],
    diagnostics: &[CandidateDiagnostic],
) -> Vec<CandidateDiagnostic> {
    let removed: BTreeSet<_> = changes
        .iter()
        .filter(|change| change.kind == PackageChangeKind::Remove)
        .map(|change| change.package.as_str())
        .collect();
    diagnostics
        .iter()
        .filter(|diagnostic| removed.contains(diagnostic.package.as_str()))
        .cloned()
        .collect()
}

pub async fn plan_migration_from_portable(
    source: crate::archive::PortableInstance,
    target_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    options: MigrationOptions,
    interaction: MigrationInteraction,
) -> Result<MigrationPlan, OrbitError> {
    let owner = source.owner();
    let mut plan = plan_migration_inner(
        source.path(),
        target_dir,
        providers,
        jar_cache,
        options,
        interaction,
        true,
    )
    .await?;
    plan._source_owner = Some(owner);
    Ok(plan)
}

/// Export a previously selected plan into the installed target instance.
/// Runtime JARs are not installed here; selected file-only artifacts are
/// preserved in the target source store so `orbit install` can materialize the
/// exact lock after the temporary migration snapshot is gone.
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
    destinations.extend(
        plan.local_sources
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

fn attach_migration_changes(
    portfolio: &mut crate::resolver::types::ResolutionPortfolio,
    source_manifest: &crate::manifest::OrbitManifest,
    source_lock: &OrbitLockfile,
    target_meta: &LockMeta,
    loader: crate::loader::LoaderKind,
    catalog: &crate::resolver::types::CandidateCatalog,
) -> Result<(), OrbitError> {
    for resolution in &mut portfolio.alternatives {
        let target_lock = migration_lockfile(
            resolution,
            source_manifest,
            source_lock,
            target_meta,
            catalog,
        )?;
        resolution.changes = migration_changes(
            source_lock,
            &target_lock,
            loader,
            catalog,
            &resolution.selected_candidates,
        );
    }
    Ok(())
}

fn migration_lockfile(
    resolution: &crate::resolver::types::ResolutionReport,
    source_manifest: &crate::manifest::OrbitManifest,
    source_lock: &OrbitLockfile,
    target_meta: &LockMeta,
    catalog: &crate::resolver::types::CandidateCatalog,
) -> Result<OrbitLockfile, OrbitError> {
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
        let remotes =
            crate::installer::package_remotes(package, source_manifest, source_lock, catalog);
        packages.push(crate::installer::lock_entry_from_candidate(
            package,
            version,
            artifact,
            remotes,
            Some(candidate),
        ));
    }
    packages.sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    let mut lockfile = OrbitLockfile {
        meta: target_meta.clone(),
        packages,
    };
    crate::installer::normalize_selected_file_remotes(&mut lockfile);
    Ok(lockfile)
}

fn prepare_migration_local_sources(
    source_dir: &Path,
    lockfile: &mut OrbitLockfile,
) -> Result<Vec<MigrationLocalSource>, OrbitError> {
    use crate::lockfile::ArtifactSource;
    use crate::manifest::PackageRemote;

    let mut sources = BTreeMap::<PathBuf, PathBuf>::new();
    for entry in &mut lockfile.packages {
        let online = entry
            .artifact_sources
            .iter()
            .any(|source| !matches!(source, ArtifactSource::File { .. }));
        if online {
            entry
                .artifact_sources
                .retain(|source| !matches!(source, ArtifactSource::File { .. }));
            entry
                .remotes
                .retain(|remote| !matches!(remote, PackageRemote::File { .. }));
            continue;
        }

        let file_sources: Vec<_> = entry
            .artifact_sources
            .iter()
            .filter_map(|source| match source {
                ArtifactSource::File { path } => Some(PathBuf::from(path)),
                ArtifactSource::Modrinth { .. } | ArtifactSource::Curseforge { .. } => None,
            })
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    source_dir.join(path)
                }
            })
            .collect();
        if file_sources.is_empty() {
            continue;
        }
        let source = file_sources
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "selected local migration source for '{}' does not exist",
                    entry.mod_id
                ))
            })?;
        let actual = crate::jar::compute_sha512(&source)?;
        if !actual.eq_ignore_ascii_case(&entry.sha512) {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "local migration source for '{}' no longer matches the selected content",
                entry.mod_id
            )));
        }

        let managed = crate::source_store::managed_remote(&entry.sha512);
        let PackageRemote::File { path } = &managed else {
            unreachable!("managed local source is always a file remote")
        };
        let relative = PathBuf::from(path);
        sources.entry(relative).or_insert(source);
        entry.artifact_sources = vec![crate::source_store::managed_artifact_source(&managed)];
        entry
            .remotes
            .retain(|remote| !matches!(remote, PackageRemote::File { .. }));
        entry.remotes.push(managed);
        entry.remotes.sort();
        entry.remotes.dedup();
    }
    crate::installer::normalize_selected_file_remotes(lockfile);
    Ok(sources
        .into_iter()
        .map(|(relative, source)| MigrationLocalSource { source, relative })
        .collect())
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
                match selected_version.cmp_precedence(&current_version) {
                    std::cmp::Ordering::Greater => PackageChangeKind::Upgrade,
                    std::cmp::Ordering::Less => PackageChangeKind::Downgrade,
                    std::cmp::Ordering::Equal => PackageChangeKind::Replace,
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
    for source in &plan.local_sources {
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
    for source in &plan.local_sources {
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
    use std::io::Write;

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
            packages: Default::default(),
            groups: Default::default(),
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

    fn write_fabric_package(path: &Path, mod_id: &str, minecraft: &str) {
        let file = std::fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"{{"schemaVersion":1,"id":"{mod_id}","version":"1","name":"{mod_id}","depends":{{"minecraft":"{minecraft}"}}}}"#
        )
        .unwrap();
        archive.finish().unwrap();
    }

    fn diagnostic(package: &str) -> CandidateDiagnostic {
        CandidateDiagnostic {
            package: package.to_string(),
            selected_version: "target".to_string(),
            candidate_version: "source".to_string(),
            kind: crate::resolver::types::CandidateDiagnosticKind::ExcludedByPropagation,
            facts: vec!["requires minecraft 26.2".to_string()],
        }
    }

    #[test]
    fn successful_migration_does_not_report_rejected_source_candidates_as_errors() {
        let changes = vec![PackageChange {
            package: "mod".to_string(),
            current_version: Some("26.2".to_string()),
            selected_version: Some("26.1.2".to_string()),
            filename: None,
            selected_filename: None,
            selected_description: None,
            kind: PackageChangeKind::Replace,
        }];
        let diagnostics = vec![diagnostic("mod"), diagnostic("bundled_dependency")];

        assert!(migration_diagnostics(&changes, &diagnostics).is_empty());
    }

    #[test]
    fn soft_migration_keeps_only_the_removed_package_explanation() {
        let changes = vec![PackageChange {
            package: "removed".to_string(),
            current_version: Some("1".to_string()),
            selected_version: None,
            filename: None,
            selected_filename: None,
            selected_description: None,
            kind: PackageChangeKind::Remove,
        }];
        let diagnostics = vec![diagnostic("removed"), diagnostic("bundled_dependency")];
        let filtered = migration_diagnostics(&changes, &diagnostics);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].package, "removed");
    }

    #[tokio::test]
    async fn migration_uses_target_provider_remotes_instead_of_portable_source_jars() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join("mods")).unwrap();
        write_empty_orbit_instance(&source, "26.2", "1");
        write_fabric_package(&source.join("mods/example.jar"), "example", "~26.2");
        crate::sync_instance(&source, &[], false).await.unwrap();

        let mut manifest = ManifestFile::open(&source).unwrap().inner;
        let provider = crate::manifest::PackageRemote::Modrinth {
            project_id: "provider-project".to_string(),
        };
        let carrier = crate::manifest::PackageRemote::File {
            path: "mods/example.jar".to_string(),
        };
        manifest.packages["example"].remotes = vec![provider.clone(), carrier.clone()];
        let mut lockfile = Lockfile::open(&source).unwrap().inner;
        lockfile.packages[0].remotes = vec![provider.clone(), carrier];
        lockfile.packages[0].artifact_sources = vec![
            crate::lockfile::ArtifactSource::Modrinth {
                project_id: "provider-project".to_string(),
                version_id: "source-version".to_string(),
                download_url: "https://example.invalid/source.jar".to_string(),
            },
            crate::lockfile::ArtifactSource::File {
                path: "mods/example.jar".to_string(),
            },
        ];

        let (remotes, projected_lock) = migration_discovery_state(&manifest, &lockfile, true);

        assert_eq!(remotes, vec![provider]);
        assert!(
            projected_lock.packages[0]
                .remotes
                .iter()
                .all(|remote| !matches!(remote, crate::manifest::PackageRemote::File { .. }))
        );
    }

    #[tokio::test]
    async fn migration_keeps_a_file_only_package_for_solver_compatibility_checking() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join("mods")).unwrap();
        write_empty_orbit_instance(&source, "26.2", "1");
        write_fabric_package(&source.join("mods/local.jar"), "local", ">=26.1");
        crate::sync_instance(&source, &[], false).await.unwrap();

        let mut manifest = ManifestFile::open(&source).unwrap().inner;
        manifest.packages["local"].remotes = vec![crate::manifest::PackageRemote::File {
            path: "mods/local.jar".to_string(),
        }];
        let mut lockfile = Lockfile::open(&source).unwrap().inner;
        lockfile.packages[0].remotes = manifest.packages["local"].remotes.clone();
        lockfile.packages[0].artifact_sources = vec![crate::lockfile::ArtifactSource::File {
            path: "mods/local.jar".to_string(),
        }];
        let (remotes, projected_lock) = migration_discovery_state(&manifest, &lockfile, true);

        assert!(
            remotes
                .iter()
                .all(|remote| matches!(remote, crate::manifest::PackageRemote::File { .. }))
        );
        assert!(
            projected_lock.packages[0]
                .remotes
                .iter()
                .all(|remote| matches!(remote, crate::manifest::PackageRemote::File { .. }))
        );
    }

    #[tokio::test]
    async fn direct_migration_preserves_an_explicit_file_alongside_a_provider_remote() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join("mods")).unwrap();
        write_empty_orbit_instance(&source, "26.2", "1");
        write_fabric_package(&source.join("mods/example.jar"), "example", ">=26.1");
        crate::sync_instance(&source, &[], false).await.unwrap();

        let mut manifest = ManifestFile::open(&source).unwrap().inner;
        manifest.packages["example"]
            .remotes
            .push(crate::manifest::PackageRemote::Modrinth {
                project_id: "provider-project".to_string(),
            });
        let lockfile = Lockfile::open(&source).unwrap().inner;
        let (remotes, _) = migration_discovery_state(&manifest, &lockfile, false);

        assert!(
            remotes
                .iter()
                .any(|remote| matches!(remote, crate::manifest::PackageRemote::Modrinth { .. }))
        );
        assert!(
            remotes
                .iter()
                .any(|remote| matches!(remote, crate::manifest::PackageRemote::File { .. }))
        );
    }

    #[tokio::test]
    async fn unavailable_strict_migration_requires_consent_then_minimizes_removals() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        std::fs::create_dir_all(source.join("mods")).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        write_empty_orbit_instance(&source, "1", "1");
        write_fabric_package(&source.join("mods/kept.jar"), "kept", ">=1");
        write_fabric_package(&source.join("mods/removed.jar"), "removed", "<2");
        crate::sync_instance(&source, &[], false).await.unwrap();
        crate::platform_detection::test_support::write_platform(&target, "2", "fabric", "1");
        let cache = crate::jar_cache::JarCache::open(root.path().join("cache")).unwrap();

        let error = plan_migration(
            &source,
            &target,
            &[],
            &cache,
            MigrationOptions::default(),
            MigrationInteraction::default(),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("package-removing migration was declined")
        );

        let confirmed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = confirmed.clone();
        let plan = plan_migration(
            &source,
            &target,
            &[],
            &cache,
            MigrationOptions::default(),
            MigrationInteraction {
                confirm_soft_fallback: Some(Box::new(move |preview| {
                    assert!(preview.strict_failure.contains("removed"));
                    observed.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(true)
                })),
                ..MigrationInteraction::default()
            },
        )
        .await
        .unwrap();

        assert!(confirmed.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(plan.selected_packages, 1);
        assert!(plan.target_manifest.packages.contains_key("kept"));
        assert!(!plan.target_manifest.packages.contains_key("removed"));
        assert!(plan.changes.iter().any(|change| {
            change.package == "removed" && change.kind == PackageChangeKind::Remove
        }));
        assert!(
            !plan
                .target_lockfile
                .packages
                .iter()
                .any(|package| package.mod_id == "removed")
        );
        assert!(
            plan.diagnostics
                .iter()
                .all(|diagnostic| diagnostic.package == "removed")
        );
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
            MigrationOptions::default(),
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
    async fn portable_file_only_package_is_preserved_for_target_install() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        std::fs::create_dir_all(source.join("mods")).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        write_empty_orbit_instance(&source, "1", "1");
        write_fabric_package(&source.join("mods/local.jar"), "local", ">=1");
        crate::sync_instance(&source, &[], false).await.unwrap();
        crate::platform_detection::test_support::write_platform(&target, "2", "fabric", "1");

        let pack = root.path().join("source.zip");
        crate::archive::export_instance(&source, &pack, None, "zip", false, None).unwrap();
        let portable = crate::archive::extract_portable_instance(&pack).unwrap();
        let cache = crate::jar_cache::JarCache::open(root.path().join("cache")).unwrap();
        let plan = plan_migration_from_portable(
            portable,
            &target,
            &[],
            &cache,
            MigrationOptions::default(),
            MigrationInteraction::default(),
        )
        .await
        .unwrap();

        let entry = plan.target_lockfile().find("local").unwrap();
        let managed = format!(".orbit/sources/{}.jar", entry.sha512);
        assert!(entry.artifact_sources.iter().any(
            |source| matches!(source, crate::lockfile::ArtifactSource::File { path } if path == &managed)
        ));
        export_migration(&plan, false).unwrap();
        assert!(target.join(managed).is_file());
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
            MigrationOptions::default(),
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
