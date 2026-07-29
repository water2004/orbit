//! Durable storage for local package remotes that originate inside `mods/`.
//!
//! Files in `mods/` are transaction output and may be removed when another
//! package version is selected. A local remote must therefore never point at
//! such a mutable output file.

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::manifest::PackageRemote;

const SOURCE_DIRECTORY: &str = ".orbit/sources";

pub(crate) fn managed_remote(sha512: &str) -> PackageRemote {
    PackageRemote::File {
        path: format!("{SOURCE_DIRECTORY}/{sha512}.jar"),
    }
}

pub(crate) fn preserve_local_remote(
    instance_dir: &Path,
    source: &Path,
    sha512: &str,
) -> Result<PackageRemote, OrbitError> {
    if sha512.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "cannot preserve a local package source without a SHA-512 content identity"
        )));
    }
    let PackageRemote::File { path: relative } = managed_remote(sha512) else {
        unreachable!("managed source is always a file remote")
    };
    let destination = instance_dir.join(Path::new(&relative));
    if destination.exists() {
        verify_source(&destination, sha512)?;
        return Ok(PackageRemote::File { path: relative });
    }

    let parent = destination.parent().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "managed local source has no parent directory"
        ))
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = temporary_path(&destination);
    std::fs::copy(source, &temporary)?;
    if let Err(error) = verify_source(&temporary, sha512) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    match std::fs::rename(&temporary, &destination) {
        Ok(()) => {}
        Err(_) if destination.exists() => {
            let _ = std::fs::remove_file(&temporary);
            verify_source(&destination, sha512)?;
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(OrbitError::Io(error));
        }
    }
    Ok(PackageRemote::File { path: relative })
}

