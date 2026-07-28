use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::LauncherError;
use crate::lockfile::LockedArtifact;

const MAX_NATIVE_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TOTAL_NATIVE_BYTES: u64 = 1024 * 1024 * 1024;

/// Rebuilds the per-instance native directory from the exact native archives
/// locked for this runtime. This deliberately runs during launch preparation,
/// matching the launcher model used by HMCL and preventing extracted binaries
/// from becoming installation artifacts.
pub fn prepare_native_directory(
    artifact_root: &Path,
    destination: &Path,
    artifacts: &[LockedArtifact],
) -> Result<usize, LauncherError> {
    let temporary = destination.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    if temporary.exists() {
        return Err(LauncherError::Transaction(format!(
            "native staging directory '{}' already exists",
            temporary.display()
        )));
    }
    std::fs::create_dir_all(&temporary)?;
    let result = extract_native_archives(artifact_root, &temporary, artifacts);
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
    if destination.exists() {
        std::fs::remove_dir_all(destination)?;
    }
    std::fs::rename(&temporary, destination)?;
    result
}

fn extract_native_archives(
    artifact_root: &Path,
    destination: &Path,
    artifacts: &[LockedArtifact],
) -> Result<usize, LauncherError> {
    let mut generated = BTreeMap::<PathBuf, String>::new();
    let mut total_bytes = 0_u64;
    for artifact in artifacts {
        let Some(extraction) = &artifact.native_extraction else {
            continue;
        };
        let file = std::fs::File::open(artifact_root.join(&artifact.path))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|error| {
            LauncherError::ArtifactIntegrity(format!(
                "native library '{}' is not a readable ZIP archive: {error}",
                artifact.logical_name
            ))
        })?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|error| {
                LauncherError::ArtifactIntegrity(format!(
                    "native library '{}' contains an unreadable entry: {error}",
                    artifact.logical_name
                ))
            })?;
            let raw_name = entry.name().to_string();
            if raw_name.contains('\\') {
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "native library '{}' contains a non-portable path",
                    artifact.logical_name
                )));
            }
            if entry.is_dir()
                || extraction
                    .excludes
                    .iter()
                    .any(|exclude| raw_name.starts_with(exclude))
                || raw_name.ends_with(".sha1")
                || raw_name.ends_with(".git")
            {
                continue;
            }
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "native library '{}' contains a symbolic link",
                    artifact.logical_name
                )));
            }
            let enclosed = entry.enclosed_name().ok_or_else(|| {
                LauncherError::ArtifactIntegrity(format!(
                    "native library '{}' contains an unsafe path",
                    artifact.logical_name
                ))
            })?;
            let portable = portable_archive_path(&enclosed)?;
            if entry.size() > MAX_NATIVE_ENTRY_BYTES {
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "native entry '{}' exceeds {MAX_NATIVE_ENTRY_BYTES} bytes",
                    portable.display()
                )));
            }
            total_bytes = total_bytes.checked_add(entry.size()).ok_or_else(|| {
                LauncherError::ArtifactIntegrity(
                    "native extraction size exceeds the supported range".to_string(),
                )
            })?;
            if total_bytes > MAX_TOTAL_NATIVE_BYTES {
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "native extraction exceeds {MAX_TOTAL_NATIVE_BYTES} bytes"
                )));
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry
                .by_ref()
                .take(MAX_NATIVE_ENTRY_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 != entry.size() {
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "native entry '{}' has inconsistent size metadata",
                    portable.display()
                )));
            }
            let digest = hex::encode(Sha256::digest(&bytes));
            if let Some(existing) = generated.get(&portable) {
                if existing != &digest {
                    return Err(LauncherError::ArtifactIntegrity(format!(
                        "native libraries produce conflicting file '{}'",
                        portable.display()
                    )));
                }
                continue;
            }
            let target = destination.join(&portable);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, bytes)?;
            generated.insert(portable, digest);
        }
    }
    Ok(generated.len())
}

fn portable_archive_path(path: &Path) -> Result<PathBuf, LauncherError> {
    let mut result = PathBuf::new();
    for component in path.components() {
        let std::path::Component::Normal(value) = component else {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "native path '{}' is not portable",
                path.display()
            )));
        };
        let value = value.to_str().ok_or_else(|| {
            LauncherError::ArtifactIntegrity(format!(
                "native path '{}' is not valid UTF-8",
                path.display()
            ))
        })?;
        if value.chars().any(char::is_control) {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "native path '{}' contains control characters",
                path.display()
            )));
        }
        result.push(value);
    }
    if result.as_os_str().is_empty() {
        return Err(LauncherError::ArtifactIntegrity(
            "native archive contains an empty path".to_string(),
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{ArtifactOwner, LockedArtifactSource, LockedNativeExtraction};
    use std::io::Write;

    #[test]
    fn native_archives_are_expanded_only_into_the_launch_directory() {
        let directory = tempfile::tempdir().unwrap();
        let archive_path = directory.path().join("native.jar");
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("native.dll", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"native").unwrap();
        archive.finish().unwrap();
        let bytes = std::fs::read(&archive_path).unwrap();
        let artifact = LockedArtifact {
            logical_name: "native".to_string(),
            owner: ArtifactOwner::Minecraft,
            source: LockedArtifactSource::InstallerOutput {
                installer_sha256: "0".repeat(64),
            },
            sha256: hex::encode(Sha256::digest(&bytes)),
            size: bytes.len() as u64,
            path: "native.jar".to_string(),
            native_extraction: Some(LockedNativeExtraction { excludes: vec![] }),
        };
        let destination = directory.path().join("natives");
        assert_eq!(
            prepare_native_directory(directory.path(), &destination, &[artifact]).unwrap(),
            1
        );
        assert_eq!(
            std::fs::read(destination.join("native.dll")).unwrap(),
            b"native"
        );
    }
}
