use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_bundle_format::{
    InstanceTarget, MrpackArchive, MrpackEnvironment, MrpackFile, MrpackHashes, MrpackIndex,
    MrpackSideRequirement,
};
use serde_json::to_vec_pretty;
use zip::write::SimpleFileOptions;

use super::{ExportTracker, ImportReport, PortableFile, add_file};
use crate::error::OrbitError;
use crate::progress::{ProgressEvent, ProgressReporter, emit as emit_progress};

const ALLOWED_DOWNLOAD_HOSTS: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];
const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024;
const MAX_TOTAL_SIZE: u64 = 8 * 1024 * 1024 * 1024;

/// Install the Orbit-owned half of an official Modrinth pack.
///
/// Runtime dependencies are checked against the actual instance first. Every
/// download and override is materialized in staging, then committed as one
/// rollback-capable filesystem transaction. Launcher-owned paths are rejected.
pub async fn import_mrpack(
    instance_dir: &Path,
    source: &Path,
    overwrite: bool,
    include_all_optional: bool,
    optional_files: &BTreeSet<String>,
    dry_run: bool,
    progress: Option<ProgressReporter>,
) -> Result<ImportReport, OrbitError> {
    let pack = MrpackArchive::open(source)?;
    let target = validate_runtime(instance_dir, &pack)?;
    let transaction_root = tempfile::Builder::new()
        .prefix(".orbit-mrpack-")
        .tempdir_in(instance_dir)?;
    let staging = transaction_root.path().join("staging");
    std::fs::create_dir_all(&staging)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let available_optional = pack
        .index
        .files
        .iter()
        .filter(|file| file.requirement(target) == MrpackSideRequirement::Optional)
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(path) = optional_files
        .iter()
        .find(|path| !available_optional.contains(path.as_str()))
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "mrpack optional file '{path}' is not optional for the selected target"
        )));
    }
    let selected_files = pack
        .index
        .files
        .iter()
        .filter(|file| match file.requirement(target) {
            MrpackSideRequirement::Required => true,
            MrpackSideRequirement::Optional => {
                include_all_optional || optional_files.contains(&file.path)
            }
            MrpackSideRequirement::Unsupported => false,
        })
        .collect::<Vec<_>>();
    let selected_overrides = pack.overrides_for(target).collect::<Vec<_>>();
    let mut total_bytes = 0_u64;
    for file in &selected_files {
        validate_instance_payload_path(&file.path)?;
        total_bytes = total_bytes
            .checked_add(file.file_size)
            .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("mrpack payload size overflowed")))?;
        if file.file_size > MAX_FILE_SIZE || total_bytes > MAX_TOTAL_SIZE {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack payload exceeds the configured safety limit"
            )));
        }
    }
    for entry in &selected_overrides {
        validate_instance_payload_path(&entry.relative_path)?;
        total_bytes = total_bytes
            .checked_add(entry.size)
            .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("mrpack override size overflowed")))?;
        if total_bytes > MAX_TOTAL_SIZE {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack payload exceeds the configured safety limit"
            )));
        }
    }
    if dry_run {
        let planned = selected_files
            .iter()
            .map(|file| file.path.clone())
            .chain(
                selected_overrides
                    .iter()
                    .map(|entry| entry.relative_path.clone()),
            )
            .collect::<BTreeSet<_>>();
        return Ok(plan_staging(instance_dir, planned, overwrite));
    }

    let files = selected_files.len() + selected_overrides.len();
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ImportStarted { files, total_bytes },
    );
    let mut completed_bytes = 0_u64;
    let mut completed_files = 0_usize;
    for file in selected_files {
        download_to_staging(&client, file, &staging.join(&file.path), |advanced| {
            completed_bytes = completed_bytes.saturating_add(advanced);
            emit_progress(
                progress.as_ref(),
                ProgressEvent::ImportAdvanced {
                    completed_bytes,
                    total_bytes,
                    completed_files,
                    files,
                },
            );
        })
        .await?;
        completed_files += 1;
        emit_progress(
            progress.as_ref(),
            ProgressEvent::ImportAdvanced {
                completed_bytes,
                total_bytes,
                completed_files,
                files,
            },
        );
    }
    extract_overrides(
        source,
        &selected_overrides,
        &staging,
        |advanced, extracted| {
            completed_bytes = completed_bytes.saturating_add(advanced);
            emit_progress(
                progress.as_ref(),
                ProgressEvent::ImportAdvanced {
                    completed_bytes,
                    total_bytes,
                    completed_files: completed_files + extracted,
                    files,
                },
            );
        },
    )?;
    completed_files += selected_overrides.len();
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ImportFinished {
            files: completed_files,
            total_bytes,
        },
    );
    let planned = collect_relative_files(&staging)?;
    commit_staging(instance_dir, &staging, planned, overwrite)
}

