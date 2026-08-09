//! Atomic package-artifact removal and rollback.

use std::path::Path;

use crate::error::OrbitError;

pub(super) fn commit_package_removal(
    instance_dir: &Path,
    jar_path: &Path,
    manifest_document: &[u8],
    lock_document: &[u8],
) -> Result<(), OrbitError> {
    let manifest_path = instance_dir.join("orbit.toml");
    let lock_path = instance_dir.join("orbit.lock");
    let original_manifest = std::fs::read(&manifest_path)?;
    let original_lock = std::fs::read(&lock_path)?;
    let mods_dir = jar_path.parent().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "package artifact has no parent directory: {}",
            jar_path.display()
        ))
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".orbit-remove-")
        .tempdir_in(mods_dir)?;
    std::fs::write(
        staging.path().join("orbit.toml.original"),
        &original_manifest,
    )?;
    std::fs::write(staging.path().join("orbit.lock.original"), &original_lock)?;
    let staged_jar = staging.path().join(jar_path.file_name().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "package artifact has no filename: {}",
            jar_path.display()
        ))
    })?);
    std::fs::rename(jar_path, &staged_jar)?;

    let mut manifest_committed = false;
    let mut lock_committed = false;
    let commit_result = (|| {
        crate::atomic_io::write_atomic(&manifest_path, manifest_document)?;
        manifest_committed = true;
        crate::atomic_io::write_atomic(&lock_path, lock_document)?;
        lock_committed = true;
        match std::fs::remove_file(&staged_jar) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(OrbitError::Io(error)),
        }
    })();
    let Err(commit_error) = commit_result else {
        return Ok(());
    };

    let mut rollback_failures = Vec::new();
    if lock_committed && let Err(error) = crate::atomic_io::write_atomic(&lock_path, &original_lock)
    {
        rollback_failures.push(format!("orbit.lock: {error}"));
    }
    if manifest_committed
        && let Err(error) = crate::atomic_io::write_atomic(&manifest_path, &original_manifest)
    {
        rollback_failures.push(format!("orbit.toml: {error}"));
    }
    match std::fs::symlink_metadata(&staged_jar) {
        Ok(_) => match std::fs::symlink_metadata(jar_path) {
            Ok(_) => rollback_failures.push(format!(
                "package path was occupied during rollback: {}",
                jar_path.display()
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(error) = std::fs::rename(&staged_jar, jar_path) {
                    rollback_failures.push(format!("package artifact: {error}"));
                }
            }
            Err(error) => rollback_failures.push(format!("package path metadata: {error}")),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => rollback_failures
            .push("staged package artifact disappeared before rollback".to_string()),
        Err(error) => rollback_failures.push(format!("package artifact metadata: {error}")),
    }

    if rollback_failures.is_empty() {
        return Err(commit_error);
    }
    let retained_staging = staging.keep();
    Err(OrbitError::Other(anyhow::anyhow!(
        "package removal failed: {commit_error}; restoring the original state also failed ({}); staged recovery data was retained at '{}'",
        rollback_failures.join("; "),
        retained_staging.display()
    )))
}
