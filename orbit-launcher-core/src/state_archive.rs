//! Portable mutable Minecraft state owned by Orbit Launcher.
//!
//! Runtime artifacts remain reproducible from official metadata. This module
//! carries only state that cannot be recreated: client saves/preferences and
//! dedicated-server worlds/operator configuration. Mod packages and their
//! configuration remain Orbit's responsibility.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::error::LauncherError;
use crate::instance::{InstanceKind, ManifestFile};
use crate::lockfile::LockFile;

const STATE_DIRECTORY: &str = ".orbit-launcher";
const LAUNCHER_PREFIX: &str = "launcher/";
const SERVER_STATE_FILES: [&str; 6] = [
    "server.properties",
    "whitelist.json",
    "ops.json",
    "banned-players.json",
    "banned-ips.json",
    "server-icon.png",
];
const CLIENT_STATE_FILES: [&str; 2] = ["options.txt", "servers.dat"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateArchiveProgressEvent {
    Started {
        files: usize,
        total_bytes: u64,
    },
    Advanced {
        completed_bytes: u64,
        total_bytes: u64,
    },
    Finished {
        files: usize,
        total_bytes: u64,
    },
}

#[derive(Debug, Clone)]
pub struct LauncherStateExportReport {
    pub path: PathBuf,
    pub kind: InstanceKind,
    pub minecraft_version: String,
    pub files: usize,
    pub bytes: u64,
    pub world_files: usize,
}

#[derive(Debug, Clone)]
pub struct LauncherStateArchiveSummary {
    pub kind: InstanceKind,
    pub minecraft_version: String,
    pub files: usize,
    pub bytes: u64,
    pub world_files: usize,
}

#[derive(Debug, Clone)]
pub struct LauncherStateRestoreReport {
    pub kind: InstanceKind,
    pub source_minecraft_version: String,
    pub target_minecraft_version: String,
    pub files: usize,
    pub bytes: u64,
    pub world_files: usize,
    pub restored_properties: usize,
    pub skipped_properties: Vec<String>,
}

#[derive(Debug, Clone)]
struct StateArchiveManifest {
    kind: InstanceKind,
    minecraft_version: String,
    files: Vec<StateArchiveFile>,
}

#[derive(Debug, Clone)]
struct StateArchiveFile {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug)]
struct StateSource {
    source: PathBuf,
    archive_path: String,
    bytes: u64,
}

/// Export the selected instance's mutable game state as a Launcher-owned
/// projection in an Orbit bundle.
pub fn export_launcher_state<F>(
    instance_root: &Path,
    output: &Path,
    progress: F,
) -> Result<LauncherStateExportReport, LauncherError>
where
    F: FnMut(StateArchiveProgressEvent),
{
    export_launcher_state_with_base(instance_root, None, output, progress)
}

/// Add Launcher-owned mutable state to a new bundle or to an existing
/// Orbit-runtime bundle without interpreting or rewriting the Orbit section.
pub fn export_launcher_state_with_base<F>(
    instance_root: &Path,
    base: Option<&Path>,
    output: &Path,
    mut progress: F,
) -> Result<LauncherStateExportReport, LauncherError>
where
    F: FnMut(StateArchiveProgressEvent),
{
    let manifest = ManifestFile::open(instance_root)?.inner;
    let lock = LockFile::open(instance_root)?.inner;
    if manifest.kind != lock.kind {
        return Err(LauncherError::InvalidLock(
            "instance manifest and lock disagree about client/server kind".to_string(),
        ));
    }
    if output.exists() && base != Some(output) {
        return Err(LauncherError::Transaction(format!(
            "refusing to overwrite Orbit bundle '{}'",
            output.display()
        )));
    }

    let sources = collect_state_sources(instance_root, manifest.kind)?;
    let total_bytes = sources.iter().try_fold(0_u64, |total, source| {
        total
            .checked_add(source.bytes)
            .ok_or_else(|| LauncherError::Transaction("Launcher state size overflowed".to_string()))
    })?;
    let world_files = sources
        .iter()
        .filter(|source| {
            source.archive_path.starts_with("world/") || source.archive_path.starts_with("saves/")
        })
        .count();
    progress(StateArchiveProgressEvent::Started {
        files: sources.len(),
        total_bytes,
    });

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temporary_output_path(output);
    if temporary.exists() {
        std::fs::remove_file(&temporary)?;
    }
    let result = write_state_archive(
        &temporary,
        &manifest,
        &lock,
        base,
        &sources,
        total_bytes,
        &mut progress,
    );
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = replace_output(&temporary, output) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    progress(StateArchiveProgressEvent::Finished {
        files: sources.len(),
        total_bytes,
    });
    Ok(LauncherStateExportReport {
        path: output.to_path_buf(),
        kind: manifest.kind,
        minecraft_version: lock.minecraft.version,
        files: sources.len(),
        bytes: total_bytes,
        world_files,
    })
}

