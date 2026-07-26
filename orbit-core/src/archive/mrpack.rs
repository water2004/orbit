use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;
use zip::write::SimpleFileOptions;

use super::{ImportReport, add_file, import_archive};
use crate::error::OrbitError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrpackIndex {
    files: Vec<MrpackFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrpackFile {
    path: String,
    hashes: MrpackHashes,
    downloads: Vec<String>,
    file_size: u64,
}

#[derive(Debug, Deserialize)]
struct MrpackHashes {
    sha1: String,
    sha512: String,
}

/// Import bundled override JARs and downloadable files from a Modrinth pack.
pub async fn import_mrpack(
    instance_dir: &Path,
    source: &Path,
    overwrite: bool,
    dry_run: bool,
) -> Result<ImportReport, OrbitError> {
    let mut report = import_archive(instance_dir, source, overwrite, dry_run)?;
    let index = read_index(source)?;
    let bundled: std::collections::HashSet<_> = report.extracted.iter().cloned().collect();
    let mods_dir = instance_dir.join("mods");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "failed to create mrpack download client: {error}"
            ))
        })?;
    let mut total_download_size = 0_u64;
    let mut indexed_files = std::collections::HashSet::new();

    for file in index.files {
        let Some(filename) = mod_filename(&file.path)? else {
            continue;
        };
        if !indexed_files.insert(filename.clone()) {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack index contains duplicate mod path '{}'",
                file.path
            )));
        }
        // Overrides are applied after downloads by the mrpack format, so a
        // bundled JAR is authoritative when both forms are present.
        if bundled.contains(&filename) {
            continue;
        }
        let destination = mods_dir.join(&filename);
        if destination.exists() && !overwrite {
            report.kept.push(filename);
            continue;
        }
        report.extracted.push(filename.clone());
        if dry_run {
            continue;
        }

        let download_url = validated_download_url(&file.downloads, &filename)?;
        let mut response = client
            .get(download_url)
            .send()
            .await
            .map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "failed to download mrpack file '{filename}': {error}"
                ))
            })?
            .error_for_status()
            .map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "failed to download mrpack file '{filename}': {error}"
                ))
            })?;
        const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_FILE_SIZE)
            || file.file_size > MAX_FILE_SIZE
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack file '{filename}' exceeds the 1 GiB safety limit"
            )));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(file.file_size.min(MAX_FILE_SIZE)).unwrap_or_default(),
        );
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "failed to read mrpack file '{filename}': {error}"
            ))
        })? {
            if bytes.len().saturating_add(chunk.len())
                > usize::try_from(MAX_FILE_SIZE).unwrap_or(usize::MAX)
            {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "mrpack file '{filename}' exceeds the 1 GiB safety limit"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() as u64 != file.file_size {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack file size mismatch for '{filename}': expected {}, got {}",
                file.file_size,
                bytes.len()
            )));
        }
        verify_hashes(&filename, &bytes, &file.hashes)?;
        total_download_size = total_download_size.saturating_add(bytes.len() as u64);
        if total_download_size > 4 * 1024 * 1024 * 1024 {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "mrpack contains more than 4 GiB of downloadable mod files"
            )));
        }
        write_jar(&mods_dir, &filename, &bytes, &destination)?;
    }

    report.extracted.sort();
    report.extracted.dedup();
    report.kept.sort();
    report.kept.dedup();
    Ok(report)
}

pub(super) fn write_contents(
    archive: &mut zip::ZipWriter<std::fs::File>,
    options: SimpleFileOptions,
    manifest: &crate::manifest::OrbitManifest,
    sources: &[(&crate::lockfile::PackageEntry, PathBuf)],
    instance_dir: &Path,
) -> Result<(), OrbitError> {
    let index = build_index(manifest, sources);
    archive.start_file("modrinth.index.json", options)?;
    archive.write_all(serde_json::to_string_pretty(&index)?.as_bytes())?;
    add_file(
        archive,
        "overrides/orbit.toml",
        &instance_dir.join("orbit.toml"),
        options,
    )?;
    add_file(
        archive,
        "overrides/orbit.lock",
        &instance_dir.join("orbit.lock"),
        options,
    )?;
    for (entry, source) in sources {
        if download_url(entry).is_none() {
            add_file(
                archive,
                &format!("overrides/mods/{}", entry.filename),
                source,
                options,
            )?;
        }
    }
    Ok(())
}

fn read_index(source: &Path) -> Result<MrpackIndex, OrbitError> {
    let file = std::fs::File::open(source)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut entry = archive.by_name("modrinth.index.json").map_err(|_| {
        OrbitError::Other(anyhow::anyhow!(
            "{} does not contain modrinth.index.json",
            source.display()
        ))
    })?;
    let mut content = String::new();
    entry.read_to_string(&mut content)?;
    serde_json::from_str(&content)
        .map_err(|error| OrbitError::Other(anyhow::anyhow!("invalid mrpack index: {error}")))
}

