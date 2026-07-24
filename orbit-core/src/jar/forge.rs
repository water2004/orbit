//! Forge and NeoForge JAR readers.

use std::io::{Read, Seek};

use zip::ZipArchive;

use super::JarModMetadata;
use crate::error::OrbitError;
use crate::metadata::ModLoader;

pub fn try_read<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    loader: ModLoader,
) -> Result<Option<JarModMetadata>, OrbitError> {
    let targets: &[&str] = match loader {
        ModLoader::Forge => &["META-INF/mods.toml"],
        // NeoForge 20.4 and earlier used mods.toml; prefer the modern name.
        ModLoader::NeoForge => &["META-INF/neoforge.mods.toml", "META-INF/mods.toml"],
        _ => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "Forge JAR reader received incompatible loader '{}'",
                loader.as_str()
            )));
        }
    };
    let Some((source_name, content)) = super::read_metadata_entry(archive, targets)? else {
        return Ok(None);
    };
    let mut parsed = crate::metadata::forge::parse_for_loader(&content, loader, &source_name)?;
    if parsed.metadata.version == "${file.jarVersion}"
        && let Some(version) = implementation_version(archive)?
    {
        parsed.metadata.version = version;
    }
    let embedded_jars = read_jarjar_paths(archive)?;
    let metadata = parsed.metadata;

    Ok(Some(JarModMetadata {
        mod_id: metadata.id,
        name: metadata.name,
        version: metadata.version,
        dependencies: parsed.dependencies,
        embedded_jars,
        implanted_mods: Vec::new(),
    }))
}

fn implementation_version<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<String>, OrbitError> {
    let Some((_, manifest)) = super::read_metadata_entry(archive, &["META-INF/MANIFEST.MF"])?
    else {
        return Ok(None);
    };
    Ok(manifest.lines().find_map(|line| {
        line.split_once(':').and_then(|(key, value)| {
            key.trim()
                .eq_ignore_ascii_case("Implementation-Version")
                .then(|| value.trim().to_string())
        })
    }))
}

fn read_jarjar_paths<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Vec<String>, OrbitError> {
    let Some((_, content)) =
        super::read_metadata_entry(archive, &["META-INF/jarjar/metadata.json"])?
    else {
        return Ok(Vec::new());
    };
    let value: serde_json::Value = serde_json::from_str(&content).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "invalid META-INF/jarjar/metadata.json: {error}"
        ))
    })?;
    Ok(value
        .get("jars")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|jar| jar.get("path").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect())
}