pub fn inspect_launcher_state(source: &Path) -> Result<LauncherStateArchiveSummary, LauncherError> {
    if !source.is_file() {
        return Err(LauncherError::Transaction(format!(
            "Launcher state archive '{}' does not exist",
            source.display()
        )));
    }
    let (_, manifest) = open_state_archive(source)?;
    let bytes = manifest.files.iter().try_fold(0_u64, |total, file| {
        total.checked_add(file.bytes).ok_or_else(|| {
            LauncherError::InvalidRemoteData("Launcher state size overflowed".to_string())
        })
    })?;
    let world_files = manifest
        .files
        .iter()
        .filter(|file| file.path.starts_with("world/") || file.path.starts_with("saves/"))
        .count();
    Ok(LauncherStateArchiveSummary {
        kind: manifest.kind,
        minecraft_version: manifest.minecraft_version,
        files: manifest.files.len(),
        bytes,
        world_files,
    })
}

/// Apply a Launcher state archive to an already installed target instance.
///
/// Server properties are never copied verbatim across versions. The target's
/// Mojang-generated property set is authoritative; source values are applied
/// only to keys that still exist in the target version.
pub fn restore_launcher_state<F>(
    target_root: &Path,
    source: &Path,
    mut progress: F,
) -> Result<LauncherStateRestoreReport, LauncherError>
where
    F: FnMut(StateArchiveProgressEvent),
{
    if !source.is_file() {
        return Err(LauncherError::Transaction(format!(
            "Launcher state archive '{}' does not exist",
            source.display()
        )));
    }
    let target_manifest = ManifestFile::open(target_root)?.inner;
    let target_lock = LockFile::open(target_root)?.inner;
    if target_manifest.kind != target_lock.kind {
        return Err(LauncherError::InvalidLock(
            "target manifest and lock disagree about client/server kind".to_string(),
        ));
    }

    let (_, archive_manifest) = open_state_archive(source)?;
    let file = std::fs::File::open(source)?;
    let mut archive = zip::ZipArchive::new(file).map_err(invalid_archive)?;
    if archive_manifest.kind != target_manifest.kind {
        return Err(LauncherError::Transaction(format!(
            "cannot install {} state into a {} instance",
            archive_manifest.kind.as_str(),
            target_manifest.kind.as_str()
        )));
    }
    let total_bytes = archive_manifest
        .files
        .iter()
        .try_fold(0_u64, |total, file| {
            total.checked_add(file.bytes).ok_or_else(|| {
                LauncherError::InvalidRemoteData("Launcher state size overflowed".to_string())
            })
        })?;
    progress(StateArchiveProgressEvent::Started {
        files: archive_manifest.files.len(),
        total_bytes,
    });

    let transaction = StateRestoreTransaction::begin(target_root)?;
    let mut source_properties = None;
    if archive_manifest.kind == InstanceKind::Server
        && archive_manifest
            .files
            .iter()
            .any(|file| file.path == "state/server.properties")
    {
        source_properties = Some(read_verified_entry(
            &mut archive,
            archive_manifest
                .files
                .iter()
                .find(|file| file.path == "state/server.properties")
                .expect("entry existence checked"),
        )?);
    }

    let mut skipped_properties = Vec::new();
    let mut restored_properties = 0;
    let mut staged_property_bytes = 0_u64;
    let target_world = if target_manifest.kind == InstanceKind::Server {
        let properties_path = target_root.join("server.properties");
        if !properties_path.is_file() {
            return Err(LauncherError::Transaction(
                "target server.properties is missing; install the target runtime before restoring state"
                    .to_string(),
            ));
        }
        let mut target_properties = read_properties(&std::fs::read(&properties_path)?, "target")?;
        if let Some(source_properties) = source_properties.as_deref() {
            let source_properties = read_properties(source_properties, "source archive")?;
            for (key, value) in source_properties {
                if let Some(target_value) = target_properties.get_mut(&key) {
                    *target_value = value;
                    restored_properties += 1;
                } else {
                    skipped_properties.push(key);
                }
            }
        }
        skipped_properties.sort();
        let target_world = server_world_relative(&target_properties)?;
        if source_properties.is_some() {
            let merged = write_properties(&target_properties)?;
            staged_property_bytes = u64::try_from(merged.len()).unwrap_or(u64::MAX);
            transaction.stage_bytes(Path::new("server.properties"), &merged)?;
        }
        Some(target_world)
    } else {
        None
    };

    let source_property_bytes = archive_manifest
        .files
        .iter()
        .find(|file| file.path == "state/server.properties")
        .map_or(0, |file| file.bytes);
    let mut completed_bytes = source_property_bytes;
    if completed_bytes > 0 {
        progress(StateArchiveProgressEvent::Advanced {
            completed_bytes,
            total_bytes,
        });
    }
    let mut staged_files = usize::from(source_properties.is_some());
    let mut staged_bytes = staged_property_bytes;
    let mut world_files = 0;
    for entry in &archive_manifest.files {
        let destination =
            destination_relative(target_manifest.kind, &entry.path, target_world.as_deref())?;
        if entry.path == "state/server.properties" {
            continue;
        } else {
            let mut zip_entry = archive
                .by_name(&format!("{LAUNCHER_PREFIX}{}", entry.path))
                .map_err(invalid_archive)?;
            if !zip_entry.is_file() {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "Launcher state entry '{}' is not a regular file",
                    entry.path
                )));
            }
            transaction.stage_reader(&destination, &mut zip_entry, entry, |advanced| {
                completed_bytes = completed_bytes.saturating_add(advanced);
                progress(StateArchiveProgressEvent::Advanced {
                    completed_bytes,
                    total_bytes,
                });
            })?;
            staged_files += 1;
            staged_bytes = staged_bytes.saturating_add(entry.bytes);
            if entry.path.starts_with("world/") || entry.path.starts_with("saves/") {
                world_files += 1;
            }
        }
    }
    if completed_bytes < total_bytes {
        progress(StateArchiveProgressEvent::Advanced {
            completed_bytes: total_bytes,
            total_bytes,
        });
    }
    transaction.commit(target_manifest.kind == InstanceKind::Server)?;
    progress(StateArchiveProgressEvent::Finished {
        files: archive_manifest.files.len(),
        total_bytes,
    });
    Ok(LauncherStateRestoreReport {
        kind: target_manifest.kind,
        source_minecraft_version: archive_manifest.minecraft_version,
        target_minecraft_version: target_lock.minecraft.version,
        files: staged_files,
        bytes: staged_bytes,
        world_files,
        restored_properties,
        skipped_properties,
    })
}

