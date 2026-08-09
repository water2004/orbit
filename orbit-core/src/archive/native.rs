use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_bundle_format::{
    BUNDLE_FORMAT_VERSION, BUNDLE_MANIFEST_PATH, BundleArchive, BundleFile, BundleFileOwner,
    BundleManifest, InstanceTarget, LauncherContent, LauncherSection, OrbitContent, OrbitSection,
    RuntimeRequirement,
};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

use super::{
    ExportTracker, ImportReport, PORTABLE_DATA_MANIFEST_PATH, PORTABLE_DATA_SCHEMA,
    PORTABLE_OWNERSHIP_PATH, PortableDataManifest, PortableFile, PortableInstance, add_content,
    archive_path,
};
use crate::error::OrbitError;
use crate::progress::{ProgressEvent, ProgressReporter, emit as emit_progress};
use crate::workspace::{Lockfile, ManifestFile};

const ORBIT_MANIFEST: &str = "orbit/orbit.toml";
const ORBIT_LOCK: &str = "orbit/orbit.lock";
const ORBIT_OWNERSHIP: &str = "orbit/.orbit/runtime-data/ownership.toml";
const ORBIT_DATA_MANIFEST: &str = "orbit/.orbit/portable-data.toml";

pub(super) struct NativeContents<'a> {
    pub manifest: &'a crate::manifest::OrbitManifest,
    pub manifest_toml: &'a str,
    pub lock_toml: &'a str,
    pub packages: &'a [(&'a crate::lockfile::PackageEntry, PathBuf, u64)],
    pub state: &'a [PortableFile],
    pub ownership: Option<&'a str>,
    pub data_manifest: Option<&'a str>,
    pub targets: Vec<InstanceTarget>,
    pub content: OrbitContent,
}

pub(super) fn write_contents(
    archive: &mut zip::ZipWriter<std::fs::File>,
    metadata_options: SimpleFileOptions,
    artifact_options: SimpleFileOptions,
    contents: NativeContents<'_>,
    progress: &mut ExportTracker,
) -> Result<(), OrbitError> {
    let mut files = Vec::new();
    add_bytes(
        archive,
        ORBIT_MANIFEST,
        contents.manifest_toml.as_bytes(),
        metadata_options,
        &mut files,
    )?;
    add_bytes(
        archive,
        ORBIT_LOCK,
        contents.lock_toml.as_bytes(),
        metadata_options,
        &mut files,
    )?;
    for (index, (entry, source, bytes)) in contents.packages.iter().enumerate() {
        let path = format!("orbit/mods/{}", entry.filename);
        let record = add_hashed_file(
            archive,
            path,
            source,
            *bytes,
            (!entry.sha256.is_empty()).then_some(entry.sha256.as_str()),
            artifact_options,
            progress,
        )?;
        files.push(record);
        progress.complete_package(index + 1);
    }
    if let (Some(ownership), Some(data_manifest)) = (contents.ownership, contents.data_manifest) {
        add_bytes(
            archive,
            ORBIT_OWNERSHIP,
            ownership.as_bytes(),
            metadata_options,
            &mut files,
        )?;
        progress.advance(ownership.len() as u64);
        add_bytes(
            archive,
            ORBIT_DATA_MANIFEST,
            data_manifest.as_bytes(),
            metadata_options,
            &mut files,
        )?;
        progress.advance(data_manifest.len() as u64);
        for source in contents.state {
            let path = format!("orbit/{}", archive_path(&source.relative)?);
            files.push(add_hashed_file(
                archive,
                path,
                &source.source,
                source.bytes,
                None,
                metadata_options,
                progress,
            )?);
        }
    }

    let bundle = BundleManifest {
        format_version: BUNDLE_FORMAT_VERSION,
        id: contents.manifest.project.name.clone(),
        name: contents.manifest.project.name.clone(),
        version: contents
            .manifest
            .project
            .version
            .clone()
            .unwrap_or_else(|| "1.0.0".to_string()),
        summary: contents.manifest.project.description.clone(),
        targets: contents.targets,
        runtime: RuntimeRequirement {
            minecraft: contents.manifest.project.mc_version.clone(),
            loader: contents.manifest.project.modloader.clone(),
            loader_version: (contents.manifest.project.modloader != "vanilla")
                .then(|| contents.manifest.project.modloader_version.clone()),
        },
        launcher: Some(LauncherSection {
            content: LauncherContent::RuntimeOnly,
        }),
        orbit: Some(OrbitSection {
            content: contents.content,
            manifest: ORBIT_MANIFEST.to_string(),
            lock: ORBIT_LOCK.to_string(),
            ownership: (contents.content == OrbitContent::ModsAndData)
                .then(|| ORBIT_OWNERSHIP.to_string()),
            data_manifest: (contents.content == OrbitContent::ModsAndData)
                .then(|| ORBIT_DATA_MANIFEST.to_string()),
        }),
        files,
    };
    bundle.validate()?;
    let document = toml::to_string_pretty(&bundle).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "failed to serialize Orbit bundle manifest: {error}"
        ))
    })?;
    archive.start_file(BUNDLE_MANIFEST_PATH, metadata_options)?;
    archive.write_all(document.as_bytes())?;
    Ok(())
}