fn validate_runtime(
    instance_dir: &Path,
    pack: &MrpackArchive,
) -> Result<InstanceTarget, OrbitError> {
    let manifest = crate::workspace::ManifestFile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest.inner)?;
    let expected = pack.runtime()?;
    let actual_loader_version =
        (platform.loader.as_str() != "vanilla").then_some(platform.loader_version.as_str());
    if expected.minecraft != platform.minecraft_version.id
        || expected.loader != platform.loader.as_str()
        || expected.loader_version.as_deref() != actual_loader_version
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "mrpack requires Minecraft {} / {} {}, but the instance is Minecraft {} / {} {}",
            expected.minecraft,
            expected.loader,
            expected.loader_version.as_deref().unwrap_or(""),
            platform.minecraft_version.id,
            platform.loader,
            platform.loader_version
        )));
    }
    Ok(match platform.physical_environment {
        crate::metadata::Environment::Client => InstanceTarget::Client,
        crate::metadata::Environment::Server => InstanceTarget::Server,
        crate::metadata::Environment::Both => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack import requires an explicitly detected client or server instance"
            )));
        }
    })
}

async fn download_to_staging<F>(
    client: &reqwest::Client,
    file: &MrpackFile,
    destination: &Path,
    mut progress: F,
) -> Result<(), OrbitError>
where
    F: FnMut(u64),
{
    let url = file
        .downloads
        .iter()
        .find(|download| allowed_download_url(download))
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "mrpack file '{}' has no allowed HTTPS download URL",
                file.path
            ))
        })?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length != file.file_size || length > MAX_FILE_SIZE)
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "mrpack response size disagrees with '{}'",
            file.path
        )));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = std::fs::File::create(destination)?;
    let mut sha1 = sha1::Sha1::new();
    let mut sha512 = sha2::Sha512::new();
    let mut bytes = 0_u64;
    use sha1::Digest as _;
    while let Some(chunk) = response.chunk().await? {
        bytes = bytes.saturating_add(chunk.len() as u64);
        if bytes > file.file_size || bytes > MAX_FILE_SIZE {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack response exceeded declared size for '{}'",
                file.path
            )));
        }
        sha1.update(&chunk);
        sha512.update(&chunk);
        output.write_all(&chunk)?;
        progress(chunk.len() as u64);
    }
    output.sync_all()?;
    if bytes != file.file_size
        || !hex::encode(sha1.finalize()).eq_ignore_ascii_case(&file.hashes.sha1)
        || !hex::encode(sha512.finalize()).eq_ignore_ascii_case(&file.hashes.sha512)
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "mrpack content verification failed for '{}'",
            file.path
        )));
    }
    Ok(())
}

fn extract_overrides<F>(
    source: &Path,
    entries: &[&orbit_bundle_format::MrpackOverride],
    staging: &Path,
    mut progress: F,
) -> Result<(), OrbitError>
where
    F: FnMut(u64, usize),
{
    let mut archive = zip::ZipArchive::new(std::fs::File::open(source)?)?;
    let mut buffer = vec![0_u8; 128 * 1024];
    for (index, expected) in entries.iter().enumerate() {
        let mut entry = archive.by_name(&expected.archive_path)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack override '{}' is a symbolic link",
                expected.archive_path
            )));
        }
        let destination = staging.join(&expected.relative_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(destination)?;
        loop {
            use std::io::Read as _;
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            progress(read as u64, index);
        }
        output.sync_all()?;
        progress(0, index + 1);
    }
    Ok(())
}

