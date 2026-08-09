//! Shared crash-safe file replacement primitives.

use std::io::Write;
use std::path::Path;

use crate::error::OrbitError;

pub(crate) fn write_atomic(path: &Path, content: &[u8]) -> Result<(), OrbitError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content)?;
    temporary.flush()?;
    temporary
        .persist(path)
        .map_err(|error| OrbitError::Io(error.error))?;
    Ok(())
}
