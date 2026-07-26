//! Persistent least-recently-used policy for the content-addressed JAR store.
//!
//! This module owns access ordering, the on-disk index, artifact enumeration,
//! and eviction. Content lookup and copying remain in the parent cache module.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

use super::{CachePruneSummary, normalized_hash, write_atomic};
use crate::error::OrbitError;

const INDEX_VERSION: u32 = 1;
const INDEX_FILE: &str = "lru-index.json";
pub(super) const LOCK_FILE: &str = "lru.lock";

#[derive(Debug, Clone, Default)]
pub(super) struct AccessTracker {
    inner: Arc<Mutex<SessionAccesses>>,
}

#[derive(Debug, Default)]
struct SessionAccesses {
    last_order: u64,
    entries: BTreeMap<String, u64>,
}

impl AccessTracker {
    pub(super) fn record(&self, sha512: String) {
        let mut accesses = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        accesses.last_order = accesses.last_order.saturating_add(1);
        let order = accesses.last_order;
        accesses.entries.insert(sha512, order);
    }

    fn snapshot(&self) -> BTreeMap<String, u64> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .clone()
    }

    fn commit(&self, snapshot: &BTreeMap<String, u64>) {
        let mut accesses = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        accesses.entries.retain(|sha512, order| {
            snapshot
                .get(sha512)
                .is_none_or(|committed| *order > *committed)
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    clock: u64,
    entries: BTreeMap<String, AccessRecord>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            clock: 0,
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessRecord {
    last_used: u64,
}

#[derive(Debug)]
struct CachedArtifact {
    sha512: String,
    path: PathBuf,
    bytes: u64,
}

pub(super) fn prune(
    root: &Path,
    accesses: &AccessTracker,
    capacity_bytes: u64,
) -> Result<CachePruneSummary, OrbitError> {
    let mut summary = CachePruneSummary {
        path: root.to_path_buf(),
        capacity_bytes,
        ..CachePruneSummary::default()
    };
    let session_accesses = accesses.snapshot();
    let persisted = read_index(root)?;
    let mut artifacts = scan_artifacts(root)?;
    let mut entries = BTreeMap::new();

    for artifact in &artifacts {
        let last_used = persisted
            .entries
            .get(&artifact.sha512)
            .map(|record| record.last_used)
            .unwrap_or_default();
        entries.insert(artifact.sha512.clone(), AccessRecord { last_used });
        summary.files_before += 1;
        summary.bytes_before = summary
            .bytes_before
            .checked_add(artifact.bytes)
            .ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "JAR cache size exceeds the supported byte range"
                ))
            })?;
    }

    let mut clock = persisted.clock;
    let mut ordered_accesses = session_accesses.iter().collect::<Vec<_>>();
    ordered_accesses.sort_by(|(left_hash, left_order), (right_hash, right_order)| {
        left_order
            .cmp(right_order)
            .then_with(|| left_hash.cmp(right_hash))
    });
    for (sha512, _) in ordered_accesses {
        let Some(record) = entries.get_mut(sha512) else {
            continue;
        };
        clock = clock.checked_add(1).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "JAR cache LRU access counter is exhausted; run 'orbit cache clean' to reset it"
            ))
        })?;
        record.last_used = clock;
    }

    artifacts.sort_by(|left, right| {
        let left_access = entries[&left.sha512].last_used;
        let right_access = entries[&right.sha512].last_used;
        left_access
            .cmp(&right_access)
            .then_with(|| left.sha512.cmp(&right.sha512))
    });

    let mut remaining_bytes = summary.bytes_before;
    for artifact in artifacts {
        if remaining_bytes <= capacity_bytes {
            break;
        }
        std::fs::remove_file(&artifact.path)?;
        remaining_bytes -= artifact.bytes;
        summary.files_removed += 1;
        summary.bytes_freed += artifact.bytes;
        entries.remove(&artifact.sha512);
    }

    let surviving_hashes = entries.keys().cloned().collect();
    remove_stale_aliases(root, &surviving_hashes)?;
    write_index(
        root,
        &Index {
            version: INDEX_VERSION,
            clock,
            entries,
        },
    )?;
    accesses.commit(&session_accesses);

    summary.files_after = summary.files_before - summary.files_removed;
    summary.bytes_after = remaining_bytes;
    Ok(summary)
}

fn read_index(root: &Path) -> Result<Index, OrbitError> {
    let path = root.join(INDEX_FILE);
    if !path.is_file() {
        return Ok(Index::default());
    }
    let bytes = std::fs::read(&path)?;
    let index: Index = serde_json::from_slice(&bytes).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "failed to parse JAR cache LRU index '{}': {error}; run 'orbit cache clean' to reset it",
            path.display()
        ))
    })?;
    if index.version != INDEX_VERSION {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported JAR cache LRU index version {} in '{}'; expected {}",
            index.version,
            path.display(),
            INDEX_VERSION
        )));
    }
    if index
        .entries
        .values()
        .any(|record| record.last_used > index.clock)
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "JAR cache LRU index '{}' contains an access value beyond its clock; run 'orbit cache clean' to reset it",
            path.display()
        )));
    }
    Ok(index)
}

fn write_index(root: &Path, index: &Index) -> Result<(), OrbitError> {
    let bytes = serde_json::to_vec_pretty(index)?;
    write_atomic(&root.join(INDEX_FILE), &bytes)
}

fn scan_artifacts(root: &Path) -> Result<Vec<CachedArtifact>, OrbitError> {
    let directory = root.join("jars").join("sha512");
    if !directory.is_dir() {
        return Ok(Vec::new());
    }

    let mut artifacts = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "unexpected non-file entry in JAR cache: '{}'",
                entry.path().display()
            )));
        }
        let raw_name = entry.file_name();
        let raw_name = raw_name.to_str().ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "JAR cache contains a non-Unicode artifact name at '{}'",
                entry.path().display()
            ))
        })?;
        let sha512 = normalized_hash(raw_name, 128)
            .filter(|hash| hash == raw_name)
            .ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "JAR cache contains an invalid SHA-512 artifact name at '{}'",
                    entry.path().display()
                ))
            })?;
        artifacts.push(CachedArtifact {
            sha512,
            path: entry.path(),
            bytes: entry.metadata()?.len(),
        });
    }
    Ok(artifacts)
}

fn remove_stale_aliases(
    root: &Path,
    surviving_hashes: &BTreeSet<String>,
) -> Result<(), OrbitError> {
    let directory = root.join("aliases").join("sha1");
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = file_type
            .is_file()
            .then(|| std::fs::read_to_string(entry.path()).ok())
            .flatten()
            .and_then(|value| normalized_hash(value.trim(), 128));
        if target.is_none_or(|target| !surviving_hashes.contains(&target)) {
            if !file_type.is_file() {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "unexpected non-file entry in JAR cache aliases: '{}'",
                    entry.path().display()
                )));
            }
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