/// Remove content-addressed local sources that are no longer referenced by
/// either the manifest or lock. The store is authoritative for genuinely
/// local packages, so pruning is reference based rather than capacity based.
pub(crate) fn prune_unreferenced(
    instance_dir: &Path,
    manifest: &crate::manifest::OrbitManifest,
    lockfile: &crate::lockfile::OrbitLockfile,
) -> Result<usize, OrbitError> {
    let source_directory = instance_dir.join(SOURCE_DIRECTORY);
    if !source_directory.is_dir() {
        return Ok(0);
    }

    let mut referenced = std::collections::HashSet::new();
    for remote in manifest
        .packages
        .values()
        .flat_map(|dependency| dependency.remotes.iter())
        .chain(
            lockfile
                .packages
                .iter()
                .flat_map(|package| package.remotes.iter()),
        )
    {
        if let PackageRemote::File { path } = remote
            && let Some(filename) = managed_filename(path)
        {
            referenced.insert(filename);
        }
    }
    for source in lockfile
        .packages
        .iter()
        .flat_map(|package| package.artifact_sources.iter())
    {
        if let crate::lockfile::ArtifactSource::File { path } = source
            && let Some(filename) = managed_filename(path)
        {
            referenced.insert(filename);
        }
    }

    let mut removed = 0;
    for entry in std::fs::read_dir(&source_directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        if valid_managed_filename(&filename) && !referenced.contains(&filename) {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    if std::fs::read_dir(&source_directory)?.next().is_none() {
        std::fs::remove_dir(&source_directory)?;
    }
    Ok(removed)
}

fn managed_filename(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    normalized
        .strip_prefix(&format!("{SOURCE_DIRECTORY}/"))
        .filter(|filename| valid_managed_filename(filename))
        .map(str::to_string)
}

fn valid_managed_filename(filename: &str) -> bool {
    let Some(hash) = filename.strip_suffix(".jar") else {
        return false;
    };
    hash.len() == 128 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn preserve_if_instance_output(
    instance_dir: &Path,
    source: &Path,
    sha512: &str,
) -> Result<PackageRemote, OrbitError> {
    let canonical_source = std::fs::canonicalize(source)?;
    let canonical_instance =
        std::fs::canonicalize(instance_dir).unwrap_or_else(|_| instance_dir.to_path_buf());
    let source_store = canonical_instance.join(SOURCE_DIRECTORY);
    if source_store.exists()
        && canonical_source.starts_with(&source_store)
        && let Ok(relative) = canonical_source.strip_prefix(&canonical_instance)
    {
        return Ok(PackageRemote::File {
            path: relative.to_string_lossy().replace('\\', "/"),
        });
    }
    let mods = canonical_instance.join("mods");
    let canonical_mods = if mods.exists() {
        std::fs::canonicalize(&mods)?
    } else {
        mods
    };
    if canonical_source.starts_with(canonical_mods) {
        preserve_local_remote(instance_dir, &canonical_source, sha512)
    } else {
        Ok(PackageRemote::File {
            path: canonical_source.to_string_lossy().into_owned(),
        })
    }
}

pub(crate) fn managed_artifact_source(remote: &PackageRemote) -> crate::lockfile::ArtifactSource {
    match remote {
        PackageRemote::File { path } => {
            crate::lockfile::ArtifactSource::File { path: path.clone() }
        }
        _ => unreachable!("managed source is always a file remote"),
    }
}

fn verify_source(path: &Path, sha512: &str) -> Result<(), OrbitError> {
    let actual = crate::jar::compute_sha512(path)?;
    if actual.eq_ignore_ascii_case(sha512) {
        Ok(())
    } else {
        Err(OrbitError::Other(anyhow::anyhow!(
            "managed local source content does not match the inspected JAR"
        )))
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    destination.with_extension(format!("tmp-{}-{nonce}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mods_output_is_copied_to_a_managed_source_without_exposing_its_hash() {
        let directory = tempfile::tempdir().unwrap();
        let mods = directory.path().join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let source = mods.join("example.jar");
        let bytes = b"local package content";
        std::fs::write(&source, bytes).unwrap();
        let sha512 = crate::jar::sha512_digest(bytes);

        let remote = preserve_if_instance_output(directory.path(), &source, &sha512).unwrap();

        let PackageRemote::File { path } = &remote else {
            panic!("managed local source must remain a file remote");
        };
        assert!(path.starts_with(".orbit/sources/"));
        assert!(directory.path().join(path).is_file());
        assert_eq!(remote.display_locator(), "file:managed local source");
        assert!(!remote.display_locator().contains(&sha512));
    }

    #[test]
    fn reference_pruning_never_treats_the_local_source_store_as_an_lru_cache() {
        let directory = tempfile::tempdir().unwrap();
        let kept_hash = "a".repeat(128);
        let removed_hash = "b".repeat(128);
        let source_directory = directory.path().join(SOURCE_DIRECTORY);
        std::fs::create_dir_all(&source_directory).unwrap();
        std::fs::write(source_directory.join(format!("{kept_hash}.jar")), b"kept").unwrap();
        std::fs::write(
            source_directory.join(format!("{removed_hash}.jar")),
            b"removed",
        )
        .unwrap();
        std::fs::write(source_directory.join("notes.txt"), b"untouched").unwrap();
        let manifest: crate::manifest::OrbitManifest = toml::from_str(&format!(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "fabric"
modloader_version = "1"
[platform]
minecraft_jar = {{ path = "minecraft.jar", sha256 = "test" }}
loader_jar = {{ path = "loader.jar", sha256 = "test" }}
runtime_jars = []
physical_environment = "client"
[packages]
alpha = {{ version = "*", remotes = [{{ type = "file", path = ".orbit/sources/{kept_hash}.jar" }}] }}
"#
        ))
        .unwrap();
        let lockfile = crate::lockfile::OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: Vec::new(),
        };

        assert_eq!(
            prune_unreferenced(directory.path(), &manifest, &lockfile).unwrap(),
            1
        );
        assert!(source_directory.join(format!("{kept_hash}.jar")).is_file());
        assert!(
            !source_directory
                .join(format!("{removed_hash}.jar"))
                .exists()
        );
        assert!(source_directory.join("notes.txt").is_file());
    }
}