fn add_hashed_file(
    archive: &mut zip::ZipWriter<std::fs::File>,
    archive_path: String,
    source: &Path,
    expected_size: u64,
    expected_sha256: Option<&str>,
    options: SimpleFileOptions,
    progress: &mut ExportTracker,
) -> Result<BundleFile, OrbitError> {
    archive.start_file(&archive_path, options)?;
    let mut input = std::fs::File::open(source)?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        use std::io::Read as _;
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size.checked_add(read as u64).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!("source file size overflowed during export"))
        })?;
        digest.update(&buffer[..read]);
        archive.write_all(&buffer[..read])?;
        progress.advance(read as u64);
    }
    let sha256 = hex::encode(digest.finalize());
    if size != expected_size {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "source '{}' changed size during export: expected {expected_size}, read {size}",
            source.display()
        )));
    }
    if let Some(expected) = expected_sha256
        && sha256 != expected
    {
        return Err(OrbitError::ChecksumMismatch {
            name: source.display().to_string(),
            expected: expected.to_string(),
            actual: sha256,
        });
    }
    Ok(file_record(archive_path, size, sha256))
}

fn add_bytes(
    archive: &mut zip::ZipWriter<std::fs::File>,
    path: &str,
    content: &[u8],
    options: SimpleFileOptions,
    files: &mut Vec<BundleFile>,
) -> Result<(), OrbitError> {
    add_content(archive, path, content, options)?;
    files.push(file_record(
        path.to_string(),
        content.len() as u64,
        hex::encode(Sha256::digest(content)),
    ));
    Ok(())
}

fn file_record(path: String, size: u64, sha256: String) -> BundleFile {
    BundleFile {
        path,
        owner: BundleFileOwner::Orbit,
        size,
        sha256,
    }
}

pub(super) fn extract_portable_instance(source: &Path) -> Result<PortableInstance, OrbitError> {
    extract_portable_instance_with_progress(source, None)
}

fn extract_portable_instance_with_progress(
    source: &Path,
    progress: Option<ProgressReporter>,
) -> Result<PortableInstance, OrbitError> {
    let bundle = BundleArchive::open(source)?;
    let orbit = bundle.manifest.orbit.as_ref().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "Orbit bundle contains no Orbit-owned content"
        ))
    })?;
    validate_orbit_layout(&bundle, orbit)?;
    let directory = std::sync::Arc::new(tempfile::tempdir()?);
    let files = bundle
        .manifest
        .files
        .iter()
        .filter(|file| file.owner == BundleFileOwner::Orbit)
        .count();
    let total_bytes = bundle
        .manifest
        .files
        .iter()
        .filter(|file| file.owner == BundleFileOwner::Orbit)
        .map(|file| file.size)
        .sum::<u64>();
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ImportStarted { files, total_bytes },
    );
    bundle.extract_owner_with_progress(
        BundleFileOwner::Orbit,
        directory.path(),
        |completed_bytes, total_bytes, completed_files, files| {
            emit_progress(
                progress.as_ref(),
                ProgressEvent::ImportAdvanced {
                    completed_bytes,
                    total_bytes,
                    completed_files,
                    files,
                },
            );
        },
    )?;
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ImportFinished { files, total_bytes },
    );
    let manifest = ManifestFile::open(directory.path())?;
    let lockfile = Lockfile::open(directory.path())?;
    let manifest_packages = manifest.inner.packages.keys().collect::<BTreeSet<_>>();
    let locked_packages = lockfile
        .inner
        .packages
        .iter()
        .map(|entry| &entry.mod_id)
        .collect::<BTreeSet<_>>();
    if manifest_packages != locked_packages {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "portable Orbit manifest and lock describe different package sets"
        )));
    }
    for entry in &lockfile.inner.packages {
        if !directory
            .path()
            .join("mods")
            .join(&entry.filename)
            .is_file()
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "portable Orbit pack is missing JAR '{}' for package '{}'",
                entry.filename,
                entry.mod_id
            )));
        }
    }
    if orbit.content == OrbitContent::ModsAndData {
        validate_portable_data(directory.path(), source)?;
    }
    Ok(PortableInstance { directory })
}

