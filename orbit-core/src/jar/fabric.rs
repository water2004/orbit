//! Fabric JAR reader — 查找并解析 fabric.mod.json。

use super::JarModMetadata;
use crate::error::OrbitError;
use std::io::{Read, Seek};
use zip::ZipArchive;

/// 在 ZIP archive 中查找 fabric.mod.json（先根路径，再一层子目录），
/// 解析后返回 `JarModMetadata`。未找到时返回 `Ok(None)`。
pub fn try_read<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<JarModMetadata>, OrbitError> {
    let Some((_, content)) = super::read_metadata_entry(archive, &["fabric.mod.json"])? else {
        return Ok(None);
    };

    let parser = crate::metadata::fabric::FabricParser;
    let meta = crate::metadata::MetadataParser::parse(&parser, &content)?;

    Ok(Some(JarModMetadata {
        mod_id: meta.id,
        name: meta.name,
        version: meta.version,
        dependencies: meta
            .dependencies
            .into_iter()
            .map(|(k, v)| (k, v, true))
            .collect(),
        embedded_jars: meta.embedded_jars,
        implanted_mods: vec![],
    }))
}
