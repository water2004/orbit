//! Manifest import and instance archive export.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;

use crate::error::OrbitError;
use crate::installer::PackageSelection;
use crate::manifest::PackageSpec;
use crate::progress::{ProgressEvent, ProgressReporter, emit as emit_progress};
use crate::workspace::{Lockfile, ManifestFile};

mod mrpack;
mod native;
pub use mrpack::import_mrpack;
pub use native::import_bundle;
pub use orbit_bundle_format::OrbitContent as ExportContent;

const PORTABLE_OWNERSHIP_PATH: &str = ".orbit/runtime-data/ownership.toml";
const PORTABLE_DATA_MANIFEST_PATH: &str = ".orbit/portable-data.toml";
const PORTABLE_DATA_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PortableDataManifest {
    schema: u32,
    ownership: Vec<crate::runtime_data::DataOwnershipEntry>,
    files: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMergeStrategy {
    PreferExisting,
    PreferImport,
    Interactive,
}

#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub added: Vec<String>,
    pub merged: Vec<String>,
    pub replaced: Vec<String>,
    pub kept: Vec<String>,
    pub extracted: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExportReport {
    pub path: PathBuf,
    pub packages: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct PortableInstance {
    directory: std::sync::Arc<tempfile::TempDir>,
}

impl PortableInstance {
    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub(crate) fn owner(&self) -> std::sync::Arc<tempfile::TempDir> {
        self.directory.clone()
    }
}

pub fn import_manifest<F>(
    instance_dir: &Path,
    source: &Path,
    strategy: ImportMergeStrategy,
    dry_run: bool,
    mut resolve_conflict: F,
) -> Result<ImportReport, OrbitError>
where
    F: FnMut(&str, &PackageSpec, &PackageSpec) -> Result<bool, OrbitError>,
{
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "import file not found: {}",
            source.display()
        )));
    }
    let incoming = crate::manifest::OrbitManifest::from_path(source)?;
    let mut current = ManifestFile::open(instance_dir)?;
    let mut report = ImportReport::default();
    for (package, requirement) in incoming.packages {
        match current.inner.packages.get(&package) {
            None => {
                report.added.push(package.clone());
                current.inner.packages.insert(package, requirement);
            }
            Some(existing) => {
                let mut remotes = existing.remotes.clone();
                remotes.extend(requirement.remotes.iter().cloned());
                remotes.sort();
                remotes.dedup();
                let remotes_changed = remotes != existing.remotes;
                if same_requirement_semantics(existing, &requirement) {
                    if remotes_changed {
                        current
                            .inner
                            .packages
                            .get_mut(&package)
                            .expect("package exists")
                            .remotes = remotes;
                        report.merged.push(package);
                    } else {
                        report.kept.push(package);
                    }
                    continue;
                }

                let use_import = match strategy {
                    ImportMergeStrategy::PreferExisting => false,
                    ImportMergeStrategy::PreferImport => true,
                    ImportMergeStrategy::Interactive => {
                        resolve_conflict(&package, existing, &requirement)?
                    }
                };
                if use_import {
                    let mut requirement = requirement;
                    requirement.remotes = remotes;
                    report.replaced.push(package.clone());
                    current.inner.packages.insert(package, requirement);
                } else if remotes_changed {
                    current
                        .inner
                        .packages
                        .get_mut(&package)
                        .expect("package exists")
                        .remotes = remotes;
                    report.merged.push(package);
                } else {
                    report.kept.push(package);
                }
            }
        }
    }
    report.added.sort();
    report.merged.sort();
    report.replaced.sort();
    report.kept.sort();
    if !dry_run
        && (!report.added.is_empty() || !report.merged.is_empty() || !report.replaced.is_empty())
    {
        current.save()?;
    }
    Ok(report)
}

fn same_requirement_semantics(left: &PackageSpec, right: &PackageSpec) -> bool {
    left.version == right.version
        && left.string == right.string
        && left.enabled == right.enabled
        && left.optional == right.optional
        && left.env == right.env
        && left.exclude == right.exclude
}

pub fn extract_portable_instance(source: &Path) -> Result<PortableInstance, OrbitError> {
    native::extract_portable_instance(source)
}

pub fn consume_portable_instance(source: &Path) -> Result<(), OrbitError> {
    if source.exists() {
        std::fs::remove_file(source)?;
    }
    Ok(())
}

