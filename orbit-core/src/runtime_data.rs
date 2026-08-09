//! Runtime-observed mutable-data ownership for managed packages.
//!
//! The Java agent writes crash-tolerant session snapshots. Orbit merges those
//! snapshots into an instance-local ledger and maps the recorded top-level JAR
//! hashes back to logical packages through `orbit.lock`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::{OrbitError, RuntimeDataError};
use crate::installer::{RemoveReport, remove_from_instance};
use crate::workspace::Lockfile;

mod rebalance;

use rebalance::rebalance_ownership;

const RUNTIME_DATA_DIRECTORY: &str = ".orbit/runtime-data";
const LEDGER_FILE: &str = "ownership.toml";
const SESSIONS_DIRECTORY: &str = "sessions";
pub(crate) const LEDGER_SCHEMA: u32 = 3;
const OBSERVATION_SCHEMA: &str = "3";

pub(crate) const RESERVED_INSTANCE_ROOTS: &[&str] = &[
    ".orbit",
    "assets",
    "config",
    "libraries",
    "logs",
    "mods",
    "natives",
    "saves",
    "screenshots",
    "versions",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedDataKind {
    File,
    Tree,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum OwnedDataPath {
    Instance { relative: String },
    External { absolute: String },
}

impl OwnedDataPath {
    pub fn display(&self) -> &str {
        match self {
            Self::Instance { relative } => relative,
            Self::External { absolute } => absolute,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataOwnershipEntry {
    pub path: OwnedDataPath,
    pub kind: OwnedDataKind,
    /// The package artifact that last changed this path. Descendants inherit
    /// the nearest tree owner unless a more specific entry overrides it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DataOwnershipLedger {
    pub(crate) schema: u32,
    #[serde(default)]
    pub(crate) entries: Vec<DataOwnershipEntry>,
    /// Highest complete snapshot generation merged for each JVM session.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    merged_sessions: BTreeMap<String, u64>,
    /// Compressed exclusion count produced by the last directory inspection.
    /// A new physical scan is delayed until that representation has doubled.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    rebalance_watermarks: BTreeMap<String, usize>,
}

impl Default for DataOwnershipLedger {
    fn default() -> Self {
        Self {
            schema: LEDGER_SCHEMA,
            entries: Vec::new(),
            merged_sessions: BTreeMap::new(),
            rebalance_watermarks: BTreeMap::new(),
        }
    }
}

impl DataOwnershipLedger {
    #[cfg(test)]
    pub(crate) fn from_entries(entries: Vec<DataOwnershipEntry>) -> Self {
        Self {
            entries,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DataPurgeEntry {
    pub path: OwnedDataPath,
    pub kind: OwnedDataKind,
    /// More specific ownership roots retained below a tree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preserved: Vec<OwnedDataPath>,
}

impl DataPurgeEntry {
    pub fn display_path(&self) -> String {
        let suffix = matches!(self.kind, OwnedDataKind::Tree)
            .then_some("/**")
            .unwrap_or("");
        format!("{}{suffix}", self.path.display())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DataPurgePlan {
    pub mod_id: String,
    #[serde(skip_serializing)]
    artifact_sha256: String,
    pub entries: Vec<DataPurgeEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataPurgeReport {
    pub mod_id: String,
    pub jar_deleted: bool,
    pub removed: Vec<DataPurgeEntry>,
}

/// Allocate a unique session snapshot path for a Java launch.
pub fn observation_session_path(instance_dir: &Path) -> Result<PathBuf, OrbitError> {
    validate_instance(instance_dir)?;
    let sessions = instance_dir
        .join(RUNTIME_DATA_DIRECTORY)
        .join(SESSIONS_DIRECTORY);
    std::fs::create_dir_all(&sessions)?;
    let unique = format!(
        "{}-{}-{}.events",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SESSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    Ok(sessions.join(unique))
}

static SESSION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn purgeable(entry: &DataOwnershipEntry, owner: &str) -> bool {
    entry.owner.as_deref() == Some(owner)
}

fn nearest_ancestor<'a>(
    entries: &'a [DataOwnershipEntry],
    path: &OwnedDataPath,
) -> Option<&'a DataOwnershipEntry> {
    entries
        .iter()
        .filter(|entry| entry.kind == OwnedDataKind::Tree && path_contains(&entry.path, path))
        .max_by_key(|entry| path_depth(&entry.path))
}

fn build_purge_entries(ledger: &DataOwnershipLedger, owner: &str) -> Vec<DataPurgeEntry> {
    let roots = ledger
        .entries
        .iter()
        .filter(|entry| purgeable(entry, owner))
        .filter(|entry| {
            nearest_ancestor(&ledger.entries, &entry.path)
                .is_none_or(|ancestor| !purgeable(ancestor, owner))
        })
        .collect::<Vec<_>>();

    roots
        .into_iter()
        .map(|root| {
            let mut preserved = ledger
                .entries
                .iter()
                .filter(|entry| path_contains(&root.path, &entry.path) && !purgeable(entry, owner))
                .filter(|entry| {
                    !ledger.entries.iter().any(|ancestor| {
                        ancestor.path != root.path
                            && ancestor.path != entry.path
                            && path_contains(&root.path, &ancestor.path)
                            && path_contains(&ancestor.path, &entry.path)
                            && !purgeable(ancestor, owner)
                    })
                })
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>();
            preserved.sort_by(|left, right| left.display().cmp(right.display()));
            DataPurgeEntry {
                path: root.path.clone(),
                kind: root.kind,
                preserved,
            }
        })
        .collect()
}

/// Merge every complete Agent snapshot. Invalid snapshots fail visibly and are
/// retained for inspection; successfully committed snapshots are removed.
pub fn merge_observation_sessions(instance_dir: &Path) -> Result<usize, OrbitError> {
    let root = validate_instance(instance_dir)?;
    let sessions = root.join(RUNTIME_DATA_DIRECTORY).join(SESSIONS_DIRECTORY);
    if !sessions.is_dir() {
        return Ok(0);
    }
    let paths = observation_snapshots(&sessions)?;
    if paths.is_empty() {
        return Ok(0);
    }

    // Claim live snapshots before reading them. The Agent may atomically
    // replace `<session>.events` while a server is still running; renaming
    // first lets its next snapshot land at the original path instead of being
    // deleted after we read an older generation. Claimed files survive an
    // Orbit crash and are merged idempotently by the next command.
    let mut claimed = Vec::with_capacity(paths.len());
    for path in paths {
        if is_claimed_snapshot(&path) {
            claimed.push(path);
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                OrbitError::RuntimeData(RuntimeDataError::NonUnicodeSnapshotName {
                    path: path.display().to_string(),
                })
            })?;
        let claimed_path = path.with_file_name(format!(
            "{file_name}.claimed-{}-{}",
            std::process::id(),
            SESSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::rename(&path, &claimed_path)?;
        claimed.push(claimed_path);
    }

    let mut ledger = load_ledger(&root)?;
    let ownership_changed = merge_snapshots_into_ledger(&root, &claimed, &mut ledger)?;
    compact_ledger(&mut ledger);
    if ownership_changed {
        rebalance_ownership(&root, &mut ledger)?;
    }
    save_ledger(&root, &ledger)?;
    for path in &claimed {
        std::fs::remove_file(path)?;
    }
    Ok(claimed.len())
}

/// Remove explicit ownership nodes whose physical paths no longer exist.
/// Only `NotFound` means absence; permission and other metadata failures abort
/// without rewriting the ledger.
pub(crate) fn prune_missing_ownership(instance_dir: &Path) -> Result<usize, OrbitError> {
    let root = validate_instance(instance_dir)?;
    let mut ledger = load_ledger(&root)?;
    let mut retained = Vec::with_capacity(ledger.entries.len());
    let mut removed = 0;
    for entry in std::mem::take(&mut ledger.entries) {
        let resolved = resolve_owned_path(&root, &entry.path)?;
        match std::fs::symlink_metadata(&resolved) {
            Ok(_) => retained.push(entry),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => removed += 1,
            Err(error) => return Err(error.into()),
        }
    }
    ledger.entries = retained;
    compact_ledger(&mut ledger);
    let active_trees = ledger
        .entries
        .iter()
        .filter(|entry| entry.kind == OwnedDataKind::Tree)
        .map(|entry| ownership_path_key(&entry.path))
        .collect::<BTreeSet<_>>();
    let previous_watermarks = ledger.rebalance_watermarks.len();
    ledger
        .rebalance_watermarks
        .retain(|path, _| active_trees.contains(path));
    if removed != 0 || ledger.rebalance_watermarks.len() != previous_watermarks {
        save_ledger(&root, &ledger)?;
    }
    Ok(removed)
}

fn observation_snapshots(sessions: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    if !sessions.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = std::fs::read_dir(sessions)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_observation_snapshot(path))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn merge_snapshots_into_ledger(
    instance_dir: &Path,
    snapshots: &[PathBuf],
    ledger: &mut DataOwnershipLedger,
) -> Result<bool, OrbitError> {
    let mut parsed = Vec::with_capacity(snapshots.len());
    for path in snapshots {
        let document = std::fs::read_to_string(path)?;
        parsed.push(parse_snapshot(path, &document)?);
    }
    parsed.sort_by(|left, right| {
        left.started_at_millis
            .cmp(&right.started_at_millis)
            .then_with(|| left.session.cmp(&right.session))
            .then_with(|| left.generation.cmp(&right.generation))
    });

    let mut changed = false;
    for mut snapshot in parsed {
        if ledger
            .merged_sessions
            .get(&snapshot.session)
            .is_some_and(|generation| *generation >= snapshot.generation)
        {
            continue;
        }
        snapshot.observations.sort_by(|left, right| {
            left.revision
                .cmp(&right.revision)
                .then_with(|| left.absolute.cmp(&right.absolute))
        });
        for observation in snapshot.observations {
            changed |= merge_observation(instance_dir, ledger, observation)?;
        }
        ledger
            .merged_sessions
            .insert(snapshot.session, snapshot.generation);
    }
    Ok(changed)
}

fn is_observation_snapshot(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".events") || name.contains(".events.claimed-"))
}

fn is_claimed_snapshot(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(".events.claimed-"))
}

pub fn plan_data_purge(
    instance_dir: &Path,
    package: &str,
    dry_run: bool,
) -> Result<DataPurgePlan, OrbitError> {
    let root = validate_instance(instance_dir)?;
    let ledger = if dry_run {
        let mut ledger = load_ledger(&root)?;
        let sessions = root.join(RUNTIME_DATA_DIRECTORY).join(SESSIONS_DIRECTORY);
        let snapshots = observation_snapshots(&sessions)?;
        let ownership_changed = merge_snapshots_into_ledger(&root, &snapshots, &mut ledger)?;
        compact_ledger(&mut ledger);
        if ownership_changed {
            rebalance_ownership(&root, &mut ledger)?;
        }
        ledger
    } else {
        merge_observation_sessions(&root)?;
        load_ledger(&root)?
    };
    let lock = Lockfile::open(&root)?;
    let entry = lock
        .find_entry(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let artifact_sha256 = entry.sha256.clone();
    let mut entries = build_purge_entries(&ledger, &artifact_sha256);
    entries.sort_by(|left, right| left.path.display().cmp(right.path.display()));
    Ok(DataPurgePlan {
        mod_id: entry.mod_id.clone(),
        artifact_sha256,
        entries,
    })
}

/// Remove the logical package first, then delete the previously confirmed
/// runtime-owned paths. The lock hash is revalidated immediately before the
/// mutation so a stale plan cannot target another package realization.
pub fn apply_data_purge(
    instance_dir: &Path,
    plan: &DataPurgePlan,
    dry_run: bool,
) -> Result<DataPurgeReport, OrbitError> {
    let root = validate_instance(instance_dir)?;
    let lock = Lockfile::open(&root)?;
    let entry = lock
        .find_entry(&plan.mod_id)
        .ok_or_else(|| OrbitError::ModNotFound(plan.mod_id.clone()))?;
    if entry.sha256 != plan.artifact_sha256 {
        return Err(OrbitError::RuntimeData(RuntimeDataError::PackageChanged {
            package: plan.mod_id.clone(),
        }));
    }
    validate_plan_paths(&root, &plan.entries)?;
    if dry_run {
        return Ok(DataPurgeReport {
            mod_id: plan.mod_id.clone(),
            jar_deleted: false,
            removed: plan.entries.clone(),
        });
    }

    let RemoveReport {
        mod_id,
        jar_deleted,
    } = remove_from_instance(&plan.mod_id, &root, false)?;
    let mut ledger = load_ledger(&root)?;
    let mut removed = Vec::new();
    for selected in &plan.entries {
        let path = resolve_owned_path(&root, &selected.path)?;
        let preserved = selected
            .preserved
            .iter()
            .map(|entry| resolve_owned_path(&root, entry))
            .collect::<Result<Vec<_>, _>>()?;
        if let Err(error) = remove_owned_path(&path, selected.kind, &preserved) {
            remove_artifact_references(&mut ledger, &plan.artifact_sha256, &removed);
            save_ledger(&root, &ledger)?;
            return Err(OrbitError::RuntimeData(
                RuntimeDataError::DeleteAfterPackageRemoval {
                    package: mod_id,
                    path: selected.display_path(),
                    completed: removed.len(),
                    detail: error.to_string(),
                },
            ));
        }
        removed.push(selected.clone());
    }
    remove_artifact_references(&mut ledger, &plan.artifact_sha256, &removed);
    save_ledger(&root, &ledger)?;
    Ok(DataPurgeReport {
        mod_id,
        jar_deleted,
        removed,
    })
}

fn validate_instance(instance_dir: &Path) -> Result<PathBuf, OrbitError> {
    if !instance_dir.join("orbit.toml").is_file() {
        return Err(OrbitError::ManifestNotFound);
    }
    dunce::canonicalize(instance_dir).map_err(OrbitError::from)
}

pub(crate) fn ledger_path(instance_dir: &Path) -> PathBuf {
    instance_dir.join(RUNTIME_DATA_DIRECTORY).join(LEDGER_FILE)
}

pub(crate) fn ownership_context(
    instance_dir: &Path,
) -> Result<Vec<(PathBuf, OwnedDataKind, Option<String>)>, OrbitError> {
    let root = validate_instance(instance_dir)?;
    let ledger = load_ledger(&root)?;
    ledger
        .entries
        .into_iter()
        .map(|entry| {
            Ok((
                resolve_owned_path(&root, &entry.path)?,
                entry.kind,
                entry.owner,
            ))
        })
        .collect()
}

pub(crate) fn load_ledger(instance_dir: &Path) -> Result<DataOwnershipLedger, OrbitError> {
    let path = ledger_path(instance_dir);
    if !path.is_file() {
        return Ok(DataOwnershipLedger::default());
    }
    let document = std::fs::read_to_string(&path)?;
    let ledger: DataOwnershipLedger = toml::from_str(&document).map_err(|error| {
        OrbitError::RuntimeData(RuntimeDataError::LedgerParse {
            path: path.display().to_string(),
            detail: error.to_string(),
        })
    })?;
    if ledger.schema != LEDGER_SCHEMA {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::UnsupportedLedgerSchema {
                schema: ledger.schema,
                path: path.display().to_string(),
            },
        ));
    }
    Ok(ledger)
}

pub(crate) fn save_ledger(
    instance_dir: &Path,
    ledger: &DataOwnershipLedger,
) -> Result<(), OrbitError> {
    let path = ledger_path(instance_dir);
    let parent = path.parent().expect("ledger path has a parent");
    std::fs::create_dir_all(parent)?;
    let document = toml::to_string_pretty(ledger).map_err(|error| {
        OrbitError::RuntimeData(RuntimeDataError::LedgerSerialize {
            detail: error.to_string(),
        })
    })?;
    crate::atomic_io::write_atomic(&path, document.as_bytes())
}

pub(crate) fn ownership_document(entries: Vec<DataOwnershipEntry>) -> Result<String, OrbitError> {
    toml::to_string_pretty(&DataOwnershipLedger {
        schema: LEDGER_SCHEMA,
        entries,
        merged_sessions: BTreeMap::new(),
        rebalance_watermarks: BTreeMap::new(),
    })
    .map_err(|error| {
        OrbitError::RuntimeData(RuntimeDataError::LedgerSerialize {
            detail: error.to_string(),
        })
    })
}

pub(crate) fn parse_ownership_document(
    path: &Path,
    document: &str,
) -> Result<Vec<DataOwnershipEntry>, OrbitError> {
    let ledger: DataOwnershipLedger = toml::from_str(document).map_err(|error| {
        OrbitError::RuntimeData(RuntimeDataError::LedgerParse {
            path: path.display().to_string(),
            detail: error.to_string(),
        })
    })?;
    if ledger.schema != LEDGER_SCHEMA {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::UnsupportedLedgerSchema {
                schema: ledger.schema,
                path: path.display().to_string(),
            },
        ));
    }
    Ok(ledger.entries)
}

pub(crate) fn ownership_entries_for(
    instance_dir: &Path,
    owners: &BTreeSet<String>,
) -> Result<Vec<DataOwnershipEntry>, OrbitError> {
    let root = validate_instance(instance_dir)?;
    let mut ledger = load_ledger(&root)?;
    compact_ledger(&mut ledger);
    let selected_trees = ledger
        .entries
        .iter()
        .filter(|entry| {
            entry.kind == OwnedDataKind::Tree
                && entry
                    .owner
                    .as_ref()
                    .is_some_and(|owner| owners.contains(owner))
        })
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    Ok(ledger
        .entries
        .into_iter()
        .filter(|entry| {
            entry
                .owner
                .as_ref()
                .is_some_and(|owner| owners.contains(owner))
                || selected_trees
                    .iter()
                    .any(|tree| path_contains(tree, &entry.path))
        })
        .collect())
}

pub(crate) fn effective_owner_for_relative(
    entries: &[DataOwnershipEntry],
    relative: &Path,
) -> Option<String> {
    let path = OwnedDataPath::Instance {
        relative: relative.to_string_lossy().replace('\\', "/"),
    };
    let ledger = DataOwnershipLedger {
        schema: LEDGER_SCHEMA,
        entries: entries.to_vec(),
        merged_sessions: BTreeMap::new(),
        rebalance_watermarks: BTreeMap::new(),
    };
    effective_owner(&ledger, &path)
}

pub(crate) fn rebind_ownership_entries(
    entries: &mut Vec<DataOwnershipEntry>,
    owners: &std::collections::BTreeMap<String, String>,
) {
    for entry in entries.iter_mut() {
        entry.owner = entry
            .owner
            .as_ref()
            .and_then(|owner| owners.get(owner).cloned());
    }
    let mut ledger = DataOwnershipLedger {
        schema: LEDGER_SCHEMA,
        entries: std::mem::take(entries),
        merged_sessions: BTreeMap::new(),
        rebalance_watermarks: BTreeMap::new(),
    };
    compact_ledger(&mut ledger);
    *entries = ledger.entries;
}

struct Observation {
    action: ObservationAction,
    kind: OwnedDataKind,
    owner: String,
    revision: u64,
    absolute: PathBuf,
}

struct ObservationSnapshot {
    session: String,
    started_at_millis: u64,
    generation: u64,
    observations: Vec<Observation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservationAction {
    Create,
    Write,
    Delete,
}

fn parse_snapshot(path: &Path, document: &str) -> Result<ObservationSnapshot, OrbitError> {
    let mut lines = document
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty());
    let (_, header) = lines.next().ok_or_else(|| {
        OrbitError::RuntimeData(RuntimeDataError::InvalidObservation {
            path: path.display().to_string(),
            line: 1,
            detail: "missing v3 snapshot header".to_string(),
        })
    })?;
    let fields = header.split('\t').collect::<Vec<_>>();
    if fields.len() != 6
        || fields[0] != OBSERVATION_SCHEMA
        || fields[1] != "snapshot"
        || fields[5] != "end"
        || fields[2].is_empty()
        || !fields[2]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::InvalidObservation {
                path: path.display().to_string(),
                line: 1,
                detail: "expected a complete v3 snapshot header".to_string(),
            },
        ));
    }
    let parse_header_number = |field: &str, detail: &str| {
        field.parse::<u64>().map_err(|_| {
            OrbitError::RuntimeData(RuntimeDataError::InvalidObservation {
                path: path.display().to_string(),
                line: 1,
                detail: detail.to_string(),
            })
        })
    };
    let started_at_millis = parse_header_number(fields[3], "invalid snapshot start time")?;
    let generation = parse_header_number(fields[4], "invalid snapshot generation")?;
    let mut observations = Vec::new();
    for (index, line) in lines {
        let observation = parse_observation(line).map_err(|detail| {
            OrbitError::RuntimeData(RuntimeDataError::InvalidObservation {
                path: path.display().to_string(),
                line: index + 1,
                detail,
            })
        })?;
        if observation.revision > generation {
            return Err(OrbitError::RuntimeData(
                RuntimeDataError::InvalidObservation {
                    path: path.display().to_string(),
                    line: index + 1,
                    detail: "record revision exceeds snapshot generation".to_string(),
                },
            ));
        }
        observations.push(observation);
    }
    Ok(ObservationSnapshot {
        session: fields[2].to_string(),
        started_at_millis,
        generation,
        observations,
    })
}

fn parse_observation(line: &str) -> Result<Observation, String> {
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 7 || fields[0] != OBSERVATION_SCHEMA || fields[6] != "end" {
        return Err("expected a complete v3 mutation record".to_string());
    }
    let action = match fields[1] {
        "create" => ObservationAction::Create,
        "write" => ObservationAction::Write,
        "delete" => ObservationAction::Delete,
        value => return Err(format!("unknown mutation action '{value}'")),
    };
    let kind = match fields[2] {
        "file" => OwnedDataKind::File,
        "tree" => OwnedDataKind::Tree,
        value => return Err(format!("unknown path kind '{value}'")),
    };
    if fields[3].len() != 64 || !fields[3].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid owner SHA-256".to_string());
    }
    let revision = fields[4]
        .parse::<u64>()
        .map_err(|_| "invalid mutation revision".to_string())?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(fields[5])
        .map_err(|error| format!("invalid path encoding: {error}"))?;
    let path = String::from_utf8(decoded).map_err(|error| format!("path is not UTF-8: {error}"))?;
    let absolute = PathBuf::from(path);
    if !absolute.is_absolute() {
        return Err("observed path is not absolute".to_string());
    }
    Ok(Observation {
        action,
        kind,
        owner: fields[3].to_ascii_lowercase(),
        revision,
        absolute,
    })
}

fn merge_observation(
    instance_dir: &Path,
    ledger: &mut DataOwnershipLedger,
    observation: Observation,
) -> Result<bool, OrbitError> {
    let path = owned_path(instance_dir, &observation.absolute)?;
    if protected_control_path(&path) {
        return Ok(false);
    }
    let changed = match observation.action {
        ObservationAction::Create => {
            apply_creation(ledger, path, observation.kind, observation.owner)
        }
        ObservationAction::Write => apply_write(ledger, path, observation.kind, observation.owner),
        ObservationAction::Delete => apply_deletion(ledger, &path),
    };
    Ok(changed)
}

fn apply_creation(
    ledger: &mut DataOwnershipLedger,
    path: OwnedDataPath,
    kind: OwnedDataKind,
    owner: String,
) -> bool {
    if kind == OwnedDataKind::Tree && protected_instance_root(&path) {
        return false;
    }
    let before = ledger.entries.clone();
    ledger
        .entries
        .retain(|entry| !path_contains(&path, &entry.path) && entry.path != path);
    if kind == OwnedDataKind::File {
        claim_unowned_parent(ledger, &path, &owner);
    }
    if effective_owner(ledger, &path).is_some_and(|inherited| inherited == owner) {
        return ledger.entries != before;
    }
    ledger.entries.push(DataOwnershipEntry {
        path,
        kind,
        owner: Some(owner),
    });
    true
}

fn apply_write(
    ledger: &mut DataOwnershipLedger,
    path: OwnedDataPath,
    kind: OwnedDataKind,
    writer: String,
) -> bool {
    if let Some(index) = ledger.entries.iter().position(|entry| entry.path == path) {
        if kind == OwnedDataKind::File && ledger.entries[index].owner.is_none() {
            ledger.entries.remove(index);
            if claim_unowned_parent(ledger, &path, &writer) {
                return true;
            }
            ledger.entries.push(DataOwnershipEntry {
                path,
                kind,
                owner: Some(writer),
            });
            return true;
        }
        let entry = &mut ledger.entries[index];
        if entry.owner.as_deref() == Some(writer.as_str()) && entry.kind == kind {
            return false;
        }
        entry.owner = Some(writer);
        entry.kind = kind;
        return true;
    }
    if effective_owner(ledger, &path).as_deref() == Some(writer.as_str()) {
        return false;
    }
    if kind == OwnedDataKind::File && claim_unowned_parent(ledger, &path, &writer) {
        return true;
    }
    ledger.entries.push(DataOwnershipEntry {
        path,
        kind,
        owner: Some(writer),
    });
    true
}

fn apply_deletion(ledger: &mut DataOwnershipLedger, path: &OwnedDataPath) -> bool {
    let previous = ledger.entries.len();
    ledger
        .entries
        .retain(|entry| entry.path != *path && !path_contains(path, &entry.path));
    ledger.entries.len() != previous
}

fn claim_unowned_parent(
    ledger: &mut DataOwnershipLedger,
    path: &OwnedDataPath,
    owner: &str,
) -> bool {
    let Some(parent) = parent_owned_path(path) else {
        return false;
    };
    if protected_instance_root(&parent) || effective_owner(ledger, &parent).is_some() {
        return false;
    }
    ledger.entries.retain(|entry| entry.path != parent);
    ledger.entries.push(DataOwnershipEntry {
        path: parent,
        kind: OwnedDataKind::Tree,
        owner: Some(owner.to_string()),
    });
    true
}

fn effective_owner(ledger: &DataOwnershipLedger, path: &OwnedDataPath) -> Option<String> {
    ledger
        .entries
        .iter()
        .filter(|entry| {
            entry.path == *path
                || (entry.kind == OwnedDataKind::Tree && path_contains(&entry.path, path))
        })
        .max_by_key(|entry| path_depth(&entry.path))
        .and_then(|entry| entry.owner.clone())
}

fn compact_ledger(ledger: &mut DataOwnershipLedger) {
    ledger.entries.sort_by(|left, right| {
        path_depth(&left.path)
            .cmp(&path_depth(&right.path))
            .then_with(|| left.path.display().cmp(right.path.display()))
    });
    let mut compacted: Vec<DataOwnershipEntry> = Vec::with_capacity(ledger.entries.len());
    for entry in ledger.entries.drain(..) {
        let inherited = compacted
            .iter()
            .filter(|parent| {
                parent.kind == OwnedDataKind::Tree && path_contains(&parent.path, &entry.path)
            })
            .max_by_key(|parent| path_depth(&parent.path));
        if inherited.is_some_and(|parent| parent.owner == entry.owner) {
            continue;
        }
        compacted.push(entry);
    }
    compacted.sort_by(|left, right| left.path.display().cmp(right.path.display()));
    ledger.entries = compacted;
}

fn owned_path(instance_dir: &Path, absolute: &Path) -> Result<OwnedDataPath, OrbitError> {
    let absolute = absolute.to_path_buf();
    if let Some(relative) = instance_relative_path(instance_dir, &absolute) {
        validate_relative(&relative)?;
        return Ok(OwnedDataPath::Instance {
            relative: relative.to_string_lossy().replace('\\', "/"),
        });
    }
    Ok(OwnedDataPath::External {
        absolute: absolute.to_string_lossy().into_owned(),
    })
}

fn instance_relative_path(instance_dir: &Path, absolute: &Path) -> Option<PathBuf> {
    let instance_dir = dunce::simplified(instance_dir);
    let absolute = dunce::simplified(absolute);
    #[cfg(not(windows))]
    {
        absolute
            .strip_prefix(instance_dir)
            .ok()
            .map(Path::to_path_buf)
    }
    #[cfg(windows)]
    {
        let root = instance_dir.components().collect::<Vec<_>>();
        let path = absolute.components().collect::<Vec<_>>();
        if root.len() > path.len()
            || root.iter().zip(&path).any(|(left, right)| {
                !left
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
            })
        {
            return None;
        }
        let mut relative = PathBuf::new();
        for component in &path[root.len()..] {
            relative.push(component.as_os_str());
        }
        Some(relative)
    }
}

fn path_depth(path: &OwnedDataPath) -> usize {
    match path {
        OwnedDataPath::Instance { relative } => Path::new(relative).components().count(),
        OwnedDataPath::External { absolute } => Path::new(absolute).components().count(),
    }
}

fn parent_owned_path(path: &OwnedDataPath) -> Option<OwnedDataPath> {
    match path {
        OwnedDataPath::Instance { relative } => {
            let parent = Path::new(relative).parent()?;
            if parent.as_os_str().is_empty() {
                return None;
            }
            Some(OwnedDataPath::Instance {
                relative: parent.to_string_lossy().replace('\\', "/"),
            })
        }
        OwnedDataPath::External { absolute } => {
            let parent = Path::new(absolute).parent()?;
            parent.parent()?;
            Some(OwnedDataPath::External {
                absolute: parent.to_string_lossy().into_owned(),
            })
        }
    }
}

fn ownership_path_key(path: &OwnedDataPath) -> String {
    match path {
        OwnedDataPath::Instance { relative } => format!("instance:{relative}"),
        OwnedDataPath::External { absolute } => format!("external:{absolute}"),
    }
}

fn protected_instance_root(path: &OwnedDataPath) -> bool {
    matches!(
        path,
        OwnedDataPath::Instance { relative }
            if RESERVED_INSTANCE_ROOTS.contains(&relative.as_str())
    )
}

fn validate_relative(relative: &Path) -> Result<(), OrbitError> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::UnsafeRelativePath {
                path: relative.display().to_string(),
            },
        ));
    }
    Ok(())
}