pub(super) fn commit_staging(
    instance_dir: &Path,
    staging: &Path,
    files: BTreeSet<String>,
    overwrite: bool,
) -> Result<ImportReport, OrbitError> {
    let backup = staging
        .parent()
        .expect("staging has transaction parent")
        .join("backup");
    std::fs::create_dir_all(&backup)?;
    let mut applied = Vec::new();
    let mut backed_up = Vec::new();
    let mut report = ImportReport::default();
    for relative in files {
        let source = staging.join(&relative);
        let destination = instance_dir.join(&relative);
        validate_destination(instance_dir, &destination)?;
        if destination.exists() && !overwrite {
            report.kept.push(relative);
            continue;
        }
        let result = (|| -> Result<(), OrbitError> {
            if destination.exists() {
                let backup_path = backup.join(&relative);
                if let Some(parent) = backup_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&destination, &backup_path)?;
                backed_up.push((backup_path, destination.clone()));
            }
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&source, &destination)?;
            applied.push(destination);
            Ok(())
        })();
        if let Err(error) = result {
            return match rollback_files(&applied, &backed_up) {
                Ok(()) => Err(error),
                Err(rollback) => Err(OrbitError::Other(anyhow::anyhow!(
                    "import transaction failed: {error}; rollback also failed: {rollback}"
                ))),
            };
        }
        report.extracted.push(relative);
    }
    report.extracted.sort();
    report.kept.sort();
    Ok(report)
}

pub(super) fn plan_staging(
    instance_dir: &Path,
    files: BTreeSet<String>,
    overwrite: bool,
) -> ImportReport {
    let mut report = ImportReport::default();
    for relative in files {
        if instance_dir.join(&relative).exists() && !overwrite {
            report.kept.push(relative);
        } else {
            report.extracted.push(relative);
        }
    }
    report
}

fn rollback_files(applied: &[PathBuf], backed_up: &[(PathBuf, PathBuf)]) -> Result<(), OrbitError> {
    let mut failures = Vec::new();
    for path in applied.iter().rev() {
        if let Err(error) = std::fs::remove_file(path) {
            failures.push(format!("remove '{}': {error}", path.display()));
        }
    }
    for (source, destination) in backed_up.iter().rev() {
        if let Err(error) = std::fs::rename(source, destination) {
            failures.push(format!(
                "restore '{}' to '{}': {error}",
                source.display(),
                destination.display()
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(OrbitError::Other(anyhow::anyhow!(failures.join("; "))))
    }
}

pub(super) fn collect_relative_files(root: &Path) -> Result<BTreeSet<String>, OrbitError> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeSet<String>) -> Result<(), OrbitError> {
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "mrpack staging unexpectedly contains a symbolic link"
                )));
            }
            if metadata.is_dir() {
                visit(root, &path, output)?;
            } else if metadata.is_file() {
                output.insert(
                    path.strip_prefix(root)
                        .expect("visited path is beneath root")
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
        Ok(())
    }
    let mut output = BTreeSet::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

fn validate_instance_payload_path(relative: &str) -> Result<(), OrbitError> {
    orbit_bundle_format::validate_relative_path(relative)?;
    let first = relative.split('/').next().unwrap_or_default();
    const RESERVED_ROOTS: &[&str] = &[
        ".orbit",
        ".orbit-launcher",
        "assets",
        "libraries",
        "runtime",
        "versions",
    ];
    const RESERVED_FILES: &[&str] = &[
        "bundle.toml",
        "minecraft.jar",
        "orbit.toml",
        "orbit.lock",
        "orbit-launcher.toml",
        "orbit-launcher.lock",
        "eula.txt",
    ];
    if RESERVED_ROOTS.contains(&first) || RESERVED_FILES.contains(&relative) {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "mrpack path '{relative}' belongs to Orbit or Launcher and cannot be installed as pack content"
        )));
    }
    Ok(())
}

fn validate_destination(instance: &Path, destination: &Path) -> Result<(), OrbitError> {
    let relative = destination.strip_prefix(instance).map_err(|_| {
        OrbitError::Other(anyhow::anyhow!("mrpack destination escaped the instance"))
    })?;
    let mut current = instance.to_path_buf();
    for component in relative
        .components()
        .take(relative.components().count().saturating_sub(1))
    {
        current.push(component.as_os_str());
        if current.exists()
            && std::fs::symlink_metadata(&current)?
                .file_type()
                .is_symlink()
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack destination traverses symbolic link '{}'",
                current.display()
            )));
        }
    }
    Ok(())
}

fn allowed_download_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url
                .host_str()
                .is_some_and(|host| ALLOWED_DOWNLOAD_HOSTS.contains(&host))
    })
}

pub(super) struct MrpackContents<'a> {
    pub manifest: &'a crate::manifest::OrbitManifest,
    pub packages: &'a [(&'a crate::lockfile::PackageEntry, PathBuf, u64)],
    pub state: &'a [PortableFile],
    pub target: Option<InstanceTarget>,
}