fn collect_state_sources(
    instance_root: &Path,
    kind: InstanceKind,
) -> Result<Vec<StateSource>, LauncherError> {
    let mut sources = Vec::new();
    let state_files = match kind {
        InstanceKind::Client => CLIENT_STATE_FILES.as_slice(),
        InstanceKind::Server => SERVER_STATE_FILES.as_slice(),
    };
    for name in state_files {
        let source = instance_root.join(name);
        if source.exists() {
            collect_file(
                instance_root,
                &source,
                &format!("state/{name}"),
                &mut sources,
            )?;
        }
    }
    match kind {
        InstanceKind::Client => {
            let saves = instance_root.join("saves");
            if saves.exists() {
                collect_directory(instance_root, &saves, &saves, "saves", &mut sources)?;
            }
        }
        InstanceKind::Server => {
            let properties = if instance_root.join("server.properties").is_file() {
                read_properties(
                    &std::fs::read(instance_root.join("server.properties"))?,
                    "server.properties",
                )?
            } else {
                HashMap::new()
            };
            let world_relative = server_world_relative(&properties)?;
            let world = instance_root.join(&world_relative);
            if world.exists() {
                collect_directory(instance_root, &world, &world, "world", &mut sources)?;
            }
        }
    }
    sources.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));
    Ok(sources)
}

fn collect_file(
    instance_root: &Path,
    source: &Path,
    archive_path: &str,
    sources: &mut Vec<StateSource>,
) -> Result<(), LauncherError> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(LauncherError::Transaction(format!(
            "Launcher state contains a symbolic link: '{}'",
            source.display()
        )));
    }
    if !metadata.is_file() {
        return Err(LauncherError::Transaction(format!(
            "Launcher state path '{}' is not a regular file",
            source.display()
        )));
    }
    let canonical_root = dunce::canonicalize(instance_root)?;
    let canonical_source = dunce::canonicalize(source)?;
    if !canonical_source.starts_with(&canonical_root) {
        return Err(LauncherError::Transaction(format!(
            "Launcher state path '{}' escapes its instance directory",
            source.display()
        )));
    }
    sources.push(StateSource {
        source: source.to_path_buf(),
        archive_path: archive_path.to_string(),
        bytes: metadata.len(),
    });
    Ok(())
}

fn collect_directory(
    instance_root: &Path,
    traversal_root: &Path,
    directory: &Path,
    archive_root: &str,
    sources: &mut Vec<StateSource>,
) -> Result<(), LauncherError> {
    let metadata = std::fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Transaction(format!(
            "Launcher state directory '{}' is not a regular directory",
            directory.display()
        )));
    }
    let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(LauncherError::Transaction(format!(
                "Launcher state contains a symbolic link: '{}'",
                path.display()
            )));
        }
        if metadata.is_dir() {
            collect_directory(instance_root, traversal_root, &path, archive_root, sources)?;
        } else if metadata.is_file() {
            let logical_relative = path.strip_prefix(traversal_root).map_err(|_| {
                LauncherError::Transaction(
                    "Launcher state traversal escaped its source directory".to_string(),
                )
            })?;
            let archive_path = format!("{archive_root}/{}", portable_path(logical_relative));
            collect_file(instance_root, &path, &archive_path, sources)?;
        } else {
            return Err(LauncherError::Transaction(format!(
                "Launcher state contains an unsupported filesystem object: '{}'",
                path.display()
            )));
        }
    }
    Ok(())
}

