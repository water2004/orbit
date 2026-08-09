//! Shared crash-safe file replacement primitives.

use std::io::{Read, Write};
use std::path::Path;

use crate::error::OrbitError;

pub(crate) fn write_atomic(path: &Path, content: &[u8]) -> Result<(), OrbitError> {
    write_atomic_with(path, |temporary| {
        temporary.write_all(content)?;
        Ok(())
    })
}

pub(crate) fn copy_atomic_verified_sha512(
    source: &Path,
    destination: &Path,
    expected_sha512: &str,
) -> Result<(), OrbitError> {
    use sha2::{Digest as _, Sha512};

    let mut source = std::fs::File::open(source)?;
    let mut digest = Sha512::new();
    write_atomic_with(destination, |temporary| {
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
            temporary.write_all(&buffer[..read])?;
        }
        let actual = hex::encode(digest.finalize());
        if !actual.eq_ignore_ascii_case(expected_sha512) {
            return Err(OrbitError::ChecksumMismatch {
                name: "cached JAR".to_string(),
                expected: expected_sha512.to_string(),
                actual,
            });
        }
        Ok(())
    })
}

fn write_atomic_with(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), OrbitError>,
) -> Result<(), OrbitError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write(temporary.as_file_mut())?;
    temporary.as_file_mut().flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| OrbitError::Io(error.error))?;
    sync_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), OrbitError> {
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), OrbitError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha512};

    #[test]
    fn verified_copy_replaces_only_after_hash_validation() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.jar");
        let destination = directory.path().join("destination.jar");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&destination, b"old").unwrap();

        assert!(copy_atomic_verified_sha512(&source, &destination, &"0".repeat(128)).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");

        let expected = hex::encode(Sha512::digest(b"new"));
        copy_atomic_verified_sha512(&source, &destination, &expected).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    }
}
