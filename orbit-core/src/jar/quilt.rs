//! Quilt JAR reader.

use std::io::{Read, Seek};

use zip::ZipArchive;

use super::JarModMetadata;
use crate::error::OrbitError;

pub fn try_read<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<JarModMetadata>, OrbitError> {
    let Some((_, content)) = super::read_metadata_entry(archive, &["quilt.mod.json"])? else {
        // Quilt Loader can load Fabric metadata, so retain that compatibility.
        return super::fabric::try_read(archive);
    };
    let metadata = crate::metadata::quilt::parse_quilt(&content)?;
    Ok(Some(super::from_mod_file(metadata)?))
}
