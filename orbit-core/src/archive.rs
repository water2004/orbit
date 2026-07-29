//! Manifest import and instance archive export.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

use crate::error::OrbitError;
use crate::installer::PackageSelection;
use crate::manifest::PackageSpec;
use crate::progress::{ProgressEvent, ProgressReporter, emit as emit_progress};
use crate::workspace::{Lockfile, ManifestFile};

mod mrpack;
pub use mrpack::import_mrpack;

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
        && left.optional == right.optional
        && left.env == right.env
        && left.exclude == right.exclude
}

pub fn import_archive(
    instance_dir: &Path,
    source: &Path,
    overwrite: bool,
    dry_run: bool,
) -> Result<ImportReport, OrbitError> {
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "import archive not found: {}",
            source.display()
        )));
    }
    let file = std::fs::File::open(source)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mods_dir = instance_dir.join("mods");
    let mut report = ImportReport::default();
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let is_jar = enclosed
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("jar"));
        let under_mods = enclosed.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("mods")
        });
        if !entry.is_file() || !is_jar || !under_mods {
            continue;
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > 4 * 1024 * 1024 * 1024 {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "archive contains more than 4 GiB of mod files"
            )));
        }
        let filename = enclosed
            .file_name()
            .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("invalid JAR path in archive")))?
            .to_string_lossy()
            .into_owned();
        let destination = mods_dir.join(&filename);
        if destination.exists() && !overwrite {
            report.kept.push(filename);
            continue;
        }
        report.extracted.push(filename.clone());
        if dry_run {
            continue;
        }
        std::fs::create_dir_all(&mods_dir)?;
        let temporary = mods_dir.join(format!(".{filename}.importing"));
        let mut output = std::fs::File::create(&temporary)?;
        std::io::copy(&mut entry, &mut output)?;
        output.sync_all()?;
        if destination.exists() {
            std::fs::remove_file(&destination)?;
        }
        std::fs::rename(temporary, destination)?;
    }
    report.extracted.sort();
    report.kept.sort();
    Ok(report)
}

pub fn extract_portable_instance(source: &Path) -> Result<PortableInstance, OrbitError> {
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "portable Orbit pack not found: {}",
            source.display()
        )));
    }
    let directory = std::sync::Arc::new(tempfile::tempdir()?);
    let mut archive = zip::ZipArchive::new(std::fs::File::open(source)?)?;
    let mut extracted = std::collections::BTreeSet::new();
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if !entry.is_file() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "portable Orbit pack contains a symbolic link"
            )));
        }
        let Some(enclosed) = entry.enclosed_name() else {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "portable Orbit pack contains an unsafe path"
            )));
        };
        let Some(relative) = portable_entry_path(&enclosed) else {
            continue;
        };
        if !extracted.insert(relative.clone()) {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "portable Orbit pack contains duplicate path '{}'",
                relative.display()
            )));
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > 8 * 1024 * 1024 * 1024 {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "portable Orbit pack expands beyond the 8 GiB safety limit"
            )));
        }
        let destination = directory.path().join(&relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(destination)?;
        std::io::copy(&mut entry, &mut output)?;
        output.sync_all()?;
    }
    if !directory.path().join("orbit.toml").is_file()
        || !directory.path().join("orbit.lock").is_file()
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "portable Orbit pack must contain orbit.toml and orbit.lock"
        )));
    }
    ManifestFile::open(directory.path())?;
    Lockfile::open(directory.path())?;
    Ok(PortableInstance { directory })
}

pub fn consume_portable_instance(source: &Path) -> Result<(), OrbitError> {
    if source.exists() {
        std::fs::remove_file(source)?;
    }
    Ok(())
}

fn portable_entry_path(path: &Path) -> Option<PathBuf> {
    let components: Vec<_> = path.components().collect();
    if components.len() == 1
        && matches!(
            components[0].as_os_str().to_str(),
            Some("orbit.toml" | "orbit.lock" | "options.txt")
        )
    {
        return Some(path.to_path_buf());
    }
    if components.len() == 2
        && components[0].as_os_str() == "mods"
        && components[1]
            .as_os_str()
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".jar")
    {
        return Some(path.to_path_buf());
    }
    if components.len() >= 2
        && matches!(
            components[0].as_os_str().to_str(),
            Some("config" | "defaultconfigs" | "serverconfig")
        )
    {
        return Some(path.to_path_buf());
    }
    None
}

