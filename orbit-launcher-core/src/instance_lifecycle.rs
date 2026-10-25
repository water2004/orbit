//! Shared by actual game lifetimes, install transactions and complete deletion.
use crate::LauncherError;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub(crate) struct InstanceLease {
    _file: File,
}

/// Reject redirects before traversing user data. Windows junctions and cloud
/// reparse points are not all reported as ordinary symbolic links.
pub(crate) fn is_redirected(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
impl InstanceLease {
    pub(crate) fn acquire(root: &Path, exclusive: bool) -> Result<Self, LauncherError> {
        // A Windows directory cannot be renamed while a lock handle inside it
        // is open. Keep leases beside the instance, not inside the deletion unit.
        let canonical = dunce::canonicalize(root)?;
        let state = canonical
            .parent()
            .ok_or_else(|| LauncherError::Transaction("instance has no parent".into()))?
            .join(".orbit-launcher-locks");
        std::fs::create_dir_all(&state)?;
        let key = hex::encode(Sha256::digest(canonical.as_os_str().as_encoded_bytes()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(state.join(format!("{key}.lock")))?;
        let result = if exclusive {
            FileExt::try_lock_exclusive(&file)
        } else {
            FileExt::try_lock_shared(&file)
        };
        result.map_err(|e| {
            LauncherError::Transaction(format!(
                "instance '{}' is running or being modified: {e}",
                root.display()
            ))
        })?;
        Ok(Self { _file: file })
    }
}