fn protected_control_path(path: &OwnedDataPath) -> bool {
    match path {
        OwnedDataPath::Instance { relative } => {
            relative == ".orbit" || relative.starts_with(".orbit/")
        }
        OwnedDataPath::External { .. } => false,
    }
}

fn path_contains(parent: &OwnedDataPath, child: &OwnedDataPath) -> bool {
    match (parent, child) {
        (
            OwnedDataPath::Instance { relative: parent },
            OwnedDataPath::Instance { relative: child },
        ) => child != parent && child.starts_with(&format!("{parent}/")),
        (
            OwnedDataPath::External { absolute: parent },
            OwnedDataPath::External { absolute: child },
        ) => {
            let parent = Path::new(parent);
            let child = Path::new(child);
            child != parent && child.starts_with(parent)
        }
        _ => false,
    }
}

fn validate_plan_paths(instance_dir: &Path, entries: &[DataPurgeEntry]) -> Result<(), OrbitError> {
    for entry in entries {
        let resolved = resolve_owned_path(instance_dir, &entry.path)?;
        if resolved == instance_dir {
            return Err(OrbitError::RuntimeData(RuntimeDataError::InstanceRoot));
        }
        if let OwnedDataPath::Instance { relative } = &entry.path {
            if relative == ".orbit" || relative.starts_with(".orbit/") {
                return Err(OrbitError::RuntimeData(RuntimeDataError::ControlData));
            }
            if entry.kind == OwnedDataKind::Tree
                && RESERVED_INSTANCE_ROOTS.contains(&relative.as_str())
            {
                return Err(OrbitError::RuntimeData(
                    RuntimeDataError::SharedInstanceRoot {
                        path: relative.clone(),
                    },
                ));
            }
        }
    }
    Ok(())
}