pub fn export_instance(
    instance_dir: &Path,
    output: &Path,
    target: Option<String>,
    format: &str,
    dry_run: bool,
    progress: Option<ProgressReporter>,
) -> Result<ExportReport, OrbitError> {
    if !matches!(format, "zip" | "mrpack") {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported export format '{format}'; expected zip or mrpack"
        )));
    }
    let manifest = ManifestFile::open(instance_dir)?;
    let lockfile = Lockfile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest.inner)?;
    let loader_package = platform.loader_package;
    let (selected, _) = crate::installer::selected_packages(
        &manifest.inner,
        &lockfile.inner,
        &PackageSelection {
            target,
            ..PackageSelection::default()
        },
        loader_package.as_ref(),
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
    let config_sources = portable_config_sources(instance_dir)?;
    let config_bytes: u64 = config_sources.iter().map(|source| source.bytes).sum();
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
    let total_work = if dry_run {
        package_bytes
    } else {
        package_bytes + archived_bytes + config_bytes
    };
    let mut tracker = ExportTracker::new(progress, sources.len(), total_work);
    tracker.started();
    for (index, (entry, source, source_bytes)) in sources.iter().enumerate() {
        if entry.sha256.is_empty() {
            tracker.advance(*source_bytes);
        } else {
            let actual = compute_sha256_with_progress(source, &mut tracker)?;
            if actual != entry.sha256 {
                return Err(OrbitError::ChecksumMismatch {
                    name: entry.filename.clone(),
                    expected: entry.sha256.clone(),
                    actual,
                });
            }
        }
        tracker.complete_package(index + 1);
    }
    let report = ExportReport {
        path: output.to_path_buf(),
        packages: sources.len(),
        bytes: package_bytes + config_bytes,
    };
    if dry_run {
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
            &portable_manifest,
            &portable_lock_toml,
            &sources,
            &mut tracker,
        )?;
    } else {
        add_content(
            &mut archive,
            "orbit.toml",
            portable_manifest_toml.as_bytes(),
            metadata_options,
        )?;
        add_content(
            &mut archive,
            "orbit.lock",
            portable_lock_toml.as_bytes(),
            metadata_options,
        )?;
        for (entry, source, _) in &sources {
            add_file(
                &mut archive,
                &format!("mods/{}", entry.filename),
                source,
                artifact_options,
                Some(&mut tracker),
            )?;
        }
    }
    for source in &config_sources {
        let relative = archive_path(&source.relative);
        let destination = if format == "mrpack" {
            format!("overrides/{relative}")
        } else {
            relative
        };
        add_file(
            &mut archive,
            &destination,
            &source.source,
            metadata_options,
            Some(&mut tracker),
        )?;
    }
    archive.finish()?.sync_all()?;
    if output.exists() {
        std::fs::remove_file(output)?;
    }
    std::fs::rename(&temporary, output)?;
    pending.commit();
    tracker.finished();
    Ok(report)
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

fn compute_sha256_with_progress(
    source: &Path,
    progress: &mut ExportTracker,
) -> Result<String, OrbitError> {
    use sha2::{Digest as _, Sha256};

    let mut file = std::fs::File::open(source)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        progress.advance(read as u64);
    }
    Ok(hex::encode(digest.finalize()))
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

#[derive(Debug)]
pub(crate) struct PortableFile {
    pub source: PathBuf,
    pub relative: PathBuf,
    pub bytes: u64,
}

pub(crate) fn portable_config_sources(
    instance_dir: &Path,
) -> Result<Vec<PortableFile>, OrbitError> {
    const ROOTS: [&str; 4] = ["config", "defaultconfigs", "serverconfig", "options.txt"];
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

fn archive_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
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
    fn archive_import_extracts_only_safe_mod_paths() {
        let directory = test_dir("zip-import");
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("pack.zip");
        let file = std::fs::File::create(&source).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("mods/example.jar", options).unwrap();
        zip.write_all(b"example").unwrap();
        zip.start_file("../mods/escape.jar", options).unwrap();
        zip.write_all(b"escape").unwrap();
        zip.finish().unwrap();

        let report = import_archive(&directory, &source, false, false).unwrap();

        assert_eq!(report.extracted, vec!["example.jar"]);
        assert!(directory.join("mods/example.jar").is_file());
        assert!(!directory.join("escape.jar").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn zip_export_contains_manifest_lock_and_selected_jar() {
        let directory = test_dir("zip-export");
        crate::platform_detection::test_support::write_platform(&directory, "1", "fabric", "1");
        std::fs::create_dir_all(directory.join("mods")).unwrap();
        ManifestFile::new(&directory, detected_manifest(&directory, "example"))
            .save()
            .unwrap();
        let jar = directory.join("mods/example.jar");
        std::fs::write(&jar, b"example").unwrap();
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
                    sha256: crate::jar::compute_sha256(&jar).unwrap(),
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
        let output = directory.join("pack.zip");

        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = events.clone();
        let progress: ProgressReporter = std::sync::Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });
        let report =
            export_instance(&directory, &output, None, "zip", false, Some(progress)).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();

        assert_eq!(report.packages, 1);
        assert!(archive.by_name("orbit.toml").is_ok());
        assert!(archive.by_name("orbit.lock").is_ok());
        assert!(archive.by_name("config/example.toml").is_ok());
        let jar = archive.by_name("mods/example.jar").unwrap();
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
                suffix: "all".to_string(),
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

        export_instance(&directory, &output, None, "mrpack", false, None).unwrap();
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
        assert!(archive.by_name("overrides/orbit.toml").is_ok());
        assert!(archive.by_name("overrides/orbit.lock").is_ok());
        assert!(archive.by_name("orbit.toml").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn mrpack_import_prefers_a_bundled_override_to_its_download_entry() {
        let directory = test_dir("mrpack-import-override");
        std::fs::create_dir_all(&directory).unwrap();
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
                    "files": [{
                        "path": "mods/bundled.jar",
                        "hashes": { "sha1": "unused", "sha512": "unused" },
                        "downloads": ["https://invalid.example/bundled.jar"],
                        "fileSize": 999
                    }],
                    "dependencies": { "minecraft": "1.21.1" }
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

        let report = import_mrpack(&directory, &source, false, false)
            .await
            .unwrap();

        assert_eq!(report.extracted, vec!["bundled.jar"]);
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

        let error = import_mrpack(&directory, &source, false, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("unsafe path"));
        assert!(!directory.join("escape.jar").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
