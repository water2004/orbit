//! Shared reconciliation of top-level package candidates discovered in `mods/`.

use std::path::Path;

use crate::error::OrbitError;
use crate::lockfile::{LockMeta, PackageEntry};
use crate::manifest::{DependencySpec, OrbitManifest};
use crate::resolver::types::PlatformCandidate;
use crate::resolver::types::{ResolutionReport, ResolutionSelector};
use crate::{InstallReport, RemovedPackage};

pub(crate) struct LocalPackageSelection {
    pub(crate) selected_entries: Vec<PackageEntry>,
    pub(crate) removed: Vec<RemovedPackage>,
    pub(crate) resolution: ResolutionReport,
}

pub(crate) async fn select_local_packages(
    manifest: &OrbitManifest,
    local_entries: &[PackageEntry],
    loader_package: Option<PlatformCandidate>,
    selector: Option<ResolutionSelector>,
) -> Result<LocalPackageSelection, String> {
    let local_lockfile = crate::lockfile::OrbitLockfile {
        meta: LockMeta {
            mc_version: manifest.project.mc_version.clone(),
            modloader: manifest.project.modloader.clone(),
            modloader_version: manifest.project.modloader_version.clone(),
        },
        packages: local_entries.to_vec(),
    };
    let mut resolution_manifest = manifest.clone();
    for entry in local_entries {
        resolution_manifest
            .dependencies
            .entry(entry.mod_id.clone())
            .or_insert_with(|| DependencySpec::Short("*".to_string()));
    }
    let portfolio = crate::resolver::resolve_candidate_portfolio(
        &resolution_manifest,
        &local_lockfile,
        &crate::resolver::types::CandidateCatalog {
            loader_package,
            ..Default::default()
        },
    )
    .await?;
    let resolution = crate::resolver::select_resolution(portfolio, selector)?;
    let selected_entries = local_entries
        .iter()
        .filter(|entry| {
            resolution
                .selected_sources
                .get(&entry.mod_id)
                .is_some_and(|source| crate::resolver::locked_source(entry) == *source)
        })
        .cloned()
        .collect();
    let removed = local_package_removals(&resolution.changes);
    Ok(LocalPackageSelection {
        selected_entries,
        removed,
        resolution,
    })
}

pub(crate) fn confirmation_report(selection: &LocalPackageSelection) -> InstallReport {
    InstallReport {
        installed: Vec::new(),
        removed: selection.removed.clone(),
        changes: selection.resolution.changes.clone(),
        already_satisfied: Vec::new(),
        skipped_optional: Vec::new(),
        diagnostics: selection.resolution.diagnostics.clone(),
        warnings: selection.resolution.warnings.clone(),
    }
}

fn local_package_removals(
    changes: &[crate::resolver::types::PackageChange],
) -> Vec<RemovedPackage> {
    let mut removals: Vec<_> = changes
        .iter()
        .filter(|change| change.kind == crate::PackageChangeKind::Remove)
        .filter_map(|change| {
            Some(RemovedPackage {
                mod_id: change.package.clone(),
                version: change.current_version.clone()?,
                filename: change.filename.clone()?,
            })
        })
        .collect();
    removals.sort_by(|left, right| left.filename.cmp(&right.filename));
    removals.dedup_by(|left, right| left.filename == right.filename);
    removals
}

pub(crate) fn remove_unselected_packages(
    instance_dir: &Path,
    removals: &[RemovedPackage],
) -> Result<(), OrbitError> {
    let mods_dir = instance_dir.join("mods");
    for removal in removals {
        let filename = Path::new(&removal.filename);
        if filename.components().count() != 1 {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "lockfile contains non-local package filename '{}'",
                removal.filename
            )));
        }
        let path = mods_dir.join(filename);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