pub fn export_instance(
    instance_dir: &Path,
    output: &Path,
    target: Option<String>,
    format: &str,
    content: ExportContent,
    dry_run: bool,
    progress: Option<ProgressReporter>,
) -> Result<ExportReport, OrbitError> {
    if !matches!(format, "orbit" | "mrpack") {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported export format '{format}'; expected orbit or mrpack"
        )));
    }
    if content == ExportContent::ModsAndData {
        crate::runtime_data::merge_observation_sessions(instance_dir)?;
    }
    let manifest = ManifestFile::open(instance_dir)?;
    let lockfile = Lockfile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest.inner)?;
    let loader_package = platform.loader_package;
    let (selected, _) = crate::installer::selected_packages(
        &manifest.inner,
        &lockfile.inner,
        &PackageSelection {
            target: target.clone(),
            ..PackageSelection::default()
        },
        loader_package.as_ref(),
        platform.minecraft_version.java_version,
    )?;
    let mut sources = Vec::new();
    for package in &selected {
        let entry = lockfile.inner.find(package).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "orbit.lock is missing export package '{package}'"
            ))
        })?;
        if entry.filename.is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "no filename recorded for export package '{package}'"
            )));
        }
        let source = instance_dir.join("mods").join(&entry.filename);
        if !source.is_file() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "JAR for export package '{package}' is missing: {}",
                source.display()
            )));
        }
        let bytes = std::fs::metadata(&source)?.len();
        sources.push((entry, source, bytes));
    }
    let (portable_manifest, portable_lock) =
        portable_state(&manifest.inner, &lockfile.inner, &selected);
    let portable_manifest_toml = portable_manifest.to_toml_string()?;
    let portable_lock_toml = portable_lock.to_toml_string()?;
    let (ownership_document, state_sources, portable_data_document) =
        if content == ExportContent::ModsAndData {
            let selected_owners = selected
                .iter()
                .filter_map(|package| lockfile.inner.find(package))
                .map(|entry| entry.mod_id.clone())
                .collect::<BTreeSet<_>>();
            let mut ownership_entries =
                crate::runtime_data::ownership_entries_for(instance_dir, &selected_owners)?;
            crate::runtime_data::retain_instance_ownership(&mut ownership_entries);
            let ownership_document =
                crate::runtime_data::ownership_document(ownership_entries.clone())?;
            let state_sources =
                portable_state_sources(instance_dir, &selected_owners, &ownership_entries)?;
            let mut portable_files = sources
                .iter()
                .map(|(entry, _, _)| format!("mods/{}", entry.filename))
                .collect::<BTreeSet<_>>();
            for source in &state_sources {
                portable_files.insert(archive_path(&source.relative)?);
            }
            let portable_data_document = toml::to_string_pretty(&PortableDataManifest {
                schema: PORTABLE_DATA_SCHEMA,
                ownership: ownership_entries,
                files: portable_files,
            })
            .map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "failed to serialize portable Orbit data manifest: {error}"
                ))
            })?;
            (
                Some(ownership_document),
                state_sources,
                Some(portable_data_document),
            )
        } else {
            (None, Vec::new(), None)
        };
    let state_bytes: u64 = state_sources.iter().map(|source| source.bytes).sum();
    let package_bytes: u64 = sources.iter().map(|(_, _, bytes)| bytes).sum();
    let archived_bytes: u64 = if format == "mrpack" {
        sources
            .iter()
            .filter(|(entry, _, _)| mrpack::is_embedded(entry))
            .map(|(_, _, bytes)| bytes)
            .sum()
    } else {
        package_bytes
    };
    let metadata_bytes = ownership_document.as_ref().map_or(0, String::len) as u64
        + portable_data_document.as_ref().map_or(0, String::len) as u64;
    let verification_bytes = if dry_run || format == "mrpack" {
        package_bytes
    } else {
        0
    };
    let total_work = if dry_run {
        verification_bytes + metadata_bytes
    } else {
        verification_bytes + archived_bytes + state_bytes + metadata_bytes
    };
    let mut tracker = ExportTracker::new(progress, sources.len(), total_work);
    tracker.started();
    if dry_run || format == "mrpack" {
        for (index, (entry, source, source_bytes)) in sources.iter().enumerate() {
            verify_export_source(
                entry,
                source,
                *source_bytes,
                format == "mrpack",
                &mut tracker,
            )?;
            tracker.complete_package(index + 1);
        }
    }
    let report = ExportReport {
        path: output.to_path_buf(),
        packages: sources.len(),
        bytes: package_bytes
            + state_bytes
            + ownership_document.as_ref().map_or(0, String::len) as u64
            + portable_data_document.as_ref().map_or(0, String::len) as u64,
    };
    if dry_run {
        tracker.advance(metadata_bytes);
        tracker.finished();
        return Ok(report);
    }
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temporary_output_path(output);
    if temporary.exists() {
        std::fs::remove_file(&temporary)?;
    }
    let pending = PendingOutput::new(temporary.clone());
    let file = std::fs::File::create(&temporary)?;
    let mut archive = zip::ZipWriter::new(file);
    let metadata_options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let artifact_options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    if format == "mrpack" {
        mrpack::write_contents(
            &mut archive,
            metadata_options,
            artifact_options,
            mrpack::MrpackContents {
                manifest: &portable_manifest,
                packages: &sources,
                state: &state_sources,
                target: export_target(&target, platform.physical_environment)?,
            },
            &mut tracker,
        )?;
    } else {
        native::write_contents(
            &mut archive,
            metadata_options,
            artifact_options,
            native::NativeContents {
                manifest: &portable_manifest,
                manifest_toml: &portable_manifest_toml,
                lock_toml: &portable_lock_toml,
                packages: &sources,
                state: &state_sources,
                ownership: ownership_document.as_deref(),
                data_manifest: portable_data_document.as_deref(),
                targets: export_targets(&target, platform.physical_environment)?,
                content,
            },
            &mut tracker,
        )?;
    }
    archive.finish()?.sync_all()?;
    replace_output(&temporary, output)?;
    pending.commit();
    tracker.finished();
    Ok(report)
}