fn resolve_owned_path(instance_dir: &Path, path: &OwnedDataPath) -> Result<PathBuf, OrbitError> {
    match path {
        OwnedDataPath::Instance { relative } => {
            let relative = Path::new(relative);
            validate_relative(relative)?;
            Ok(instance_dir.join(relative))
        }
        OwnedDataPath::External { absolute } => {
            let path = PathBuf::from(absolute);
            if !path.is_absolute()
                || path
                    .components()
                    .any(|component| matches!(component, Component::ParentDir))
                || path.parent().is_none()
            {
                return Err(OrbitError::RuntimeData(
                    RuntimeDataError::UnsafeExternalPath {
                        path: absolute.clone(),
                    },
                ));
            }
            Ok(path)
        }
    }
}

fn remove_owned_path(
    path: &Path,
    kind: OwnedDataKind,
    preserved: &[PathBuf],
) -> Result<(), std::io::Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return std::fs::remove_file(path);
    }
    if metadata.is_dir() {
        return match kind {
            OwnedDataKind::File => std::fs::remove_dir(path),
            OwnedDataKind::Tree => remove_tree_preserving(path, preserved),
        };
    }
    Ok(())
}

fn remove_tree_preserving(path: &Path, preserved: &[PathBuf]) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if preserved.iter().any(|keep| keep == &child) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&child)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            if preserved.iter().any(|keep| keep.starts_with(&child)) {
                remove_tree_preserving(&child, preserved)?;
            } else {
                remove_tree_preserving(&child, &[])?;
            }
        } else {
            std::fs::remove_file(&child)?;
        }
    }
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_artifact_references(
    ledger: &mut DataOwnershipLedger,
    artifact_sha256: &str,
    removed: &[DataPurgeEntry],
) {
    ledger.entries.retain_mut(|entry| {
        let was_removed = removed.iter().any(|item| {
            item.path == entry.path
                || (item.kind == OwnedDataKind::Tree
                    && path_contains(&item.path, &entry.path)
                    && !item.preserved.iter().any(|preserved| {
                        *preserved == entry.path || path_contains(preserved, &entry.path)
                    }))
        });
        if was_removed {
            return false;
        }
        if entry.owner.as_deref() == Some(artifact_sha256) {
            entry.owner = None;
        }
        true
    });
    compact_ledger(ledger);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{ArtifactSource, LockMeta, OrbitLockfile, PackageEntry};
    use crate::manifest::{
        OrbitManifest, PackageRemote, PackageSpec, PlatformArtifact, PlatformSnapshot, ProjectMeta,
        ResolverConfig,
    };
    use crate::workspace::{Lockfile, ManifestFile};

    fn instance() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("orbit.toml"), "test fixture").unwrap();
        directory
    }

    fn observation_line(action: &str, kind: &str, owner: &str, path: &Path) -> String {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(path.to_string_lossy().as_bytes());
        format!(
            "3\tsnapshot\ttest-session\t1\t1\tend\n3\t{action}\t{kind}\t{owner}\t1\t{encoded}\tend\n"
        )
    }

    #[test]
    fn parses_complete_observation_records() {
        let absolute = std::env::temp_dir().join("orbit-runtime-data-parse");
        let path = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(absolute.to_string_lossy().as_bytes());
        let record = parse_observation(&format!(
            "3\tcreate\ttree\t{}\t1\t{path}\tend",
            "a".repeat(64)
        ))
        .unwrap();
        assert_eq!(record.kind, OwnedDataKind::Tree);
        assert_eq!(record.action, ObservationAction::Create);
    }

    #[test]
    fn rejects_truncated_observation_records() {
        assert!(parse_observation("3\tcreate\tfile").is_err());
    }

    #[test]
    fn recognizes_an_instance_descendant_after_path_simplification() {
        let directory = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(directory.path()).unwrap();
        assert_eq!(
            instance_relative_path(&root, &root.join("config/example.toml")),
            Some(PathBuf::from("config/example.toml"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_instance_paths_ignore_verbatim_prefix_and_case() {
        assert_eq!(
            instance_relative_path(
                Path::new(r"\\?\D:\Games\Orbit\Instance"),
                Path::new(r"d:\games\orbit\instance\BlueMap\index.json")
            ),
            Some(PathBuf::from(r"BlueMap\index.json"))
        );
    }

    #[test]
    fn recovers_a_snapshot_claimed_by_an_interrupted_merge() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let session = observation_session_path(&root).unwrap();
        let claimed = session.with_file_name(format!(
            "{}.claimed-interrupted",
            session.file_name().unwrap().to_string_lossy()
        ));
        let owner = "a".repeat(64);
        let owned = root.join("config/example/state.db");
        std::fs::write(&session, observation_line("create", "file", &owner, &owned)).unwrap();
        std::fs::rename(&session, &claimed).unwrap();

        assert_eq!(merge_observation_sessions(&root).unwrap(), 1);
        assert!(!claimed.exists());
        assert_eq!(load_ledger(&root).unwrap().entries.len(), 1);
    }

    #[test]
    fn tree_deletion_does_not_follow_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("owned");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("data"), b"owned").unwrap();
        remove_owned_path(&tree, OwnedDataKind::Tree, &[]).unwrap();
        assert!(!tree.exists());
    }

    #[test]
    fn control_directory_is_never_attributed() {
        assert!(protected_control_path(&OwnedDataPath::Instance {
            relative: ".orbit/runtime-data/session".to_string(),
        }));
    }

    #[test]
    fn merges_session_snapshots_into_instance_relative_ownership() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let session = observation_session_path(&root).unwrap();
        let owner = "a".repeat(64);
        let owned = root.join("config/example/database");
        std::fs::write(&session, observation_line("create", "tree", &owner, &owned)).unwrap();

        assert_eq!(merge_observation_sessions(&root).unwrap(), 1);
        let ledger = load_ledger(&root).unwrap();
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(
            ledger.entries[0].path,
            OwnedDataPath::Instance {
                relative: "config/example/database".to_string()
            }
        );
        assert_eq!(ledger.entries[0].owner.as_deref(), Some(owner.as_str()));
        assert!(!session.exists());
    }

    #[test]
    fn prunes_only_missing_ownership_nodes_before_launch() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("bluemap")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/current.toml"), b"present").unwrap();

        let present_tree = OwnedDataPath::Instance {
            relative: "bluemap".to_string(),
        };
        let missing_tree = OwnedDataPath::Instance {
            relative: "generated/removed".to_string(),
        };
        let mut ledger = DataOwnershipLedger::from_entries(vec![
            DataOwnershipEntry {
                path: present_tree.clone(),
                kind: OwnedDataKind::Tree,
                owner: Some("a".repeat(64)),
            },
            DataOwnershipEntry {
                path: OwnedDataPath::Instance {
                    relative: "config/current.toml".to_string(),
                },
                kind: OwnedDataKind::File,
                owner: Some("b".repeat(64)),
            },
            DataOwnershipEntry {
                path: OwnedDataPath::Instance {
                    relative: "config/deleted.toml".to_string(),
                },
                kind: OwnedDataKind::File,
                owner: Some("c".repeat(64)),
            },
            DataOwnershipEntry {
                path: missing_tree.clone(),
                kind: OwnedDataKind::Tree,
                owner: Some("d".repeat(64)),
            },
        ]);
        ledger
            .rebalance_watermarks
            .insert(ownership_path_key(&present_tree), 64);
        ledger
            .rebalance_watermarks
            .insert(ownership_path_key(&missing_tree), 128);
        save_ledger(&root, &ledger).unwrap();

        assert_eq!(prune_missing_ownership(&root).unwrap(), 2);

        let ledger = load_ledger(&root).unwrap();
        assert_eq!(ledger.entries.len(), 2);
        assert!(
            ledger
                .entries
                .iter()
                .any(|entry| entry.path == present_tree)
        );
        assert!(ledger.entries.iter().any(|entry| {
            entry.path
                == OwnedDataPath::Instance {
                    relative: "config/current.toml".to_string(),
                }
        }));
        assert_eq!(
            ledger.rebalance_watermarks,
            BTreeMap::from([(ownership_path_key(&present_tree), 64)])
        );
    }

    #[cfg(unix)]
    #[test]
    fn pruning_preserves_a_broken_symlink_node() {
        use std::os::unix::fs::symlink;

        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let link = root.join("config/broken-link");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink("missing-target", &link).unwrap();
        let path = OwnedDataPath::Instance {
            relative: "config/broken-link".to_string(),
        };
        save_ledger(
            &root,
            &DataOwnershipLedger::from_entries(vec![DataOwnershipEntry {
                path: path.clone(),
                kind: OwnedDataKind::File,
                owner: Some("a".repeat(64)),
            }]),
        )
        .unwrap();

        assert_eq!(prune_missing_ownership(&root).unwrap(), 0);
        assert_eq!(load_ledger(&root).unwrap().entries[0].path, path);
    }

    #[test]
    fn last_writer_replaces_the_previous_owner() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let owner = "a".repeat(64);
        let other = "b".repeat(64);
        save_ledger(
            &root,
            &DataOwnershipLedger::from_entries(vec![DataOwnershipEntry {
                path: OwnedDataPath::Instance {
                    relative: "config/shared.db".to_string(),
                },
                kind: OwnedDataKind::File,
                owner: Some(other),
            }]),
        )
        .unwrap();
        std::fs::write(
            root.join("orbit.lock"),
            format!(
                "[meta]\nmc_version = \"1\"\nmodloader = \"fabric\"\nmodloader_version = \"1\"\n\n[[package]]\nmod_id = \"example\"\nversion = \"1\"\nsha256 = \"{owner}\"\nsha512 = \"{}\"\nfilename = \"example.jar\"\nremotes = [{{ type = \"file\", path = \"example.jar\" }}]\nartifact_sources = [{{ type = \"file\", path = \"example.jar\" }}]\n",
                "c".repeat(128)
            ),
        )
        .unwrap();

        let plan = plan_data_purge(&root, "example", false).unwrap();
        assert!(plan.entries.is_empty());
    }

    #[test]
    fn nested_owner_overrides_parent_and_is_preserved_by_parent_purge() {
        let owner = "a".repeat(64);
        let nested_owner = "b".repeat(64);
        let mut ledger = DataOwnershipLedger::default();
        apply_creation(
            &mut ledger,
            OwnedDataPath::Instance {
                relative: "shaderpacks".to_string(),
            },
            OwnedDataKind::Tree,
            owner.clone(),
        );
        apply_creation(
            &mut ledger,
            OwnedDataPath::Instance {
                relative: "shaderpacks/generated-by-b".to_string(),
            },
            OwnedDataKind::Tree,
            nested_owner.clone(),
        );
        compact_ledger(&mut ledger);

        assert_eq!(
            effective_owner_for_relative(&ledger.entries, Path::new("shaderpacks/user.zip")),
            Some(owner.clone())
        );
        assert_eq!(
            effective_owner_for_relative(
                &ledger.entries,
                Path::new("shaderpacks/generated-by-b/state.bin")
            ),
            Some(nested_owner)
        );
        let plan = build_purge_entries(&ledger, &owner);
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].preserved,
            vec![OwnedDataPath::Instance {
                relative: "shaderpacks/generated-by-b".to_string(),
            }]
        );

        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("shaderpacks/generated-by-b")).unwrap();
        std::fs::write(root.path().join("shaderpacks/user.zip"), b"user content").unwrap();
        std::fs::write(
            root.path().join("shaderpacks/generated-by-b/state.bin"),
            b"nested package content",
        )
        .unwrap();
        remove_owned_path(
            &root.path().join("shaderpacks"),
            OwnedDataKind::Tree,
            &[root.path().join("shaderpacks/generated-by-b")],
        )
        .unwrap();
        assert!(!root.path().join("shaderpacks/user.zip").exists());
        assert_eq!(
            std::fs::read(root.path().join("shaderpacks/generated-by-b/state.bin")).unwrap(),
            b"nested package content"
        );
    }

    #[test]
    fn foreign_write_transfers_only_the_mutated_descendant() {
        let owner = "a".repeat(64);
        let writer = "b".repeat(64);
        let root = OwnedDataPath::Instance {
            relative: "bluemap".to_string(),
        };
        let shared = OwnedDataPath::Instance {
            relative: "bluemap/web/config.js".to_string(),
        };
        let mut ledger = DataOwnershipLedger::default();
        apply_creation(
            &mut ledger,
            root.clone(),
            OwnedDataKind::Tree,
            owner.clone(),
        );
        apply_write(
            &mut ledger,
            shared.clone(),
            OwnedDataKind::File,
            writer.clone(),
        );
        compact_ledger(&mut ledger);

        let shared_entry = ledger
            .entries
            .iter()
            .find(|entry| entry.path == shared)
            .unwrap();
        assert_eq!(shared_entry.owner.as_deref(), Some(writer.as_str()));
        let plan = build_purge_entries(&ledger, &owner);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].path, root);
        assert_eq!(plan[0].preserved, vec![shared]);
    }

    #[test]
    fn first_write_claims_an_unowned_parent_but_not_a_reserved_root() {
        let owner = "a".repeat(64);
        let mut ledger = DataOwnershipLedger::default();
        apply_write(
            &mut ledger,
            OwnedDataPath::Instance {
                relative: "config/example/state.bin".to_string(),
            },
            OwnedDataKind::File,
            owner.clone(),
        );
        compact_ledger(&mut ledger);
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(
            ledger.entries[0].path,
            OwnedDataPath::Instance {
                relative: "config/example".to_string(),
            }
        );
        assert_eq!(ledger.entries[0].kind, OwnedDataKind::Tree);

        let mut reserved = DataOwnershipLedger::default();
        apply_write(
            &mut reserved,
            OwnedDataPath::Instance {
                relative: "config/global-state.bin".to_string(),
            },
            OwnedDataKind::File,
            owner,
        );
        assert_eq!(reserved.entries.len(), 1);
        assert_eq!(reserved.entries[0].kind, OwnedDataKind::File);
    }

    #[test]
    fn directory_rebalance_changes_only_the_compressed_default_owner() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let first = "a".repeat(64);
        let majority = "b".repeat(64);
        let physical = root.join("dynamic-data");
        std::fs::create_dir_all(&physical).unwrap();
        let owned_root = OwnedDataPath::Instance {
            relative: "dynamic-data".to_string(),
        };
        let mut ledger = DataOwnershipLedger::default();
        apply_creation(
            &mut ledger,
            owned_root.clone(),
            OwnedDataKind::Tree,
            first.clone(),
        );
        for index in 0..100 {
            let relative = format!("dynamic-data/{index}.bin");
            std::fs::write(root.join(&relative), [index as u8]).unwrap();
            if index < 70 {
                apply_write(
                    &mut ledger,
                    OwnedDataPath::Instance {
                        relative: relative.clone(),
                    },
                    OwnedDataKind::File,
                    majority.clone(),
                );
            }
        }
        compact_ledger(&mut ledger);
        let before = (0..100)
            .map(|index| {
                effective_owner_for_relative(
                    &ledger.entries,
                    Path::new(&format!("dynamic-data/{index}.bin")),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(ledger.entries.len(), 71);

        let runtime_data = root.join(".orbit/runtime-data");
        std::fs::create_dir_all(&runtime_data).unwrap();
        std::fs::write(runtime_data.join("observation.active"), b"active-session").unwrap();
        rebalance_ownership(&root, &mut ledger).unwrap();
        assert_eq!(ledger.entries.len(), 71);
        std::fs::remove_file(runtime_data.join("observation.active")).unwrap();

        rebalance_ownership(&root, &mut ledger).unwrap();
        let after = (0..100)
            .map(|index| {
                effective_owner_for_relative(
                    &ledger.entries,
                    Path::new(&format!("dynamic-data/{index}.bin")),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(before, after);
        assert_eq!(
            effective_owner(&ledger, &owned_root),
            Some(majority.clone())
        );
        assert_eq!(ledger.entries.len(), 31);
        assert_eq!(
            before
                .iter()
                .filter(|owner| owner.as_deref() == Some(first.as_str()))
                .count(),
            30
        );

        let majority_plan = build_purge_entries(&ledger, &majority);
        assert_eq!(majority_plan.len(), 1);
        assert_eq!(majority_plan[0].path, owned_root);
        assert_eq!(majority_plan[0].preserved.len(), 30);
        let mut after_purge = ledger.clone();
        remove_artifact_references(&mut after_purge, &majority, &majority_plan);
        assert_eq!(
            after_purge
                .entries
                .iter()
                .filter(|entry| entry.owner.as_deref() == Some(first.as_str()))
                .count(),
            30
        );
    }

    #[test]
    fn snapshot_generations_are_ordered_and_replay_is_ignored() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let path = root.join("config/example/state.bin");
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(path.to_string_lossy().as_bytes());
        let owner_a = "a".repeat(64);
        let owner_b = "b".repeat(64);
        let old = root.join("old.events");
        let new = root.join("new.events");
        std::fs::write(
            &old,
            format!(
                "3\tsnapshot\tordered-session\t10\t1\tend\n3\tcreate\tfile\t{owner_a}\t1\t{encoded}\tend\n"
            ),
        )
        .unwrap();
        std::fs::write(
            &new,
            format!(
                "3\tsnapshot\tordered-session\t10\t2\tend\n3\twrite\tfile\t{owner_b}\t2\t{encoded}\tend\n"
            ),
        )
        .unwrap();
        let mut ledger = DataOwnershipLedger::default();
        assert!(
            merge_snapshots_into_ledger(&root, &[new.clone(), old.clone()], &mut ledger).unwrap()
        );
        assert_eq!(
            effective_owner_for_relative(&ledger.entries, Path::new("config/example/state.bin")),
            Some(owner_b.clone())
        );
        assert!(!merge_snapshots_into_ledger(&root, &[old], &mut ledger).unwrap());
        assert_eq!(
            effective_owner_for_relative(&ledger.entries, Path::new("config/example/state.bin")),
            Some(owner_b)
        );
    }

    #[test]
    fn creation_under_same_owner_tree_is_inherited_without_ledger_growth() {
        let owner = "a".repeat(64);
        let mut ledger = DataOwnershipLedger::default();
        apply_creation(
            &mut ledger,
            OwnedDataPath::Instance {
                relative: "bluemap".to_string(),
            },
            OwnedDataKind::Tree,
            owner.clone(),
        );
        for relative in [
            "bluemap/web/index.html",
            "bluemap/web/assets",
            "bluemap/maps/world/tiles/0/0.bin",
        ] {
            apply_creation(
                &mut ledger,
                OwnedDataPath::Instance {
                    relative: relative.to_string(),
                },
                if relative.ends_with("assets") {
                    OwnedDataKind::Tree
                } else {
                    OwnedDataKind::File
                },
                owner.clone(),
            );
        }
        compact_ledger(&mut ledger);
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.entries[0].owner.as_deref(), Some(owner.as_str()));
    }

    #[test]
    fn purge_removes_package_manifest_lock_and_owned_tree() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("mods")).unwrap();
        let bytes = b"top-level package";
        std::fs::write(root.join("mods/example.jar"), bytes).unwrap();
        let sha256 = crate::jar::sha256_digest(bytes);
        let sha512 = crate::jar::sha512_digest(bytes);
        let remote = PackageRemote::File {
            path: "example.jar".to_string(),
        };
        ManifestFile::new(
            root,
            OrbitManifest {
                project: ProjectMeta {
                    name: "test".to_string(),
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                    description: None,
                    authors: None,
                    version: None,
                },
                platform: PlatformSnapshot {
                    minecraft_jar: PlatformArtifact {
                        path: "minecraft.jar".to_string(),
                        sha256: "minecraft".to_string(),
                    },
                    loader_jar: PlatformArtifact {
                        path: "loader.jar".to_string(),
                        sha256: "loader".to_string(),
                    },
                    runtime_jars: Vec::new(),
                    physical_environment: crate::metadata::Environment::Client,
                },
                resolver: ResolverConfig::default(),
                packages: indexmap::IndexMap::from([(
                    "example".to_string(),
                    PackageSpec::new("*", vec![remote.clone()]),
                )]),
                groups: indexmap::IndexMap::new(),
            },
        )
        .save()
        .unwrap();
        Lockfile::new(
            root,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![PackageEntry {
                    mod_id: "example".to_string(),
                    version: "1".to_string(),
                    sha1: String::new(),
                    sha256: sha256.clone(),
                    sha512,
                    filename: "example.jar".to_string(),
                    remotes: vec![remote],
                    artifact_sources: vec![ArtifactSource::File {
                        path: "example.jar".to_string(),
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
        let owned = root.join("config/example/database");
        std::fs::create_dir_all(&owned).unwrap();
        std::fs::write(owned.join("data"), b"runtime data").unwrap();
        save_ledger(
            root,
            &DataOwnershipLedger::from_entries(vec![DataOwnershipEntry {
                path: OwnedDataPath::Instance {
                    relative: "config/example".to_string(),
                },
                kind: OwnedDataKind::Tree,
                owner: Some(sha256),
            }]),
        )
        .unwrap();

        let plan = plan_data_purge(root, "example", false).unwrap();
        assert_eq!(plan.entries.len(), 1);
        let report = apply_data_purge(root, &plan, false).unwrap();
        assert!(report.jar_deleted);
        assert!(!root.join("mods/example.jar").exists());
        assert!(!root.join("config/example").exists());
        assert!(ManifestFile::open(root).unwrap().inner.packages.is_empty());
        assert!(Lockfile::open(root).unwrap().inner.packages.is_empty());
    }

    #[test]
    fn dry_run_reads_pending_snapshots_without_consuming_or_persisting_them() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let owner = "a".repeat(64);
        std::fs::write(
            root.join("orbit.lock"),
            format!(
                "[meta]\nmc_version = \"1\"\nmodloader = \"fabric\"\nmodloader_version = \"1\"\n\n[[package]]\nmod_id = \"example\"\nversion = \"1\"\nsha256 = \"{owner}\"\nsha512 = \"{}\"\nfilename = \"example.jar\"\nremotes = [{{ type = \"file\", path = \"example.jar\" }}]\nartifact_sources = [{{ type = \"file\", path = \"example.jar\" }}]\n",
                "c".repeat(128)
            ),
        )
        .unwrap();
        let session = observation_session_path(&root).unwrap();
        std::fs::write(
            &session,
            observation_line(
                "create",
                "tree",
                &owner,
                &root.join("config/example/database"),
            ),
        )
        .unwrap();

        let plan = plan_data_purge(&root, "example", true).unwrap();
        assert_eq!(plan.entries.len(), 1);
        assert!(session.exists());
        assert!(!ledger_path(&root).exists());
    }
}
