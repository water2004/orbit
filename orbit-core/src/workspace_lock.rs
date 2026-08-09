//! Cross-process serialization for instance mutations.
//!
//! The GUI is intentionally free to run independent read-only commands in
//! parallel.  Mutations of one instance, however, must cover the complete
//! read/solve/materialize/commit operation; locking only the final writes
//! would still permit lost updates.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::OrbitError;

#[derive(Debug)]
pub struct WorkspaceMutationLocks {
    files: Vec<File>,
}

impl WorkspaceMutationLocks {
    pub fn acquire(directories: impl IntoIterator<Item = PathBuf>) -> Result<Self, OrbitError> {
        let mut directories = directories
            .into_iter()
            .map(|directory| canonical_directory(&directory))
            .collect::<Result<Vec<_>, _>>()?;
        directories.sort();
        directories.dedup();

        let mut files = Vec::with_capacity(directories.len());
        for directory in directories {
            let control = directory.join(".orbit");
            match std::fs::symlink_metadata(&control) {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "workspace control path is not a real directory: {}",
                        control.display()
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&control)?;
                }
                Err(error) => return Err(error.into()),
            }

            let path = control.join("mutation.lock");
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_file() => {}
                Ok(_) => {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "workspace mutation lock is not a regular file: {}",
                        path.display()
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?;
            if let Err(error) = fs2::FileExt::try_lock_exclusive(&file) {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "another Orbit mutation is already running for '{}': {error}",
                    directory.display()
                )));
            }
            files.push(file);
        }
        Ok(Self { files })
    }
}

impl Drop for WorkspaceMutationLocks {
    fn drop(&mut self) {
        for file in self.files.iter().rev() {
            let _ = fs2::FileExt::unlock(file);
        }
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf, OrbitError> {
    let path = dunce::canonicalize(path).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "cannot lock workspace '{}': {error}",
            path.display()
        ))
    })?;
    if !path.is_dir() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "workspace mutation target is not a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_process_guard_cannot_mutate_the_same_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let first = WorkspaceMutationLocks::acquire([directory.path().to_path_buf()]).unwrap();

        let error = WorkspaceMutationLocks::acquire([directory.path().to_path_buf()]).unwrap_err();

        assert!(error.to_string().contains("already running"));
        drop(first);
        WorkspaceMutationLocks::acquire([directory.path().to_path_buf()]).unwrap();
    }

    #[test]
    fn repeated_targets_are_locked_only_once() {
        let directory = tempfile::tempdir().unwrap();
        WorkspaceMutationLocks::acquire([
            directory.path().to_path_buf(),
            directory.path().to_path_buf(),
        ])
        .unwrap();
    }
}