pub(super) fn write_contents(
    archive: &mut zip::ZipWriter<std::fs::File>,
    metadata_options: SimpleFileOptions,
    artifact_options: SimpleFileOptions,
    contents: MrpackContents<'_>,
    progress: &mut ExportTracker,
) -> Result<(), OrbitError> {
    let index = build_index(contents.manifest, contents.packages);
    index.validate()?;
    archive.start_file(orbit_bundle_format::MRPACK_INDEX_PATH, metadata_options)?;
    archive.write_all(&to_vec_pretty(&index)?)?;
    for (entry, source, _) in contents.packages {
        if is_embedded(entry) {
            let environment = environment(contents.manifest, entry);
            let prefix = override_prefix(environment, contents.target);
            add_file(
                archive,
                &format!("{prefix}mods/{}", entry.filename),
                source,
                artifact_options,
                Some(progress),
            )?;
        }
    }
    let prefix = match contents.target {
        Some(InstanceTarget::Client) => "client-overrides/",
        Some(InstanceTarget::Server) => "server-overrides/",
        None => "overrides/",
    };
    for source in contents.state {
        add_file(
            archive,
            &format!("{prefix}{}", super::archive_path(&source.relative)),
            &source.source,
            metadata_options,
            Some(progress),
        )?;
    }
    Ok(())
}

fn build_index(
    manifest: &crate::manifest::OrbitManifest,
    sources: &[(&crate::lockfile::PackageEntry, PathBuf, u64)],
) -> MrpackIndex {
    let files = sources
        .iter()
        .filter_map(|(entry, _, bytes)| {
            let download = download_url(entry)?;
            Some(MrpackFile {
                path: format!("mods/{}", entry.filename),
                hashes: MrpackHashes {
                    sha1: entry.sha1.clone(),
                    sha512: entry.sha512.clone(),
                },
                env: environment(manifest, entry),
                downloads: vec![download.to_string()],
                file_size: *bytes,
            })
        })
        .collect();
    let loader_key = match manifest.project.modloader.as_str() {
        "fabric" => "fabric-loader",
        "quilt" => "quilt-loader",
        other => other,
    };
    let mut dependencies =
        BTreeMap::from([("minecraft".to_string(), manifest.project.mc_version.clone())]);
    if manifest.project.modloader != "vanilla" {
        dependencies.insert(
            loader_key.to_string(),
            manifest.project.modloader_version.clone(),
        );
    }
    MrpackIndex {
        format_version: orbit_bundle_format::MRPACK_FORMAT_VERSION,
        game: "minecraft".to_string(),
        version_id: manifest
            .project
            .version
            .clone()
            .unwrap_or_else(|| "1.0.0".to_string()),
        name: manifest.project.name.clone(),
        summary: manifest.project.description.clone(),
        files,
        dependencies,
    }
}

fn download_url(entry: &crate::lockfile::PackageEntry) -> Option<&str> {
    if entry.sha1.is_empty() || entry.sha512.is_empty() {
        return None;
    }
    entry
        .artifact_sources
        .iter()
        .filter_map(|source| match source {
            crate::lockfile::ArtifactSource::Modrinth { download_url, .. }
            | crate::lockfile::ArtifactSource::Curseforge { download_url, .. } => {
                Some(download_url.as_str())
            }
            crate::lockfile::ArtifactSource::File { .. } => None,
        })
        .find(|url| allowed_download_url(url))
}

pub(super) fn is_embedded(entry: &crate::lockfile::PackageEntry) -> bool {
    download_url(entry).is_none()
}

fn environment(
    manifest: &crate::manifest::OrbitManifest,
    entry: &crate::lockfile::PackageEntry,
) -> MrpackEnvironment {
    let Some(requirement) = manifest.packages.get(&entry.mod_id) else {
        return MrpackEnvironment::default();
    };
    let supported = if requirement.optional() {
        MrpackSideRequirement::Optional
    } else {
        MrpackSideRequirement::Required
    };
    match requirement.effective_environment(entry.environment) {
        crate::metadata::Environment::Client => MrpackEnvironment {
            client: supported,
            server: MrpackSideRequirement::Unsupported,
        },
        crate::metadata::Environment::Server => MrpackEnvironment {
            client: MrpackSideRequirement::Unsupported,
            server: supported,
        },
        crate::metadata::Environment::Both => MrpackEnvironment {
            client: supported,
            server: supported,
        },
    }
}

fn override_prefix(
    environment: MrpackEnvironment,
    _target: Option<InstanceTarget>,
) -> &'static str {
    if environment.client == MrpackSideRequirement::Unsupported {
        "server-overrides/"
    } else if environment.server == MrpackSideRequirement::Unsupported {
        "client-overrides/"
    } else {
        "overrides/"
    }
}