fn export_targets(
    target: &Option<String>,
    physical: crate::metadata::Environment,
) -> Result<Vec<orbit_bundle_format::InstanceTarget>, OrbitError> {
    use orbit_bundle_format::InstanceTarget::{Client, Server};
    match target.as_deref() {
        Some("client") => Ok(vec![Client]),
        Some("server") => Ok(vec![Server]),
        Some("both") => Ok(vec![Client, Server]),
        Some(other) => Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported export target '{other}'; expected client, server, or both"
        ))),
        None => match physical {
            crate::metadata::Environment::Client => Ok(vec![Client]),
            crate::metadata::Environment::Server => Ok(vec![Server]),
            crate::metadata::Environment::Both => Ok(vec![Client, Server]),
        },
    }
}

fn export_target(
    target: &Option<String>,
    physical: crate::metadata::Environment,
) -> Result<Option<orbit_bundle_format::InstanceTarget>, OrbitError> {
    let targets = export_targets(target, physical)?;
    Ok((targets.len() == 1).then(|| targets[0]))
}

fn add_file(
    archive: &mut zip::ZipWriter<std::fs::File>,
    archive_path: &str,
    source: &Path,
    options: SimpleFileOptions,
    mut progress: Option<&mut ExportTracker>,
) -> Result<(), OrbitError> {
    archive.start_file(archive_path, options)?;
    let mut input = std::fs::File::open(source)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        archive.write_all(&buffer[..read])?;
        if let Some(progress) = progress.as_mut() {
            progress.advance(read as u64);
        }
    }
    Ok(())
}

fn add_content(
    archive: &mut zip::ZipWriter<std::fs::File>,
    archive_path: &str,
    content: &[u8],
    options: SimpleFileOptions,
) -> Result<(), OrbitError> {
    archive.start_file(archive_path, options)?;
    archive.write_all(content)?;
    Ok(())
}

fn portable_state(
    manifest: &crate::manifest::OrbitManifest,
    lockfile: &crate::lockfile::OrbitLockfile,
    selected: &[String],
) -> (
    crate::manifest::OrbitManifest,
    crate::lockfile::OrbitLockfile,
) {
    let selected: std::collections::BTreeSet<_> = selected.iter().map(String::as_str).collect();
    let mut portable_manifest = manifest.clone();
    portable_manifest
        .packages
        .retain(|package, _| selected.contains(package.as_str()));

    let mut portable_lock = lockfile.clone();
    portable_lock
        .packages
        .retain(|entry| selected.contains(entry.mod_id.as_str()));
    for entry in &mut portable_lock.packages {
        let path = format!("mods/{}", entry.filename);
        let remote = crate::manifest::PackageRemote::File { path: path.clone() };
        if !entry.remotes.contains(&remote) {
            entry.remotes.push(remote.clone());
            entry.remotes.sort();
        }
        let source = crate::lockfile::ArtifactSource::File { path };
        if !entry.artifact_sources.contains(&source) {
            entry.artifact_sources.push(source);
        }
        if let Some(package) = portable_manifest.packages.get_mut(&entry.mod_id)
            && !package.remotes.contains(&remote)
        {
            package.remotes.push(remote);
            package.remotes.sort();
        }
    }
    (portable_manifest, portable_lock)
}