fn write_state_archive<F>(
    output: &Path,
    instance: &crate::instance::InstanceManifest,
    lock: &crate::lockfile::LauncherLock,
    base: Option<&Path>,
    sources: &[StateSource],
    total_bytes: u64,
    progress: &mut F,
) -> Result<(), LauncherError>
where
    F: FnMut(StateArchiveProgressEvent),
{
    let file = std::fs::File::create(output)?;
    let mut archive = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let mut bundle = if let Some(base) = base {
        let bundle = orbit_bundle_format::BundleArchive::open(base)?;
        bundle.verify()?;
        validate_bundle_runtime(&bundle.manifest, instance.kind, lock)?;
        if bundle.manifest.launcher.as_ref().is_some_and(|section| {
            section.content == orbit_bundle_format::LauncherContent::RuntimeAndState
        }) {
            return Err(LauncherError::Transaction(
                "base bundle already contains Launcher state".to_string(),
            ));
        }
        let mut input =
            zip::ZipArchive::new(std::fs::File::open(base)?).map_err(invalid_archive)?;
        for index in 0..input.len() {
            let entry = input.by_index(index).map_err(invalid_archive)?;
            if !entry.is_file() || entry.name() == orbit_bundle_format::BUNDLE_MANIFEST_PATH {
                continue;
            }
            archive.raw_copy_file(entry).map_err(write_archive_error)?;
        }
        bundle.manifest
    } else {
        let target = match instance.kind {
            InstanceKind::Client => orbit_bundle_format::InstanceTarget::Client,
            InstanceKind::Server => orbit_bundle_format::InstanceTarget::Server,
        };
        orbit_bundle_format::BundleManifest {
            format_version: orbit_bundle_format::BUNDLE_FORMAT_VERSION,
            id: instance.id.to_string(),
            name: instance.name.clone(),
            version: lock.minecraft.version.clone(),
            summary: None,
            targets: vec![target],
            runtime: orbit_bundle_format::RuntimeRequirement {
                minecraft: lock.minecraft.version.clone(),
                loader: lock.loader.kind.as_str().to_string(),
                loader_version: lock.loader.version.clone(),
            },
            launcher: None,
            orbit: None,
            files: Vec::new(),
        }
    };
    bundle
        .files
        .retain(|file| file.owner != orbit_bundle_format::BundleFileOwner::Launcher);
    let mut files = Vec::with_capacity(sources.len());
    let mut completed_bytes = 0_u64;
    let mut buffer = vec![0_u8; 128 * 1024];
    for source in sources {
        let archive_path = format!("{LAUNCHER_PREFIX}{}", source.archive_path);
        archive
            .start_file(&archive_path, options)
            .map_err(write_archive_error)?;
        let mut input = std::fs::File::open(&source.source)?;
        let mut hasher = Sha256::new();
        let mut written = 0_u64;
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            archive.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            let read = u64::try_from(read).expect("buffer length fits u64");
            written = written.saturating_add(read);
            completed_bytes = completed_bytes.saturating_add(read);
            progress(StateArchiveProgressEvent::Advanced {
                completed_bytes,
                total_bytes,
            });
        }
        if written != source.bytes {
            return Err(LauncherError::Transaction(format!(
                "Launcher state file '{}' changed while it was exported",
                source.source.display()
            )));
        }
        files.push(StateArchiveFile {
            path: archive_path,
            bytes: written,
            sha256: hex::encode(hasher.finalize()),
        });
    }
    bundle.launcher = Some(orbit_bundle_format::LauncherSection {
        content: orbit_bundle_format::LauncherContent::RuntimeAndState,
    });
    bundle.files.extend(
        files
            .into_iter()
            .map(|file| orbit_bundle_format::BundleFile {
                path: file.path,
                owner: orbit_bundle_format::BundleFileOwner::Launcher,
                size: file.bytes,
                sha256: file.sha256,
            }),
    );
    bundle.validate()?;
    archive
        .start_file(orbit_bundle_format::BUNDLE_MANIFEST_PATH, options)
        .map_err(write_archive_error)?;
    archive.write_all(toml::to_string_pretty(&bundle)?.as_bytes())?;
    archive.finish().map_err(write_archive_error)?;
    Ok(())
}

fn validate_bundle_runtime(
    bundle: &orbit_bundle_format::BundleManifest,
    kind: InstanceKind,
    lock: &crate::lockfile::LauncherLock,
) -> Result<(), LauncherError> {
    let target = match kind {
        InstanceKind::Client => orbit_bundle_format::InstanceTarget::Client,
        InstanceKind::Server => orbit_bundle_format::InstanceTarget::Server,
    };
    if bundle.runtime.minecraft != lock.minecraft.version
        || bundle.runtime.loader != lock.loader.kind.as_str()
        || bundle.runtime.loader_version != lock.loader.version
        || !bundle.targets.contains(&target)
    {
        return Err(LauncherError::Transaction(
            "base Orbit bundle runtime does not match the Launcher instance".to_string(),
        ));
    }
    Ok(())
}

