//! Runtime-observed mutable-data ownership for managed packages.
//!
//! The Java agent writes crash-tolerant session snapshots. Orbit merges those
//! snapshots into an instance-local ledger and maps the recorded top-level JAR
//! hashes back to logical packages through `orbit.lock`.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::{OrbitError, RuntimeDataError};
use crate::installer::{RemoveReport, remove_from_instance};
use crate::workspace::Lockfile;

const RUNTIME_DATA_DIRECTORY: &str = ".orbit/runtime-data";
const LEDGER_FILE: &str = "ownership.toml";
const SESSIONS_DIRECTORY: &str = "sessions";
const LEDGER_SCHEMA: u32 = 1;

const PROTECTED_INSTANCE_ROOTS: &[&str] = &[
    ".orbit",
    "assets",
    "config",
    "libraries",
    "logs",
    "mods",
    "natives",
    "resourcepacks",
    "saves",
    "screenshots",
    "shaderpacks",
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
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub created_by: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub readers: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub writers: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataOwnershipLedger {
    schema: u32,
    #[serde(default)]
    entries: Vec<DataOwnershipEntry>,
}

impl Default for DataOwnershipLedger {
    fn default() -> Self {
        Self {
            schema: LEDGER_SCHEMA,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DataPurgeEntry {
    pub path: OwnedDataPath,
    pub kind: OwnedDataKind,
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
    merge_snapshots_into_ledger(&root, &claimed, &mut ledger)?;
    compact_ledger(&mut ledger);
    save_ledger(&root, &ledger)?;
    for path in &claimed {
        std::fs::remove_file(path)?;
    }
    Ok(claimed.len())
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
) -> Result<(), OrbitError> {
    for path in snapshots {
        let document = std::fs::read_to_string(path)?;
        for (index, line) in document.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let observation = parse_observation(line).map_err(|detail| {
                OrbitError::RuntimeData(RuntimeDataError::InvalidObservation {
                    path: path.display().to_string(),
                    line: index + 1,
                    detail,
                })
            })?;
            merge_observation(instance_dir, ledger, observation)?;
        }
    }
    Ok(())
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
        merge_snapshots_into_ledger(&root, &snapshots, &mut ledger)?;
        compact_ledger(&mut ledger);
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
    let mut entries = ledger
        .entries
        .into_iter()
        .filter(|owned| {
            owned.created_by.len() == 1
                && owned.created_by.contains(&artifact_sha256)
                && owned.writers.len() == 1
                && owned.writers.contains(&artifact_sha256)
        })
        .map(|owned| DataPurgeEntry {
            path: owned.path,
            kind: owned.kind,
        })
        .collect::<Vec<_>>();
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
        if let Err(error) = remove_owned_path(&path, selected.kind) {
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
    instance_dir.canonicalize().map_err(OrbitError::from)
}

fn ledger_path(instance_dir: &Path) -> PathBuf {
    instance_dir.join(RUNTIME_DATA_DIRECTORY).join(LEDGER_FILE)
}

fn load_ledger(instance_dir: &Path) -> Result<DataOwnershipLedger, OrbitError> {
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

fn save_ledger(instance_dir: &Path, ledger: &DataOwnershipLedger) -> Result<(), OrbitError> {
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

struct Observation {
    kind: OwnedDataKind,
    created: bool,
    read: bool,
    write: bool,
    owner: String,
    absolute: PathBuf,
}

fn parse_observation(line: &str) -> Result<Observation, String> {
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 6 || fields[0] != "1" || fields[5] != "end" {
        return Err("expected a complete v1 record".to_string());
    }
    let kind = match fields[1] {
        "file" => OwnedDataKind::File,
        "tree" => OwnedDataKind::Tree,
        value => return Err(format!("unknown path kind '{value}'")),
    };
    let flags = fields[2].as_bytes();
    if flags.len() != 3 || flags.iter().any(|flag| !matches!(flag, b'0' | b'1')) {
        return Err("invalid created/read/write flags".to_string());
    }
    if fields[3].len() != 64 || !fields[3].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid owner SHA-256".to_string());
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(fields[4])
        .map_err(|error| format!("invalid path encoding: {error}"))?;
    let path = String::from_utf8(decoded).map_err(|error| format!("path is not UTF-8: {error}"))?;
    let absolute = PathBuf::from(path);
    if !absolute.is_absolute() {
        return Err("observed path is not absolute".to_string());
    }
    Ok(Observation {
        kind,
        created: flags[0] == b'1',
        read: flags[1] == b'1',
        write: flags[2] == b'1',
        owner: fields[3].to_ascii_lowercase(),
        absolute,
    })
}

fn merge_observation(
    instance_dir: &Path,
    ledger: &mut DataOwnershipLedger,
    observation: Observation,
) -> Result<(), OrbitError> {
    let path = owned_path(instance_dir, &observation.absolute)?;
    if protected_control_path(&path) {
        return Ok(());
    }
    if observation.kind == OwnedDataKind::File {
        if let Some(parent) = ledger.entries.iter_mut().find(|entry| {
            entry.kind == OwnedDataKind::Tree && contains_path(instance_dir, &entry.path, &path)
        }) {
            merge_owners(parent, &observation);
            return Ok(());
        }
    }
    if let Some(existing) = ledger
        .entries
        .iter_mut()
        .find(|entry| entry.path == path && entry.kind == observation.kind)
    {
        merge_owners(existing, &observation);
        return Ok(());
    }
    let mut entry = DataOwnershipEntry {
        path,
        kind: observation.kind,
        created_by: BTreeSet::new(),
        readers: BTreeSet::new(),
        writers: BTreeSet::new(),
    };
    merge_owners(&mut entry, &observation);
    ledger.entries.push(entry);
    Ok(())
}

fn merge_owners(entry: &mut DataOwnershipEntry, observation: &Observation) {
    if observation.created {
        entry.created_by.insert(observation.owner.clone());
    }
    if observation.read {
        entry.readers.insert(observation.owner.clone());
    }
    if observation.write {
        entry.writers.insert(observation.owner.clone());
    }
}

fn compact_ledger(ledger: &mut DataOwnershipLedger) {
    let tree_indices = ledger
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.kind == OwnedDataKind::Tree)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut remove = BTreeSet::new();
    for tree_index in tree_indices {
        if remove.contains(&tree_index) {
            continue;
        }
        let tree_path = ledger.entries[tree_index].path.clone();
        for child_index in 0..ledger.entries.len() {
            if child_index == tree_index || remove.contains(&child_index) {
                continue;
            }
            if path_contains(&tree_path, &ledger.entries[child_index].path) {
                let child = ledger.entries[child_index].clone();
                ledger.entries[tree_index]
                    .created_by
                    .extend(child.created_by);
                ledger.entries[tree_index].readers.extend(child.readers);
                ledger.entries[tree_index].writers.extend(child.writers);
                remove.insert(child_index);
            }
        }
    }
    ledger.entries = ledger
        .entries
        .drain(..)
        .enumerate()
        .filter_map(|(index, entry)| (!remove.contains(&index)).then_some(entry))
        .collect();
    ledger.entries.sort_by(|left, right| {
        left.path
            .display()
            .cmp(right.path.display())
            .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
    });
}

fn owned_path(instance_dir: &Path, absolute: &Path) -> Result<OwnedDataPath, OrbitError> {
    let absolute = absolute.to_path_buf();
    if let Ok(relative) = absolute.strip_prefix(instance_dir) {
        validate_relative(relative)?;
        return Ok(OwnedDataPath::Instance {
            relative: relative.to_string_lossy().replace('\\', "/"),
        });
    }
    Ok(OwnedDataPath::External {
        absolute: absolute.to_string_lossy().into_owned(),
    })
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

fn contains_path(instance_dir: &Path, parent: &OwnedDataPath, child: &OwnedDataPath) -> bool {
    let Ok(parent) = resolve_owned_path(instance_dir, parent) else {
        return false;
    };
    let Ok(child) = resolve_owned_path(instance_dir, child) else {
        return false;
    };
    child != parent && child.starts_with(parent)
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
                && PROTECTED_INSTANCE_ROOTS.contains(&relative.as_str())
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

fn remove_owned_path(path: &Path, kind: OwnedDataKind) -> Result<(), std::io::Error> {
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
            OwnedDataKind::Tree => remove_tree_without_following_links(path),
        };
    }
    Ok(())
}

fn remove_tree_without_following_links(path: &Path) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let metadata = std::fs::symlink_metadata(&child)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            remove_tree_without_following_links(&child)?;
        } else {
            std::fs::remove_file(&child)?;
        }
    }
    std::fs::remove_dir(path)
}

fn remove_artifact_references(
    ledger: &mut DataOwnershipLedger,
    artifact_sha256: &str,
    removed: &[DataPurgeEntry],
) {
    ledger.entries.retain_mut(|entry| {
        let was_removed = removed
            .iter()
            .any(|item| item.path == entry.path && item.kind == entry.kind);
        if was_removed {
            return false;
        }
        entry.created_by.remove(artifact_sha256);
        entry.readers.remove(artifact_sha256);
        entry.writers.remove(artifact_sha256);
        !(entry.created_by.is_empty() && entry.readers.is_empty() && entry.writers.is_empty())
    });
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

    fn observation_line(kind: &str, flags: &str, owner: &str, path: &Path) -> String {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(path.to_string_lossy().as_bytes());
        format!("1\t{kind}\t{flags}\t{owner}\t{encoded}\tend\n")
    }

    #[test]
    fn parses_complete_observation_records() {
        let absolute = std::env::temp_dir().join("orbit-runtime-data-parse");
        let path = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(absolute.to_string_lossy().as_bytes());
        let record =
            parse_observation(&format!("1\ttree\t101\t{}\t{path}\tend", "a".repeat(64))).unwrap();
        assert_eq!(record.kind, OwnedDataKind::Tree);
        assert!(record.created);
        assert!(!record.read);
        assert!(record.write);
    }

    #[test]
    fn rejects_truncated_observation_records() {
        assert!(parse_observation("1\tfile\t111").is_err());
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
        std::fs::write(&session, observation_line("file", "101", &owner, &owned)).unwrap();
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
        remove_owned_path(&tree, OwnedDataKind::Tree).unwrap();
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
        std::fs::write(&session, observation_line("tree", "101", &owner, &owned)).unwrap();

        assert_eq!(merge_observation_sessions(&root).unwrap(), 1);
        let ledger = load_ledger(&root).unwrap();
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(
            ledger.entries[0].path,
            OwnedDataPath::Instance {
                relative: "config/example/database".to_string()
            }
        );
        assert!(ledger.entries[0].created_by.contains(&owner));
        assert!(!session.exists());
    }

    #[test]
    fn shared_writers_are_not_purgeable() {
        let directory = instance();
        let root = directory.path().canonicalize().unwrap();
        let owner = "a".repeat(64);
        let other = "b".repeat(64);
        save_ledger(
            &root,
            &DataOwnershipLedger {
                schema: LEDGER_SCHEMA,
                entries: vec![DataOwnershipEntry {
                    path: OwnedDataPath::Instance {
                        relative: "config/shared.db".to_string(),
                    },
                    kind: OwnedDataKind::File,
                    created_by: BTreeSet::from([owner.clone()]),
                    readers: BTreeSet::new(),
                    writers: BTreeSet::from([owner.clone(), other]),
                }],
            },
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
            &DataOwnershipLedger {
                schema: LEDGER_SCHEMA,
                entries: vec![DataOwnershipEntry {
                    path: OwnedDataPath::Instance {
                        relative: "config/example".to_string(),
                    },
                    kind: OwnedDataKind::Tree,
                    created_by: BTreeSet::from([sha256.clone()]),
                    readers: BTreeSet::new(),
                    writers: BTreeSet::from([sha256]),
                }],
            },
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
            observation_line("tree", "101", &owner, &root.join("config/example/database")),
        )
        .unwrap();

        let plan = plan_data_purge(&root, "example", true).unwrap();
        assert_eq!(plan.entries.len(), 1);
        assert!(session.exists());
        assert!(!ledger_path(&root).exists());
    }
}