fn verify_export_source(
    entry: &crate::lockfile::PackageEntry,
    source: &Path,
    expected_size: u64,
    verify_mrpack_hashes: bool,
    progress: &mut ExportTracker,
) -> Result<(), OrbitError> {
    use sha2::{Digest as _, Sha256, Sha512};

    let mut file = std::fs::File::open(source)?;
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size.checked_add(read as u64).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!("source file size overflowed during export"))
        })?;
        sha1.update(&buffer[..read]);
        sha256.update(&buffer[..read]);
        sha512.update(&buffer[..read]);
        progress.advance(read as u64);
    }
    if size != expected_size {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "source '{}' changed size during export: expected {expected_size}, read {size}",
            source.display()
        )));
    }
    let actual_sha256 = hex::encode(sha256.finalize());
    if !entry.sha256.is_empty() && actual_sha256 != entry.sha256 {
        return Err(OrbitError::ChecksumMismatch {
            name: entry.filename.clone(),
            expected: entry.sha256.clone(),
            actual: actual_sha256,
        });
    }
    if verify_mrpack_hashes && !mrpack::is_embedded(entry) {
        let actual_sha1 = hex::encode(sha1.finalize());
        let actual_sha512 = hex::encode(sha512.finalize());
        if !actual_sha1.eq_ignore_ascii_case(&entry.sha1)
            || !actual_sha512.eq_ignore_ascii_case(&entry.sha512)
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack hashes for '{}' do not match its JAR",
                entry.filename
            )));
        }
    }
    Ok(())
}

struct ExportTracker {
    progress: Option<ProgressReporter>,
    packages: usize,
    completed_packages: usize,
    completed_bytes: u64,
    total_bytes: u64,
    last_emitted_bytes: u64,
}

const EXPORT_PROGRESS_INTERVAL_BYTES: u64 = 1024 * 1024;

impl ExportTracker {
    fn new(progress: Option<ProgressReporter>, packages: usize, total_bytes: u64) -> Self {
        Self {
            progress,
            packages,
            completed_packages: 0,
            completed_bytes: 0,
            total_bytes,
            last_emitted_bytes: 0,
        }
    }

    fn started(&self) {
        emit_progress(
            self.progress.as_ref(),
            ProgressEvent::ExportStarted {
                packages: self.packages,
                total_bytes: self.total_bytes,
            },
        );
    }

    fn advance(&mut self, bytes: u64) {
        self.completed_bytes = self
            .completed_bytes
            .saturating_add(bytes)
            .min(self.total_bytes);
        if self.completed_bytes == self.total_bytes
            || self.completed_bytes.saturating_sub(self.last_emitted_bytes)
                >= EXPORT_PROGRESS_INTERVAL_BYTES
        {
            self.emit_advanced();
        }
    }

    fn complete_package(&mut self, completed: usize) {
        self.completed_packages = completed;
        self.emit_advanced();
    }

    fn emit_advanced(&mut self) {
        self.last_emitted_bytes = self.completed_bytes;
        emit_progress(
            self.progress.as_ref(),
            ProgressEvent::ExportAdvanced {
                completed: self.completed_bytes,
                total: self.total_bytes,
                completed_packages: self.completed_packages,
                packages: self.packages,
            },
        );
    }

    fn finished(&self) {
        emit_progress(
            self.progress.as_ref(),
            ProgressEvent::ExportFinished {
                packages: self.packages,
                total_bytes: self.total_bytes,
            },
        );
    }
}

struct PendingOutput {
    path: PathBuf,
    committed: bool,
}

