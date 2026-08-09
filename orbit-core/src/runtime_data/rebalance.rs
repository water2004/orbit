//! Cold-path physical-tree recompression for runtime ownership.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::error::OrbitError;

use super::{
    DataOwnershipEntry, DataOwnershipLedger, OwnedDataKind, OwnedDataPath, compact_ledger,
    nearest_ancestor, owned_path, path_contains, path_depth, protected_instance_root,
    resolve_owned_path,
};

const MIN_EXCLUSIONS: usize = 64;

#[derive(Clone)]
struct IndexedOwner {
    kind: OwnedDataKind,
    owner: Option<String>,
}

type OwnerCounts = BTreeMap<Option<String>, u64>;

/// Recompress one ownership tree after its explicit exclusions have grown
/// geometrically. The scan reads directory metadata only; file contents are
/// never opened. Every file keeps the same effective owner before and after
/// the directory defaults are rebuilt.
pub(super) fn rebalance_ownership(
    instance_dir: &Path,
    ledger: &mut DataOwnershipLedger,
) -> Result<(), OrbitError> {
    let Ok(observation_epoch) = inactive_observation_epoch(instance_dir) else {
        return Ok(());
    };
    let mut candidates = ledger
        .entries
        .iter()
        .filter(|entry| entry.kind == OwnedDataKind::Tree && !protected_instance_root(&entry.path))
        .filter_map(|entry| {
            let exclusions = ledger
                .entries
                .iter()
                .filter(|candidate| path_contains(&entry.path, &candidate.path))
                .count();
            let key = ownership_path_key(&entry.path);
            let previous = ledger.rebalance_watermarks.get(&key).copied().unwrap_or(0);
            let threshold = if previous == 0 {
                MIN_EXCLUSIONS
            } else {
                previous.saturating_mul(2)
            };
            (exclusions >= threshold).then(|| (entry.path.clone(), exclusions, key))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| path_depth(&right.0).cmp(&path_depth(&left.0)))
    });
    let Some((root_path, exclusion_count, watermark_key)) = candidates.into_iter().next() else {
        return Ok(());
    };
    let physical_root = resolve_owned_path(instance_dir, &root_path)?;
    match std::fs::symlink_metadata(&physical_root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) | Err(_) => {
            ledger
                .rebalance_watermarks
                .insert(watermark_key, exclusion_count);
            return Ok(());
        }
    }

    let mut lookup = HashMap::with_capacity(ledger.entries.len());
    for entry in &ledger.entries {
        let resolved = resolve_owned_path(instance_dir, &entry.path)?;
        lookup.insert(
            normalized_physical_key(&resolved),
            IndexedOwner {
                kind: entry.kind,
                owner: entry.owner.clone(),
            },
        );
    }
    let inherited =
        nearest_ancestor(&ledger.entries, &root_path).and_then(|entry| entry.owner.clone());
    let mut directory_defaults = HashMap::new();
    if index_directory_owners(
        &physical_root,
        &lookup,
        inherited.clone(),
        &mut directory_defaults,
    )
    .is_err()
    {
        ledger
            .rebalance_watermarks
            .insert(watermark_key, exclusion_count);
        return Ok(());
    }

    let mut replacement = Vec::new();
    if encode_directory_owners(
        instance_dir,
        &physical_root,
        &lookup,
        &directory_defaults,
        inherited.clone(),
        inherited,
        &mut replacement,
    )
    .is_err()
    {
        ledger
            .rebalance_watermarks
            .insert(watermark_key, exclusion_count);
        return Ok(());
    }

    let mut missing = Vec::new();
    for entry in ledger
        .entries
        .iter()
        .filter(|entry| entry.path == root_path || path_contains(&root_path, &entry.path))
    {
        let resolved = resolve_owned_path(instance_dir, &entry.path)?;
        if !resolved.exists() {
            missing.push(entry.clone());
        }
    }
    if inactive_observation_epoch(instance_dir).ok() != Some(observation_epoch) {
        return Ok(());
    }
    ledger
        .entries
        .retain(|entry| entry.path != root_path && !path_contains(&root_path, &entry.path));
    ledger.entries.extend(replacement);
    ledger.entries.extend(missing);
    compact_ledger(ledger);
    let new_exclusions = ledger
        .entries
        .iter()
        .filter(|entry| path_contains(&root_path, &entry.path))
        .count();
    ledger
        .rebalance_watermarks
        .insert(watermark_key, new_exclusions.max(1));
    Ok(())
}

