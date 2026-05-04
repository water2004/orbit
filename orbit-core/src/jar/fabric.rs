//! Fabric JAR reader — 查找并解析 fabric.mod.json。

use std::io::{Read, Seek};
use zip::ZipArchive;
use crate::error::OrbitError;
use super::JarModMetadata;

/// 在 ZIP archive 中查找 fabric.mod.json（先根路径，再一层子目录），
/// 解析后返回 `JarModMetadata`。未找到时返回 `Ok(None)`。
pub fn try_read<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Option<JarModMetadata>, OrbitError> {
    let target = "fabric.mod.json";

    let content = if let Ok(mut entry) = archive.by_name(target) {
        let mut s = String::new();
        entry.read_to_string(&mut s).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot read {target}: {e}"))
        })?;
        Some(s)
    } else {
        let idx = (0..archive.len()).find(|&i| {
            archive.by_index(i)
                .map(|e| {
                    let name = e.name();
                    name.ends_with(target)
                        && (name == target || name.matches('/').count() == 1)
                })
                .unwrap_or(false)
        });
        match idx {
            Some(i) => {
                let mut entry = archive.by_index(i).map_err(|e| {
                    OrbitError::Other(anyhow::anyhow!("cannot read ZIP entry: {e}"))
                })?;
                let mut s = String::new();
                entry.read_to_string(&mut s).map_err(|e| {
                    OrbitError::Other(anyhow::anyhow!("cannot read {target}: {e}"))
                })?;
                Some(s)
            }
            None => None,
        }
    };

    let Some(content) = content else { return Ok(None) };

    let parser = crate::metadata::fabric::FabricParser;
    let meta = crate::metadata::MetadataParser::parse(&parser, &content)?;

    Ok(Some(JarModMetadata {
        mod_id: meta.id,
        name: meta.name,
        version: meta.version,
        dependencies: meta.dependencies.into_iter().map(|(k, v)| (k, v, true)).collect(),
        embedded_jars: meta.embedded_jars,
    }))
}