fn mod_filename(path: &str) -> Result<Option<String>, OrbitError> {
    use std::path::Component;

    let path = Path::new(path);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unsafe path in mrpack index: {}",
            path.display()
        )));
    }
    let components: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect();
    if components.len() != 2
        || !components[0].to_string_lossy().eq_ignore_ascii_case("mods")
        || !components[1]
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".jar")
    {
        return Ok(None);
    }
    Ok(Some(components[1].to_string_lossy().into_owned()))
}

fn validated_download_url<'a>(
    downloads: &'a [String],
    filename: &str,
) -> Result<&'a str, OrbitError> {
    const ALLOWED_HOSTS: &[&str] = &[
        "cdn.modrinth.com",
        "github.com",
        "raw.githubusercontent.com",
        "gitlab.com",
    ];
    for download in downloads {
        let Ok(url) = url::Url::parse(download) else {
            continue;
        };
        if url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url
                .host_str()
                .is_some_and(|host| ALLOWED_HOSTS.contains(&host))
        {
            return Ok(download);
        }
    }
    Err(OrbitError::Other(anyhow::anyhow!(
        "mrpack file '{filename}' has no allowed HTTPS download URL"
    )))
}

fn verify_hashes(filename: &str, bytes: &[u8], hashes: &MrpackHashes) -> Result<(), OrbitError> {
    if hashes.sha1.is_empty() || hashes.sha512.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "mrpack file '{filename}' must include SHA-1 and SHA-512 hashes"
        )));
    }
    let actual_sha1 = crate::jar::sha1_digest(bytes);
    if !actual_sha1.eq_ignore_ascii_case(&hashes.sha1) {
        return Err(OrbitError::ChecksumMismatch {
            name: filename.to_string(),
            expected: hashes.sha1.clone(),
            actual: actual_sha1,
        });
    }
    let actual_sha512 = crate::jar::sha512_digest(bytes);
    if !actual_sha512.eq_ignore_ascii_case(&hashes.sha512) {
        return Err(OrbitError::ChecksumMismatch {
            name: filename.to_string(),
            expected: hashes.sha512.clone(),
            actual: actual_sha512,
        });
    }
    Ok(())
}

fn write_jar(
    mods_dir: &Path,
    filename: &str,
    bytes: &[u8],
    destination: &Path,
) -> Result<(), OrbitError> {
    std::fs::create_dir_all(mods_dir)?;
    let temporary = mods_dir.join(format!(".{filename}.importing"));
    let mut output = std::fs::File::create(&temporary)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    if destination.exists() {
        std::fs::remove_file(destination)?;
    }
    std::fs::rename(temporary, destination)?;
    Ok(())
}

fn build_index(
    manifest: &crate::manifest::OrbitManifest,
    sources: &[(&crate::lockfile::PackageEntry, PathBuf)],
) -> serde_json::Value {
    let files: Vec<_> = sources
        .iter()
        .filter_map(|(entry, source)| {
            let download_url = download_url(entry)?;
            let (client, server) = environment(manifest, &entry.mod_id);
            Some(json!({
                "path": format!("mods/{}", entry.filename),
                "hashes": {
                    "sha1": entry.sha1,
                    "sha512": entry.sha512,
                },
                "env": { "client": client, "server": server },
                "downloads": [download_url],
                "fileSize": std::fs::metadata(source).map(|metadata| metadata.len()).unwrap_or(0),
            }))
        })
        .collect();
    let loader_key = match manifest.project.modloader.as_str() {
        "fabric" => "fabric-loader",
        "quilt" => "quilt-loader",
        other => other,
    };
    let mut dependencies = serde_json::Map::new();
    dependencies.insert("minecraft".to_string(), json!(manifest.project.mc_version));
    dependencies.insert(
        loader_key.to_string(),
        json!(manifest.project.modloader_version),
    );
    json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": manifest.project.version.as_deref().unwrap_or("1.0.0"),
        "name": manifest.project.name,
        "summary": manifest.project.description.as_deref().unwrap_or(""),
        "files": files,
        "dependencies": dependencies,
    })
}

fn download_url(entry: &crate::lockfile::PackageEntry) -> Option<&str> {
    if entry.sha1.is_empty() || entry.sha512.is_empty() {
        return None;
    }
    let download_url = entry
        .artifact_sources
        .iter()
        .find_map(|source| match source {
            crate::lockfile::ArtifactSource::Modrinth { download_url, .. }
            | crate::lockfile::ArtifactSource::Curseforge { download_url, .. } => {
                Some(download_url.as_str())
            }
            crate::lockfile::ArtifactSource::File { .. } => None,
        })?;
    let url = url::Url::parse(download_url).ok()?;
    (url.scheme() == "https").then_some(download_url)
}

fn environment(
    manifest: &crate::manifest::OrbitManifest,
    mod_id: &str,
) -> (&'static str, &'static str) {
    let Some(requirement) = manifest.dependencies.get(mod_id) else {
        return ("required", "required");
    };
    let supported = if requirement.optional() {
        "optional"
    } else {
        "required"
    };
    match requirement.env().unwrap_or("both") {
        "client" => (supported, "unsupported"),
        "server" => ("unsupported", supported),
        _ => (supported, supported),
    }
}