fn open_state_archive(
    source: &Path,
) -> Result<(orbit_bundle_format::BundleArchive, StateArchiveManifest), LauncherError> {
    let bundle = orbit_bundle_format::BundleArchive::open(source)?;
    let launcher = bundle.manifest.launcher.as_ref().ok_or_else(|| {
        LauncherError::InvalidRemoteData(
            "Orbit bundle contains no Launcher-owned content".to_string(),
        )
    })?;
    if launcher.content != orbit_bundle_format::LauncherContent::RuntimeAndState {
        return Err(LauncherError::InvalidRemoteData(
            "Orbit bundle contains runtime metadata but no Launcher state".to_string(),
        ));
    }
    let kind = match bundle.manifest.targets.as_slice() {
        [orbit_bundle_format::InstanceTarget::Client] => InstanceKind::Client,
        [orbit_bundle_format::InstanceTarget::Server] => InstanceKind::Server,
        _ => {
            return Err(LauncherError::InvalidRemoteData(
                "Launcher state requires exactly one client or server target".to_string(),
            ));
        }
    };
    let mut paths = HashSet::new();
    let mut files = Vec::new();
    for file in bundle
        .manifest
        .files
        .iter()
        .filter(|file| file.owner == orbit_bundle_format::BundleFileOwner::Launcher)
    {
        let path = file.path.strip_prefix(LAUNCHER_PREFIX).ok_or_else(|| {
            LauncherError::InvalidRemoteData(format!(
                "Launcher bundle path '{}' is outside its namespace",
                file.path
            ))
        })?;
        validate_archive_path(kind, path)?;
        if !paths.insert(path.to_string()) {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Launcher state archive contains duplicate path '{}'",
                path
            )));
        }
        files.push(StateArchiveFile {
            path: path.to_string(),
            bytes: file.size,
            sha256: file.sha256.clone(),
        });
    }
    let minecraft_version = bundle.manifest.runtime.minecraft.clone();
    Ok((
        bundle,
        StateArchiveManifest {
            kind,
            minecraft_version,
            files,
        },
    ))
}

fn validate_archive_path(kind: InstanceKind, path: &str) -> Result<(), LauncherError> {
    let allowed_state = match kind {
        InstanceKind::Client => CLIENT_STATE_FILES.as_slice(),
        InstanceKind::Server => SERVER_STATE_FILES.as_slice(),
    };
    if let Some(name) = path.strip_prefix("state/") {
        if allowed_state.contains(&name) && !name.contains('/') {
            return Ok(());
        }
    } else {
        let root = match kind {
            InstanceKind::Client => "saves/",
            InstanceKind::Server => "world/",
        };
        if let Some(relative) = path.strip_prefix(root) {
            validate_relative_path(Path::new(relative), "archive entry")?;
            return Ok(());
        }
    }
    Err(LauncherError::InvalidRemoteData(format!(
        "unsupported Launcher state archive path '{path}'"
    )))
}

fn destination_relative(
    kind: InstanceKind,
    archive_path: &str,
    server_world: Option<&Path>,
) -> Result<PathBuf, LauncherError> {
    validate_archive_path(kind, archive_path)?;
    if let Some(name) = archive_path.strip_prefix("state/") {
        return Ok(PathBuf::from(name));
    }
    match kind {
        InstanceKind::Client => Ok(Path::new("saves").join(
            archive_path
                .strip_prefix("saves/")
                .expect("client archive path was validated"),
        )),
        InstanceKind::Server => Ok(server_world
            .expect("server target world was resolved")
            .join(
                archive_path
                    .strip_prefix("world/")
                    .expect("server archive path was validated"),
            )),
    }
}

fn read_verified_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    expected: &StateArchiveFile,
) -> Result<Vec<u8>, LauncherError> {
    let mut entry = archive
        .by_name(&format!("{LAUNCHER_PREFIX}{}", expected.path))
        .map_err(invalid_archive)?;
    let mut bytes = Vec::new();
    verify_entry_reader(&mut entry, expected, |chunk| bytes.extend_from_slice(chunk))?;
    Ok(bytes)
}

fn verify_entry_reader<R, F>(
    reader: &mut R,
    expected: &StateArchiveFile,
    mut consume: F,
) -> Result<(), LauncherError>
where
    R: Read,
    F: FnMut(&[u8]),
{
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        consume(&buffer[..read]);
        bytes = bytes.saturating_add(u64::try_from(read).expect("buffer length fits u64"));
    }
    let digest = hex::encode(hasher.finalize());
    if bytes != expected.bytes || !digest.eq_ignore_ascii_case(&expected.sha256) {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "Launcher state entry '{}' failed size or SHA-256 verification",
            expected.path
        )));
    }
    Ok(())
}

fn read_properties(bytes: &[u8], subject: &str) -> Result<HashMap<String, String>, LauncherError> {
    java_properties::read(Cursor::new(bytes)).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse {subject} as Java properties: {error}"
        ))
    })
}