fn inactive_observation_epoch(instance_dir: &Path) -> Result<Option<Vec<u8>>, std::io::Error> {
    let runtime_data = instance_dir.join(".orbit/runtime-data");
    if runtime_data.join("observation.active").exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "runtime observation is active",
        ));
    }
    match std::fs::read(runtime_data.join("observation.epoch")) {
        Ok(epoch) => Ok(Some(epoch)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn index_directory_owners(
    directory: &Path,
    lookup: &HashMap<String, IndexedOwner>,
    inherited: Option<String>,
    defaults: &mut HashMap<String, Option<String>>,
) -> Result<OwnerCounts, std::io::Error> {
    let key = normalized_physical_key(directory);
    let exact = lookup.get(&key);
    let directory_owner = exact.map_or_else(|| inherited.clone(), |node| node.owner.clone());
    let child_inherited = exact
        .filter(|node| node.kind == OwnedDataKind::Tree)
        .map_or_else(|| inherited.clone(), |node| node.owner.clone());
    let mut counts = OwnerCounts::new();
    for child in std::fs::read_dir(directory)? {
        let child = child?.path();
        let metadata = std::fs::symlink_metadata(&child)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            merge_owner_counts(
                &mut counts,
                index_directory_owners(&child, lookup, child_inherited.clone(), defaults)?,
            );
        } else if metadata.is_file() || metadata.file_type().is_symlink() {
            let owner = lookup
                .get(&normalized_physical_key(&child))
                .map_or_else(|| child_inherited.clone(), |node| node.owner.clone());
            *counts.entry(owner).or_default() += 1;
        }
    }
    defaults.insert(key, dominant_owner(&counts, directory_owner));
    Ok(counts)
}

#[allow(clippy::too_many_arguments)]
fn encode_directory_owners(
    instance_dir: &Path,
    directory: &Path,
    lookup: &HashMap<String, IndexedOwner>,
    defaults: &HashMap<String, Option<String>>,
    old_inherited: Option<String>,
    new_inherited: Option<String>,
    output: &mut Vec<DataOwnershipEntry>,
) -> Result<(), OrbitError> {
    let key = normalized_physical_key(directory);
    let exact = lookup.get(&key);
    let old_child_inherited = exact
        .filter(|node| node.kind == OwnedDataKind::Tree)
        .map_or_else(|| old_inherited.clone(), |node| node.owner.clone());
    let selected = defaults
        .get(&key)
        .cloned()
        .unwrap_or_else(|| old_child_inherited.clone());
    let owned_directory = owned_path(instance_dir, directory)?;
    if selected != new_inherited && !protected_instance_root(&owned_directory) {
        output.push(DataOwnershipEntry {
            path: owned_directory,
            kind: OwnedDataKind::Tree,
            owner: selected.clone(),
        });
    }
    for child in std::fs::read_dir(directory)? {
        let child = child?.path();
        let metadata = std::fs::symlink_metadata(&child)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            encode_directory_owners(
                instance_dir,
                &child,
                lookup,
                defaults,
                old_child_inherited.clone(),
                selected.clone(),
                output,
            )?;
        } else if metadata.is_file() || metadata.file_type().is_symlink() {
            let owner = lookup
                .get(&normalized_physical_key(&child))
                .map_or_else(|| old_child_inherited.clone(), |node| node.owner.clone());
            if owner != selected {
                output.push(DataOwnershipEntry {
                    path: owned_path(instance_dir, &child)?,
                    kind: OwnedDataKind::File,
                    owner,
                });
            }
        }
    }
    Ok(())
}

fn dominant_owner(counts: &OwnerCounts, current: Option<String>) -> Option<String> {
    let Some(maximum) = counts.values().copied().max() else {
        return current;
    };
    let mut winners = counts
        .iter()
        .filter(|(_, count)| **count == maximum)
        .map(|(owner, _)| owner.clone());
    let first = winners.next().unwrap_or_else(|| current.clone());
    if winners.next().is_some() {
        current
    } else {
        first
    }
}

fn merge_owner_counts(target: &mut OwnerCounts, source: OwnerCounts) {
    for (owner, count) in source {
        *target.entry(owner).or_default() += count;
    }
}

fn normalized_physical_key(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        value.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value
    }
}

fn ownership_path_key(path: &OwnedDataPath) -> String {
    match path {
        OwnedDataPath::Instance { relative } => format!("instance:{relative}"),
        OwnedDataPath::External { absolute } => format!("external:{absolute}"),
    }
}