impl PendingOutput {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for PendingOutput {
    fn drop(&mut self) {
        if !self.committed && self.path.is_file() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn temporary_output_path(output: &Path) -> PathBuf {
    let filename = output
        .file_name()
        .map(|filename| filename.to_string_lossy())
        .unwrap_or_default();
    output.with_file_name(format!(".{filename}.tmp"))
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

#[derive(Debug)]
pub(crate) struct PortableFile {
    pub source: PathBuf,
    pub relative: PathBuf,
    pub bytes: u64,
}

pub(crate) fn portable_config_sources(
    instance_dir: &Path,
) -> Result<Vec<PortableFile>, OrbitError> {
    const ROOTS: [&str; 3] = ["config", "defaultconfigs", "serverconfig"];
    let mut sources = Vec::new();
    for root in ROOTS {
        let path = instance_dir.join(root);
        if path.exists() {
            collect_portable_files(instance_dir, &path, &mut sources)?;
        }
    }
    sources.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(sources)
}

pub(crate) fn portable_state_sources(
    instance_dir: &Path,
    owners: &BTreeSet<String>,
    ownership: &[crate::runtime_data::DataOwnershipEntry],
) -> Result<Vec<PortableFile>, OrbitError> {
    let mut by_path = portable_config_sources(instance_dir)?
        .into_iter()
        .map(|source| (source.relative.clone(), source))
        .collect::<BTreeMap<_, _>>();
    if owners.is_empty() || ownership.is_empty() {
        return Ok(by_path.into_values().collect());
    }

    let all_entries = crate::runtime_data::load_ledger(instance_dir)?.entries;
    let mut roots = ownership
        .iter()
        .filter_map(|entry| match &entry.path {
            crate::runtime_data::OwnedDataPath::Instance { relative } => {
                Some((PathBuf::from(relative), entry.kind))
            }
            crate::runtime_data::OwnedDataPath::External { .. } => None,
        })
        .collect::<Vec<_>>();
    roots.sort_by(|left, right| {
        left.0
            .components()
            .count()
            .cmp(&right.0.components().count())
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut selected_roots: Vec<(PathBuf, crate::runtime_data::OwnedDataKind)> = Vec::new();
    for (path, kind) in roots {
        if selected_roots.iter().any(|(root, root_kind)| {
            *root_kind == crate::runtime_data::OwnedDataKind::Tree && path.starts_with(root)
        }) {
            continue;
        }
        selected_roots.push((path, kind));
    }
    for (relative, kind) in selected_roots {
        let source = instance_dir.join(&relative);
        if !source.exists() {
            continue;
        }
        collect_owned_files(
            instance_dir,
            &source,
            kind,
            owners,
            &all_entries,
            &mut by_path,
        )?;
    }
    Ok(by_path.into_values().collect())
}

fn collect_owned_files(
    instance_dir: &Path,
    path: &Path,
    kind: crate::runtime_data::OwnedDataKind,
    owners: &BTreeSet<String>,
    ownership: &[crate::runtime_data::DataOwnershipEntry],
    sources: &mut BTreeMap<PathBuf, PortableFile>,
) -> Result<(), OrbitError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "portable package data contains a symbolic link: {}",
            path.display()
        )));
    }
    if metadata.is_dir() {
        if kind == crate::runtime_data::OwnedDataKind::File {
            return Ok(());
        }
        for entry in std::fs::read_dir(path)? {
            collect_owned_files(
                instance_dir,
                &entry?.path(),
                crate::runtime_data::OwnedDataKind::Tree,
                owners,
                ownership,
                sources,
            )?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Ok(());
    }
    let relative = path.strip_prefix(instance_dir).map_err(|_| {
        OrbitError::Other(anyhow::anyhow!(
            "portable package data escaped its instance directory"
        ))
    })?;
    if crate::runtime_data::effective_owner_for_relative(ownership, relative)
        .is_none_or(|owner| !owners.contains(&owner))
    {
        return Ok(());
    }
    sources
        .entry(relative.to_path_buf())
        .or_insert(PortableFile {
            source: path.to_path_buf(),
            relative: relative.to_path_buf(),
            bytes: metadata.len(),
        });
    Ok(())
}

fn collect_portable_files(
    instance_dir: &Path,
    path: &Path,
    sources: &mut Vec<PortableFile>,
) -> Result<(), OrbitError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "portable instance content contains a symbolic link: {}",
            path.display()
        )));
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect_portable_files(instance_dir, &entry?.path(), sources)?;
        }
    } else if metadata.is_file() {
        let relative = path.strip_prefix(instance_dir).map_err(|_| {
            OrbitError::Other(anyhow::anyhow!(
                "portable instance content escaped its instance directory"
            ))
        })?;
        sources.push(PortableFile {
            source: path.to_path_buf(),
            relative: relative.to_path_buf(),
            bytes: metadata.len(),
        });
    }
    Ok(())
}

pub(crate) fn archive_path(path: &Path) -> Result<String, OrbitError> {
    let mut components = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "portable path must be relative and normalized: {}",
                path.display()
            )));
        };
        let component = component.to_str().ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "portable path is not valid UTF-8: {}",
                path.display()
            ))
        })?;
        components.push(component);
    }
    let value = components.join("/");
    orbit_bundle_format::validate_relative_path(&value).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "invalid portable path '{}': {error}",
            path.display()
        ))
    })?;
    Ok(value)
}

