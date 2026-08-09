//! Rollback-capable package-file updates.
//!
//! A solver result changes three pieces of state together: JARs in `mods/`,
//! `orbit.toml`, and `orbit.lock`.  The workspace documents are persisted by
//! the caller; this guard keeps the original JARs recoverable until that
//! persistence succeeds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::OrbitError;

struct StagedArtifact {
    destination: PathBuf,
    backup: Option<PathBuf>,
}

pub(super) struct PackageFileTransaction {
    mods_dir: PathBuf,
    mods_existed: bool,
    staging: Option<tempfile::TempDir>,
    artifacts: Vec<StagedArtifact>,
    finished: bool,
}

impl PackageFileTransaction {
    pub(super) fn validate_filenames<'a>(
        filenames: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), OrbitError> {
        Self::collect_filenames(filenames).map(|_| ())
    }

    pub(super) fn begin<'a>(
        mods_dir: &Path,
        filenames: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, OrbitError> {
        let instance_dir = mods_dir.parent().ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "mods directory has no instance parent: {}",
                mods_dir.display()
            ))
        })?;
        let unique = Self::collect_filenames(filenames)?;
        match std::fs::symlink_metadata(mods_dir) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "mods path is not a real directory: {}",
                    mods_dir.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let staging = tempfile::Builder::new()
            .prefix(".orbit-package-transaction-")
            .tempdir_in(instance_dir)?;
        let mut transaction = Self {
            mods_dir: mods_dir.to_path_buf(),
            mods_existed: mods_dir.is_dir(),
            staging: Some(staging),
            artifacts: Vec::with_capacity(unique.len()),
            finished: false,
        };
        for (index, filename) in unique.values().enumerate() {
            let destination = mods_dir.join(filename);
            let backup = match std::fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_file() => {
                    let backup = transaction
                        .staging
                        .as_ref()
                        .expect("active transaction owns staging")
                        .path()
                        .join(format!("artifact-{index}"));
                    if let Err(error) = std::fs::rename(&destination, &backup) {
                        let cause = OrbitError::Io(error);
                        return Err(transaction.rollback(cause));
                    }
                    Some(backup)
                }
                Ok(_) => {
                    let cause = OrbitError::Other(anyhow::anyhow!(
                        "package destination is not a regular file: {}",
                        destination.display()
                    ));
                    return Err(transaction.rollback(cause));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(transaction.rollback(error.into())),
            };
            transaction.artifacts.push(StagedArtifact {
                destination,
                backup,
            });
        }
        Ok(transaction)
    }

    fn collect_filenames<'a>(
        filenames: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, String>, OrbitError> {
        let mut unique = BTreeMap::<String, String>::new();
        for filename in filenames {
            super::safe_artifact_filename(filename)?;
            let identity = orbit_bundle_format::portable_path_identity(filename);
            match unique.get(&identity) {
                Some(existing) if existing != filename => {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "package transaction contains portable filename collision '{existing}' and '{filename}'"
                    )));
                }
                Some(_) => {}
                None => {
                    unique.insert(identity, filename.to_string());
                }
            }
        }
        Ok(unique)
    }

    /// Finish the update and delete staged originals.
    ///
    /// A cleanup failure does not make the committed workspace inconsistent,
    /// so it is returned as a warning rather than an operation failure.
    pub(super) fn commit(mut self) -> Option<String> {
        self.finished = true;
        let staging = self.staging.take()?;
        staging.close().err().map(|error| {
            format!(
                "package update committed, but its transaction staging could not be removed: {error}"
            )
        })
    }

    pub(super) fn rollback(mut self, cause: OrbitError) -> OrbitError {
        let failures = self.restore();
        self.finished = true;
        if failures.is_empty() {
            return cause;
        }
        let retained = self
            .staging
            .take()
            .map(tempfile::TempDir::keep)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unavailable>".to_string());
        OrbitError::Other(anyhow::anyhow!(
            "package update failed: {cause}; restoring the original package files also failed ({}); recovery data was retained at '{retained}'",
            failures.join("; ")
        ))
    }

    fn restore(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        for artifact in self.artifacts.iter().rev() {
            match std::fs::symlink_metadata(&artifact.destination) {
                Ok(metadata) if metadata.file_type().is_file() => {
                    if let Err(error) = std::fs::remove_file(&artifact.destination) {
                        failures.push(format!(
                            "remove replacement '{}': {error}",
                            artifact.destination.display()
                        ));
                        continue;
                    }
                }
                Ok(_) => {
                    failures.push(format!(
                        "replacement path is no longer a regular file: {}",
                        artifact.destination.display()
                    ));
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    failures.push(format!(
                        "inspect replacement '{}': {error}",
                        artifact.destination.display()
                    ));
                    continue;
                }
            }
            if let Some(backup) = &artifact.backup
                && let Err(error) = std::fs::rename(backup, &artifact.destination)
            {
                failures.push(format!(
                    "restore package '{}': {error}",
                    artifact.destination.display()
                ));
            }
        }
        if !self.mods_existed && self.mods_dir.is_dir() {
            match std::fs::remove_dir(&self.mods_dir) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!(
                    "remove newly created mods directory '{}': {error}",
                    self.mods_dir.display()
                )),
            }
        }
        failures
    }
}

impl Drop for PackageFileTransaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_restores_replaced_and_removed_artifacts() {
        let instance = tempfile::tempdir().unwrap();
        let mods = instance.path().join("mods");
        std::fs::create_dir(&mods).unwrap();
        std::fs::write(mods.join("replace.jar"), b"old").unwrap();
        std::fs::write(mods.join("remove.jar"), b"remove").unwrap();

        let transaction =
            PackageFileTransaction::begin(&mods, ["replace.jar", "remove.jar", "new.jar"]).unwrap();
        std::fs::write(mods.join("replace.jar"), b"new").unwrap();
        std::fs::write(mods.join("new.jar"), b"new").unwrap();

        let error = transaction.rollback(OrbitError::Other(anyhow::anyhow!("test failure")));

        assert!(error.to_string().contains("test failure"));
        assert_eq!(std::fs::read(mods.join("replace.jar")).unwrap(), b"old");
        assert_eq!(std::fs::read(mods.join("remove.jar")).unwrap(), b"remove");
        assert!(!mods.join("new.jar").exists());
    }

    #[test]
    fn commit_keeps_replacements_and_removals() {
        let instance = tempfile::tempdir().unwrap();
        let mods = instance.path().join("mods");
        std::fs::create_dir(&mods).unwrap();
        std::fs::write(mods.join("replace.jar"), b"old").unwrap();
        std::fs::write(mods.join("remove.jar"), b"remove").unwrap();

        let transaction =
            PackageFileTransaction::begin(&mods, ["replace.jar", "remove.jar"]).unwrap();
        std::fs::write(mods.join("replace.jar"), b"new").unwrap();

        assert!(transaction.commit().is_none());
        assert_eq!(std::fs::read(mods.join("replace.jar")).unwrap(), b"new");
        assert!(!mods.join("remove.jar").exists());
    }

    #[test]
    fn portable_filename_collisions_are_rejected_without_mutation() {
        let instance = tempfile::tempdir().unwrap();
        let mods = instance.path().join("mods");
        std::fs::create_dir(&mods).unwrap();
        std::fs::write(mods.join("Example.jar"), b"old").unwrap();

        let result = PackageFileTransaction::begin(&mods, ["Example.jar", "example.jar"]);

        assert!(result.is_err());
        assert_eq!(std::fs::read(mods.join("Example.jar")).unwrap(), b"old");
    }
}
