use std::io::Write;
use std::path::Path;

use crate::error::LauncherError;

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), LauncherError> {
    let parent = path.parent().ok_or_else(|| {
        LauncherError::InvalidConfig(format!("path '{}' has no parent directory", path.display()))
    })?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_parent(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), LauncherError> {
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), LauncherError> {
    Ok(())
}