pub(crate) fn validate_instance_destination(
    instance: &Path,
    relative: &Path,
) -> Result<(), OrbitError> {
    let relative = archive_path(relative)?;
    mrpack::validate_import_destination(instance, &relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{ArtifactSource, LockMeta, OrbitLockfile, PackageEntry};
    use crate::manifest::{OrbitManifest, PackageRemote, ProjectMeta, ResolverConfig};
    use serde_json::json;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("orbit-archive-test-{name}-{}", std::process::id()))
    }

    fn manifest(dependency: &str) -> OrbitManifest {
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
            packages: indexmap::IndexMap::from([(
                dependency.to_string(),
                PackageSpec::new(
                    "*",
                    vec![PackageRemote::File {
                        path: format!("sources/{dependency}.jar"),
                    }],
                ),
            )]),
            groups: indexmap::IndexMap::new(),
        }
    }

    fn detected_manifest(directory: &Path, dependency: &str) -> OrbitManifest {
        let mut manifest = manifest(dependency);
        manifest.platform =
            crate::platform_detection::discover_platform_for_init(directory, "1", "fabric", "1")
                .unwrap()
                .snapshot(directory)
                .unwrap();
        manifest
    }

    #[test]
    fn manifest_import_respects_prefer_existing() {
        let directory = test_dir("manifest");
        std::fs::create_dir_all(&directory).unwrap();
        ManifestFile::new(&directory, manifest("existing"))
            .save()
            .unwrap();
        let source = directory.join("incoming.toml");
        let mut incoming = manifest("added");
        incoming.packages.insert(
            "existing".to_string(),
            PackageSpec::new(
                "2",
                vec![PackageRemote::File {
                    path: "sources/existing-v2.jar".to_string(),
                }],
            ),
        );
        std::fs::write(&source, incoming.to_toml_string().unwrap()).unwrap();

        let report = import_manifest(
            &directory,
            &source,
            ImportMergeStrategy::PreferExisting,
            false,
            |_, _, _| Ok(false),
        )
        .unwrap();

        assert_eq!(report.added, vec!["added"]);
        assert_eq!(report.merged, vec!["existing"]);
        let imported = ManifestFile::open(&directory).unwrap();
        assert_eq!(
            imported.inner.packages["existing"].version_constraint(),
            "*"
        );
        assert_eq!(imported.inner.packages["existing"].remotes.len(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_generic_zip_is_not_an_orbit_bundle() {
        let directory = test_dir("legacy-zip-import");
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("pack.zip");
        let file = std::fs::File::create(&source).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("mods/example.jar", options).unwrap();
        zip.write_all(b"example").unwrap();
        zip.start_file("mods/disabled.jar.disabled", options)
            .unwrap();
        zip.write_all(b"disabled").unwrap();
        zip.start_file("../mods/escape.jar", options).unwrap();
        zip.write_all(b"escape").unwrap();
        zip.finish().unwrap();

        assert!(extract_portable_instance(&source).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn orbit_bundle_contains_declared_manifest_lock_mods_and_data() {
        let directory = test_dir("bundle-export");
        crate::platform_detection::test_support::write_platform(&directory, "1", "fabric", "1");
        std::fs::create_dir_all(directory.join("mods")).unwrap();
        ManifestFile::new(&directory, detected_manifest(&directory, "example"))
            .save()
            .unwrap();
        let jar = directory.join("mods/example.jar");
        std::fs::write(&jar, b"example").unwrap();
        let owner = crate::jar::compute_sha256(&jar).unwrap();
        std::fs::create_dir_all(directory.join("config")).unwrap();
        std::fs::write(directory.join("config/example.toml"), b"enabled = true\n").unwrap();
        Lockfile::new(
            &directory,
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
                    sha256: owner.clone(),
                    sha512: crate::jar::compute_sha512(&jar).unwrap(),
                    filename: "example.jar".to_string(),
                    remotes: vec![PackageRemote::File {
                        path: "mods/example.jar".to_string(),
                    }],
                    artifact_sources: vec![ArtifactSource::File {
                        path: "mods/example.jar".to_string(),
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
        std::fs::create_dir_all(directory.join("bluemap/maps/world")).unwrap();
        std::fs::create_dir_all(directory.join("bluemap/foreign")).unwrap();
        std::fs::write(
            directory.join("bluemap/maps/world/user-marker.json"),
            b"user-authored content",
        )
        .unwrap();
        std::fs::write(
            directory.join("bluemap/foreign/state.bin"),
            b"other package",
        )
        .unwrap();
        crate::runtime_data::save_ledger(
            &directory,
            &crate::runtime_data::DataOwnershipLedger::from_entries(vec![
                crate::runtime_data::DataOwnershipEntry {
                    path: crate::runtime_data::OwnedDataPath::Instance {
                        relative: "bluemap".to_string(),
                    },
                    kind: crate::runtime_data::OwnedDataKind::Tree,
                    owner: Some("example".to_string()),
                },
                crate::runtime_data::DataOwnershipEntry {
                    path: crate::runtime_data::OwnedDataPath::Instance {
                        relative: "bluemap/foreign".to_string(),
                    },
                    kind: crate::runtime_data::OwnedDataKind::Tree,
                    owner: Some("other-package".to_string()),
                },
            ]),
        )
        .unwrap();
        let output = directory.join("pack.orbitbundle");

        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = events.clone();
        let progress: ProgressReporter = std::sync::Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });
        let report = export_instance(
            &directory,
            &output,
            None,
            "orbit",
            ExportContent::ModsAndData,
            false,
            Some(progress),
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();

        assert_eq!(report.packages, 1);
        assert!(archive.by_name("bundle.toml").is_ok());
        assert!(archive.by_name("orbit/orbit.toml").is_ok());
        assert!(archive.by_name("orbit/orbit.lock").is_ok());
        assert!(archive.by_name("orbit/config/example.toml").is_ok());
        assert!(
            archive
                .by_name("orbit/bluemap/maps/world/user-marker.json")
                .is_ok()
        );
        assert!(archive.by_name("orbit/bluemap/foreign/state.bin").is_err());
        let jar = archive.by_name("orbit/mods/example.jar").unwrap();
        assert_eq!(jar.compression(), zip::CompressionMethod::Stored);
        drop(jar);
        let events = events.lock().unwrap();
        assert!(matches!(
            events.first(),
            Some(ProgressEvent::ExportStarted { packages: 1, .. })
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            ProgressEvent::ExportAdvanced {
                completed,
                total,
                ..
            } if completed == total
        )));
        assert!(matches!(
            events.last(),
            Some(ProgressEvent::ExportFinished { packages: 1, .. })
        ));
        drop(archive);
        let portable = extract_portable_instance(&output).unwrap();
        let portable_manifest = ManifestFile::open(portable.path()).unwrap();
        assert!(portable_manifest.inner.packages["example"]
            .remotes
            .iter()
            .any(|remote| matches!(remote, PackageRemote::File { path } if path == "mods/example.jar")));
        let portable_lock = Lockfile::open(portable.path()).unwrap();
        assert!(portable_lock.inner.packages[0].artifact_sources.iter().any(
            |source| matches!(source, ArtifactSource::File { path } if path == "mods/example.jar")
        ));
        assert_eq!(
            std::fs::read_to_string(portable.path().join("config/example.toml")).unwrap(),
            "enabled = true\n"
        );
        assert_eq!(
            std::fs::read(portable.path().join("bluemap/maps/world/user-marker.json")).unwrap(),
            b"user-authored content"
        );
        assert!(!portable.path().join("bluemap/foreign/state.bin").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mrpack_references_online_files_and_embeds_local_overrides() {
        let directory = test_dir("mrpack-export");
        crate::platform_detection::test_support::write_platform(&directory, "1", "fabric", "1");
        std::fs::create_dir_all(directory.join("mods")).unwrap();
        let mut project = detected_manifest(&directory, "online");
        project.packages.insert(
            "local".to_string(),
            PackageSpec::new(
                "*",
                vec![PackageRemote::File {
                    path: "mods/local.jar".to_string(),
                }],
            ),
        );
        project.packages.insert(
            "online".to_string(),
            PackageSpec {
                version: "*".to_string(),
                string: "all".to_string(),
                enabled: true,
                optional: true,
                env: Some(crate::metadata::Environment::Client),
                exclude: Vec::new(),
                remotes: vec![PackageRemote::Modrinth {
                    project_id: "online-project".to_string(),
                }],
            },
        );
        ManifestFile::new(&directory, project).save().unwrap();
        let online_jar = directory.join("mods/online.jar");
        let local_jar = directory.join("mods/local.jar");
        std::fs::write(&online_jar, b"online").unwrap();
        std::fs::write(&local_jar, b"local").unwrap();
        let online_bytes = std::fs::read(&online_jar).unwrap();
        let local_bytes = std::fs::read(&local_jar).unwrap();
        Lockfile::new(
            &directory,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![
                    PackageEntry {
                        mod_id: "online".to_string(),
                        version: "1".to_string(),
                        sha1: crate::jar::sha1_digest(&online_bytes),
                        sha256: crate::jar::sha256_digest(&online_bytes),
                        sha512: crate::jar::sha512_digest(&online_bytes),
                        filename: "online.jar".to_string(),
                        remotes: vec![PackageRemote::Modrinth {
                            project_id: "online-project".to_string(),
                        }],
                        artifact_sources: vec![ArtifactSource::Modrinth {
                            project_id: "online-project".to_string(),
                            version_id: "online-version".to_string(),
                            download_url: "https://cdn.modrinth.com/online.jar".to_string(),
                        }],
                        dependencies: Vec::new(),
                        environment: crate::metadata::Environment::Both,
                        provides: Vec::new(),
                        language_loader: None,
                        embedded_artifacts: Vec::new(),
                        bundled: Vec::new(),
                    },
                    PackageEntry {
                        mod_id: "local".to_string(),
                        version: "1".to_string(),
                        sha1: crate::jar::sha1_digest(&local_bytes),
                        sha256: crate::jar::sha256_digest(&local_bytes),
                        sha512: crate::jar::sha512_digest(&local_bytes),
                        filename: "local.jar".to_string(),
                        remotes: vec![PackageRemote::File {
                            path: "mods/local.jar".to_string(),
                        }],
                        artifact_sources: vec![ArtifactSource::File {
                            path: "mods/local.jar".to_string(),
                        }],
                        dependencies: Vec::new(),
                        environment: crate::metadata::Environment::Both,
                        provides: Vec::new(),
                        language_loader: None,
                        embedded_artifacts: Vec::new(),
                        bundled: Vec::new(),
                    },
                ],
            },
        )
        .save()
        .unwrap();
        let output = directory.join("pack.mrpack");

        export_instance(
            &directory,
            &output,
            None,
            "mrpack",
            ExportContent::ModsAndData,
            false,
            None,
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let index: serde_json::Value = {
            let mut entry = archive.by_name("modrinth.index.json").unwrap();
            serde_json::from_reader(&mut entry).unwrap()
        };

        assert_eq!(index["files"].as_array().unwrap().len(), 1);
        assert_eq!(index["files"][0]["path"], "mods/online.jar");
        assert_eq!(index["files"][0]["env"]["client"], "optional");
        assert_eq!(index["files"][0]["env"]["server"], "unsupported");
        assert!(archive.by_name("mods/online.jar").is_err());
        assert!(archive.by_name("overrides/mods/local.jar").is_ok());
        assert!(archive.by_name("overrides/orbit.toml").is_err());
        assert!(archive.by_name("overrides/orbit.lock").is_err());
        assert!(archive.by_name("orbit.toml").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn mrpack_import_applies_common_overrides() {
        let directory = test_dir("mrpack-import-override");
        crate::platform_detection::test_support::write_platform(&directory, "1", "fabric", "1");
        ManifestFile::new(&directory, detected_manifest(&directory, "example"))
            .save()
            .unwrap();
        let source = directory.join("input.mrpack");
        let file = std::fs::File::create(&source).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        archive.start_file("modrinth.index.json", options).unwrap();
        archive
            .write_all(
                serde_json::to_string(&json!({
                    "formatVersion": 1,
                    "game": "minecraft",
                    "versionId": "test",
                    "name": "test",
                    "files": [],
                    "dependencies": { "minecraft": "1", "fabric-loader": "1" }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        archive
            .start_file("overrides/mods/bundled.jar", options)
            .unwrap();
        archive.write_all(b"bundled bytes").unwrap();
        archive.finish().unwrap();

        let report = import_mrpack(
            &directory,
            &source,
            false,
            false,
            &BTreeSet::new(),
            false,
            None,
        )
        .await
        .unwrap();

        assert_eq!(report.extracted, vec!["mods/bundled.jar"]);
        assert_eq!(
            std::fs::read(directory.join("mods/bundled.jar")).unwrap(),
            b"bundled bytes"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn mrpack_import_rejects_unsafe_index_paths() {
        let directory = test_dir("mrpack-import-unsafe");
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("unsafe.mrpack");
        let file = std::fs::File::create(&source).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        archive.start_file("modrinth.index.json", options).unwrap();
        archive
            .write_all(
                serde_json::to_string(&json!({
                    "formatVersion": 1,
                    "game": "minecraft",
                    "versionId": "test",
                    "name": "test",
                    "files": [{
                        "path": "../mods/escape.jar",
                        "hashes": { "sha1": "unused", "sha512": "unused" },
                        "downloads": ["https://cdn.modrinth.com/escape.jar"],
                        "fileSize": 1
                    }],
                    "dependencies": { "minecraft": "1.21.1" }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        archive.finish().unwrap();

        import_mrpack(
            &directory,
            &source,
            false,
            false,
            &BTreeSet::new(),
            false,
            None,
        )
        .await
        .unwrap_err();

        assert!(!directory.join("escape.jar").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn mrpack_import_rejects_an_optional_path_not_available_for_the_target() {
        let directory = test_dir("mrpack-import-optional-selection");
        crate::platform_detection::test_support::write_platform(&directory, "1", "fabric", "1");
        ManifestFile::new(&directory, detected_manifest(&directory, "example"))
            .save()
            .unwrap();
        let source = directory.join("optional.mrpack");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&source).unwrap());
        archive
            .start_file("modrinth.index.json", SimpleFileOptions::default())
            .unwrap();
        archive
            .write_all(
                serde_json::to_string(&json!({
                    "formatVersion": 1,
                    "game": "minecraft",
                    "versionId": "test",
                    "name": "test",
                    "files": [{
                        "path": "mods/optional.jar",
                        "hashes": { "sha1": "0".repeat(40), "sha512": "0".repeat(128) },
                        "env": { "client": "optional", "server": "unsupported" },
                        "downloads": ["https://cdn.modrinth.com/data/test/optional.jar"],
                        "fileSize": 1
                    }],
                    "dependencies": { "minecraft": "1", "fabric-loader": "1" }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        archive.finish().unwrap();

        let error = import_mrpack(
            &directory,
            &source,
            false,
            false,
            &BTreeSet::from(["mods/not-declared.jar".to_string()]),
            false,
            None,
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not optional for the selected target")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