pub fn import_bundle(
    instance_dir: &Path,
    source: &Path,
    overwrite: bool,
    dry_run: bool,
    progress: Option<ProgressReporter>,
) -> Result<ImportReport, OrbitError> {
    let bundle = BundleArchive::open(source)?;
    validate_target_runtime(instance_dir, &bundle)?;
    let portable = extract_portable_instance_with_progress(source, progress)?;
    let files = super::mrpack::collect_relative_files(portable.path())?;
    let visible = files
        .iter()
        .filter(|path| path.as_str() != PORTABLE_DATA_MANIFEST_PATH)
        .cloned()
        .collect::<BTreeSet<_>>();
    for relative in &visible {
        super::mrpack::validate_import_destination(instance_dir, relative)?;
    }
    if dry_run {
        return Ok(super::mrpack::plan_staging(
            instance_dir,
            visible,
            overwrite,
        ));
    }

    let target = ManifestFile::open(instance_dir)?;
    let mut incoming = ManifestFile::open(portable.path())?.inner;
    incoming.project.name = target.inner.project.name;
    incoming.project.mc_version = target.inner.project.mc_version;
    incoming.project.modloader = target.inner.project.modloader;
    incoming.project.modloader_version = target.inner.project.modloader_version;
    incoming.platform = target.inner.platform;
    ManifestFile::new(portable.path(), incoming).save()?;
    let data_manifest = portable.path().join(PORTABLE_DATA_MANIFEST_PATH);
    if data_manifest.exists() {
        std::fs::remove_file(data_manifest)?;
    }
    super::mrpack::commit_staging(instance_dir, portable.path(), visible, overwrite)
}

fn validate_orbit_layout(bundle: &BundleArchive, orbit: &OrbitSection) -> Result<(), OrbitError> {
    if orbit.manifest != ORBIT_MANIFEST || orbit.lock != ORBIT_LOCK {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "Orbit bundle uses a non-canonical Orbit manifest layout"
        )));
    }
    if orbit.content == OrbitContent::ModsAndData
        && (orbit.ownership.as_deref() != Some(ORBIT_OWNERSHIP)
            || orbit.data_manifest.as_deref() != Some(ORBIT_DATA_MANIFEST))
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "Orbit bundle uses a non-canonical data layout"
        )));
    }
    if orbit.content == OrbitContent::Mods
        && bundle
            .manifest
            .files
            .iter()
            .filter(|file| file.owner == BundleFileOwner::Orbit)
            .any(|file| {
                file.path != ORBIT_MANIFEST
                    && file.path != ORBIT_LOCK
                    && !file.path.starts_with("orbit/mods/")
            })
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "mods-only Orbit bundle contains non-mod package data"
        )));
    }
    Ok(())
}

fn validate_target_runtime(instance_dir: &Path, bundle: &BundleArchive) -> Result<(), OrbitError> {
    let manifest = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest.inner)?;
    let runtime = &bundle.manifest.runtime;
    let actual_loader_version =
        (platform.loader.as_str() != "vanilla").then_some(platform.loader_version.as_str());
    let actual_target = match platform.physical_environment {
        crate::metadata::Environment::Client => InstanceTarget::Client,
        crate::metadata::Environment::Server => InstanceTarget::Server,
        crate::metadata::Environment::Both => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "Orbit bundle import requires an explicitly detected client or server instance"
            )));
        }
    };
    if runtime.minecraft != platform.minecraft_version.id
        || runtime.loader != platform.loader.as_str()
        || runtime.loader_version.as_deref() != actual_loader_version
        || !bundle.manifest.targets.contains(&actual_target)
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "Orbit bundle runtime does not match the target instance"
        )));
    }
    Ok(())
}

fn validate_portable_data(directory: &Path, source: &Path) -> Result<(), OrbitError> {
    let data_path = directory.join(PORTABLE_DATA_MANIFEST_PATH);
    let document = std::fs::read_to_string(&data_path)?;
    let manifest: PortableDataManifest = toml::from_str(&document).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "invalid portable Orbit data manifest '{}': {error}",
            source.display()
        ))
    })?;
    if manifest.schema != PORTABLE_DATA_SCHEMA {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported portable Orbit data schema {}",
            manifest.schema
        )));
    }
    let ownership_path = directory.join(PORTABLE_OWNERSHIP_PATH);
    let ownership = crate::runtime_data::parse_ownership_document(
        source,
        &std::fs::read_to_string(ownership_path)?,
    )?;
    if ownership != manifest.ownership {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "portable Orbit data manifest does not match its ownership ledger"
        )));
    }
    for relative in &manifest.files {
        orbit_bundle_format::validate_relative_path(relative)?;
        if !directory.join(relative).is_file() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "portable Orbit pack is missing declared file '{relative}'"
            )));
        }
    }
    let controls = [
        "orbit.toml",
        "orbit.lock",
        PORTABLE_OWNERSHIP_PATH,
        PORTABLE_DATA_MANIFEST_PATH,
    ];
    let actual = super::mrpack::collect_relative_files(directory)?
        .into_iter()
        .filter(|path| !controls.contains(&path.as_str()))
        .collect::<BTreeSet<_>>();
    if actual != manifest.files {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "portable Orbit data inventory does not exactly match its extracted payload"
        )));
    }
    Ok(())
}