fn write_properties(properties: &HashMap<String, String>) -> Result<Vec<u8>, LauncherError> {
    let ordered: BTreeMap<_, _> = properties.iter().collect();
    let mut output = Vec::new();
    let mut writer = java_properties::PropertiesWriter::new(&mut output);
    writer
        .write_comment(
            "Restored by Orbit Launcher using values supported by this Minecraft version",
        )
        .map_err(property_write_error)?;
    for (key, value) in ordered {
        writer.write(key, value).map_err(property_write_error)?;
    }
    writer.finish().map_err(property_write_error)?;
    Ok(output)
}

fn server_world_relative(properties: &HashMap<String, String>) -> Result<PathBuf, LauncherError> {
    let level_name = properties
        .get("level-name")
        .map(String::as_str)
        .unwrap_or("world")
        .trim();
    if level_name.is_empty() {
        return Err(LauncherError::InvalidConfig(
            "server.properties level-name cannot be empty".to_string(),
        ));
    }
    let relative = Path::new(level_name);
    validate_relative_path(relative, "server.properties level-name")?;
    Ok(relative.to_path_buf())
}

fn validate_relative_path(path: &Path, subject: &str) -> Result<(), LauncherError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} '{}' is not a safe relative path",
            path.display()
        )));
    }
    Ok(())
}

fn portable_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn temporary_output_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    output.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}

fn replace_output(temporary: &Path, output: &Path) -> Result<(), std::io::Error> {
    #[cfg(windows)]
    if output.exists() {
        use std::os::windows::ffi::OsStrExt as _;
        use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

        let output = output
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let temporary = temporary
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: both paths are terminated UTF-16 strings that remain alive
        // for the call. The replacement is in the same directory and no
        // backup or reserved parameters are supplied.
        let replaced = unsafe {
            ReplaceFileW(
                output.as_ptr(),
                temporary.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if replaced == 0 {
            return Err(std::io::Error::last_os_error());
        }
        return Ok(());
    }
    std::fs::rename(temporary, output)
}

fn invalid_archive(error: zip::result::ZipError) -> LauncherError {
    LauncherError::InvalidRemoteData(format!("invalid Orbit bundle archive: {error}"))
}

fn write_archive_error(error: zip::result::ZipError) -> LauncherError {
    LauncherError::Transaction(format!("failed to write Orbit bundle archive: {error}"))
}

fn property_write_error(error: java_properties::PropertiesError) -> LauncherError {
    LauncherError::Transaction(format!("failed to write server.properties: {error}"))
}

struct StateRestoreTransaction {
    target_root: PathBuf,
    root: PathBuf,
    staging: PathBuf,
    files: Vec<PathBuf>,
}

impl StateRestoreTransaction {
    fn begin(target_root: &Path) -> Result<Self, LauncherError> {
        let root = target_root
            .join(STATE_DIRECTORY)
            .join("state-restore")
            .join(Uuid::new_v4().to_string());
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging)?;
        Ok(Self {
            target_root: target_root.to_path_buf(),
            root,
            staging,
            files: Vec::new(),
        })
    }

    fn stage_bytes(&self, relative: &Path, bytes: &[u8]) -> Result<(), LauncherError> {
        validate_relative_path(relative, "state restore destination")?;
        let destination = self.staging.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(destination, bytes)?;
        Ok(())
    }

    fn stage_reader<R, F>(
        &self,
        relative: &Path,
        reader: &mut R,
        expected: &StateArchiveFile,
        mut advanced: F,
    ) -> Result<(), LauncherError>
    where
        R: Read,
        F: FnMut(u64),
    {
        validate_relative_path(relative, "state restore destination")?;
        let destination = self.staging.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(destination)?;
        let mut hasher = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = vec![0_u8; 128 * 1024];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            let read = u64::try_from(read).expect("buffer length fits u64");
            bytes = bytes.saturating_add(read);
            advanced(read);
        }
        output.flush()?;
        let digest = hex::encode(hasher.finalize());
        if bytes != expected.bytes || !digest.eq_ignore_ascii_case(&expected.sha256) {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "Launcher state entry '{}' failed size or SHA-256 verification",
                expected.path
            )));
        }
        Ok(())
    }

    fn commit(mut self, replace_server_properties: bool) -> Result<(), LauncherError> {
        collect_staged_relative(&self.staging, &self.staging, &mut self.files)?;
        self.files.sort();
        let mut seen = HashSet::new();
        for relative in &self.files {
            if !seen.insert(relative.clone()) {
                return Err(LauncherError::Transaction(format!(
                    "state restore contains duplicate destination '{}'",
                    relative.display()
                )));
            }
            let target = self.target_root.join(relative);
            let may_replace =
                replace_server_properties && relative == Path::new("server.properties");
            if target.exists() && !may_replace {
                return Err(LauncherError::Transaction(format!(
                    "refusing to overwrite existing state path '{}'",
                    target.display()
                )));
            }
        }

        let backup = self.root.join("backup");
        let properties = self.target_root.join("server.properties");
        let properties_backup = backup.join("server.properties");
        if replace_server_properties && properties.exists() {
            std::fs::create_dir_all(&backup)?;
            std::fs::rename(&properties, &properties_backup)?;
        }
        let mut committed = Vec::new();
        for relative in &self.files {
            let staged = self.staging.join(relative);
            let target = self.target_root.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if let Err(error) = std::fs::rename(&staged, &target) {
                for committed in committed.iter().rev() {
                    let _ = std::fs::remove_file(self.target_root.join(committed));
                }
                if properties_backup.exists() {
                    let _ = std::fs::rename(&properties_backup, &properties);
                }
                return Err(error.into());
            }
            committed.push(relative.clone());
        }
        let _ = std::fs::remove_dir_all(&self.root);
        Ok(())
    }
}

impl Drop for StateRestoreTransaction {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn collect_staged_relative(
    root: &Path,
    path: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), LauncherError> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_staged_relative(root, &entry.path(), files)?;
        } else if metadata.is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|_| LauncherError::Transaction("staging path escaped".to_string()))?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eula::{EulaAcceptance, EulaAcceptanceMethod, MINECRAFT_EULA_URL};
    use crate::instance::{InstanceManifest, LoaderKind};
    use crate::lockfile::{
        ArtifactOwner, LOCK_SCHEMA, LauncherLock, LockedArguments, LockedArtifact,
        LockedArtifactSource, LockedEntrypoint, LockedLoader, LockedMinecraft,
    };

    fn write_server_instance(root: &Path, version: &str) {
        let id = Uuid::new_v4();
        ManifestFile::new(
            root,
            InstanceManifest::new(
                id,
                format!("server-{version}"),
                InstanceKind::Server,
                version,
                LoaderKind::Vanilla,
                None,
            )
            .unwrap(),
        )
        .save()
        .unwrap();
        LockFile::new(
            root,
            LauncherLock {
                schema: LOCK_SCHEMA,
                instance_id: id,
                kind: InstanceKind::Server,
                minecraft: LockedMinecraft {
                    version: version.to_string(),
                    version_type: "release".to_string(),
                    asset_index: None,
                    version_manifest_url:
                        "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
                            .to_string(),
                    version_manifest_sha256: "a".repeat(64),
                    version_json_url: "https://piston-meta.mojang.com/version.json".to_string(),
                    version_json_sha1: "b".repeat(40),
                },
                loader: LockedLoader::vanilla(),
                java: None,
                authlib_injector: None,
                entrypoint: LockedEntrypoint::Jar {
                    path: "server.jar".to_string(),
                },
                arguments: LockedArguments::default(),
                artifacts: vec![LockedArtifact {
                    logical_name: "Minecraft server".to_string(),
                    owner: ArtifactOwner::Minecraft,
                    source: LockedArtifactSource::Download {
                        url: "https://piston-data.mojang.com/server.jar".to_string(),
                        upstream_sha1: Some("c".repeat(40)),
                    },
                    sha256: "d".repeat(64),
                    size: 100,
                    path: "server.jar".to_string(),
                    native_extraction: None,
                }],
                generated_files: vec!["eula.txt".to_string(), "server.properties".to_string()],
                eula: Some(EulaAcceptance {
                    url: MINECRAFT_EULA_URL.to_string(),
                    digest_sha256: "e".repeat(64),
                    accepted_at_unix_seconds: 1,
                    method: EulaAcceptanceMethod::DigestCommand,
                }),
            },
        )
        .save()
        .unwrap();
    }

    #[test]
    fn server_world_name_is_relative_and_defaults_to_world() {
        assert_eq!(
            server_world_relative(&HashMap::new()).unwrap(),
            Path::new("world")
        );
        assert_eq!(
            server_world_relative(&HashMap::from([(
                "level-name".to_string(),
                "survival".to_string()
            )]))
            .unwrap(),
            Path::new("survival")
        );
        assert!(
            server_world_relative(&HashMap::from([(
                "level-name".to_string(),
                "../outside".to_string()
            )]))
            .is_err()
        );
    }

    #[test]
    fn client_state_uses_the_isolated_game_directory_saves_root() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("saves/local/region")).unwrap();
        std::fs::write(directory.path().join("options.txt"), b"lang:en_us\n").unwrap();
        std::fs::write(
            directory.path().join("saves/local/region/r.0.0.mca"),
            b"world-data",
        )
        .unwrap();
        let sources = collect_state_sources(directory.path(), InstanceKind::Client).unwrap();
        assert_eq!(
            sources
                .iter()
                .map(|source| source.archive_path.as_str())
                .collect::<Vec<_>>(),
            ["saves/local/region/r.0.0.mca", "state/options.txt"]
        );
    }

    #[test]
    fn server_property_merge_uses_target_key_intersection() {
        let mut target = HashMap::from([
            ("level-name".to_string(), "world".to_string()),
            ("new-setting".to_string(), "default".to_string()),
        ]);
        let source = HashMap::from([
            ("level-name".to_string(), "old-world".to_string()),
            ("removed-setting".to_string(), "legacy".to_string()),
        ]);
        let mut skipped = Vec::new();
        for (key, value) in source {
            if let Some(target) = target.get_mut(&key) {
                *target = value;
            } else {
                skipped.push(key);
            }
        }
        assert_eq!(target["level-name"], "old-world");
        assert_eq!(target["new-setting"], "default");
        assert_eq!(skipped, ["removed-setting"]);
    }

    #[test]
    fn server_state_export_and_install_follow_level_name_and_target_keys() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("target");
        std::fs::create_dir_all(source.join("alpha/region")).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        write_server_instance(&source, "1.20.1");
        write_server_instance(&target, "1.21.1");
        std::fs::write(
            source.join("server.properties"),
            "level-name=alpha\nshared=source\nremoved-setting=legacy\n",
        )
        .unwrap();
        std::fs::write(source.join("alpha/region/r.0.0.mca"), b"world-data").unwrap();
        std::fs::write(source.join("whitelist.json"), b"[]").unwrap();
        std::fs::write(
            target.join("server.properties"),
            "level-name=world\nshared=target\nnew-setting=default\n",
        )
        .unwrap();

        let archive = directory.path().join("state.orbitbundle");
        let exported = export_launcher_state(&source, &archive, |_| {}).unwrap();
        assert_eq!(exported.world_files, 1);
        let inspected = inspect_launcher_state(&archive).unwrap();
        assert_eq!(inspected.kind, InstanceKind::Server);
        assert_eq!(inspected.world_files, 1);

        let applied = restore_launcher_state(&target, &archive, |_| {}).unwrap();
        assert_eq!(applied.world_files, 1);
        assert_eq!(applied.skipped_properties, ["removed-setting"]);
        let properties = read_properties(
            &std::fs::read(target.join("server.properties")).unwrap(),
            "test target",
        )
        .unwrap();
        assert_eq!(properties["level-name"], "alpha");
        assert_eq!(properties["shared"], "source");
        assert_eq!(properties["new-setting"], "default");
        assert!(!properties.contains_key("removed-setting"));
        assert_eq!(
            std::fs::read(target.join("alpha/region/r.0.0.mca")).unwrap(),
            b"world-data"
        );
        assert_eq!(std::fs::read(target.join("whitelist.json")).unwrap(), b"[]");
    }

    #[test]
    fn launcher_state_composes_with_an_orbit_projection_without_rewriting_it() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        write_server_instance(&source, "1.21.1");
        std::fs::write(source.join("server.properties"), "level-name=world\n").unwrap();
        let output = directory.path().join("combined.orbitbundle");
        let orbit_manifest = b"orbit projection";
        let orbit_lock = b"lock projection";
        let files = [
            ("orbit/orbit.toml", orbit_manifest.as_slice()),
            ("orbit/orbit.lock", orbit_lock.as_slice()),
        ];
        let bundle = orbit_bundle_format::BundleManifest {
            format_version: orbit_bundle_format::BUNDLE_FORMAT_VERSION,
            id: "test".to_string(),
            name: "test".to_string(),
            version: "1".to_string(),
            summary: None,
            targets: vec![orbit_bundle_format::InstanceTarget::Server],
            runtime: orbit_bundle_format::RuntimeRequirement {
                minecraft: "1.21.1".to_string(),
                loader: "vanilla".to_string(),
                loader_version: None,
            },
            launcher: Some(orbit_bundle_format::LauncherSection {
                content: orbit_bundle_format::LauncherContent::RuntimeOnly,
            }),
            orbit: Some(orbit_bundle_format::OrbitSection {
                content: orbit_bundle_format::OrbitContent::Mods,
                manifest: files[0].0.to_string(),
                lock: files[1].0.to_string(),
                ownership: None,
                data_manifest: None,
            }),
            files: files
                .iter()
                .map(|(path, content)| orbit_bundle_format::BundleFile {
                    path: (*path).to_string(),
                    owner: orbit_bundle_format::BundleFileOwner::Orbit,
                    size: content.len() as u64,
                    sha256: hex::encode(Sha256::digest(content)),
                })
                .collect(),
        };
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&output).unwrap());
        let options = SimpleFileOptions::default();
        for (path, content) in files {
            archive.start_file(path, options).unwrap();
            archive.write_all(content).unwrap();
        }
        archive
            .start_file(orbit_bundle_format::BUNDLE_MANIFEST_PATH, options)
            .unwrap();
        archive
            .write_all(toml::to_string_pretty(&bundle).unwrap().as_bytes())
            .unwrap();
        archive.finish().unwrap();

        export_launcher_state_with_base(&source, Some(&output), &output, |_| {}).unwrap();

        let combined = orbit_bundle_format::BundleArchive::open(&output).unwrap();
        combined.verify().unwrap();
        assert!(combined.manifest.orbit.is_some());
        assert_eq!(
            combined.manifest.launcher.unwrap().content,
            orbit_bundle_format::LauncherContent::RuntimeAndState
        );
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let mut actual = Vec::new();
        archive
            .by_name("orbit/orbit.toml")
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, orbit_manifest);
    }
}
