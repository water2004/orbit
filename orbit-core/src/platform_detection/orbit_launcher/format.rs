//! Orbit's read-only view of Launcher storage. Only installed runtime facts
//! are projected; no settings migration or write-back is performed here.
use super::invalid;
use crate::error::OrbitError;
use serde::Deserialize;
use std::path::Path;

pub(super) use orbit_launcher_format::{
    LOCK_FILE, MANIFEST_FILE, RuntimeArtifact, RuntimeEntrypoint, RuntimeLock, RuntimeManifest,
};
use orbit_launcher_format::{
    READABLE_LOCK_SCHEMAS as LOCK_SCHEMAS, READABLE_MANIFEST_SCHEMAS as MANIFEST_SCHEMAS,
};

pub(super) fn read(root: &Path) -> Result<Option<(RuntimeManifest, RuntimeLock)>, OrbitError> {
    let manifest_path = root.join(MANIFEST_FILE);
    let lock_path = root.join(LOCK_FILE);
    let manifest = std::fs::read_to_string(&manifest_path);
    let lock = std::fs::read_to_string(&lock_path);
    let missing = |result: &Result<String, std::io::Error>| matches!(result, Err(error) if error.kind() == std::io::ErrorKind::NotFound);
    // Permission and I/O errors are not absence and must never enable fallback.
    if let Err(error) = &manifest
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(invalid(format!(
            "cannot read '{}': {error}",
            manifest_path.display()
        )));
    }
    if let Err(error) = &lock
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(invalid(format!(
            "cannot read '{}': {error}",
            lock_path.display()
        )));
    }
    if missing(&manifest) && missing(&lock) {
        return Ok(None);
    }
    if missing(&lock) {
        return Err(invalid(format!(
            "'{}' exists but '{}' is missing; run 'orbit-launcher install'",
            manifest_path.display(),
            lock_path.display()
        )));
    }
    if missing(&manifest) {
        return Err(invalid(format!(
            "'{}' exists but '{}' is missing",
            lock_path.display(),
            manifest_path.display()
        )));
    }
    let manifest: RuntimeManifest = parse(&manifest?, "launcher manifest", &manifest_path)?;
    let lock: RuntimeLock = parse(&lock?, "launcher lock", &lock_path)?;
    if !MANIFEST_SCHEMAS.contains(&manifest.schema) {
        return Err(invalid(format!(
            "launcher manifest schema {} is unsupported; supported: {MANIFEST_SCHEMAS:?}",
            manifest.schema
        )));
    }
    if !LOCK_SCHEMAS.contains(&lock.schema) {
        return Err(invalid(format!(
            "launcher lock schema {} is unsupported; supported: {LOCK_SCHEMAS:?}",
            lock.schema
        )));
    }
    if manifest.id != lock.instance_id {
        return Err(invalid(
            "orbit-launcher.toml and orbit-launcher.lock identify different instances",
        ));
    }
    if manifest.kind != lock.kind {
        return Err(invalid(
            "orbit-launcher.toml and orbit-launcher.lock select different instance kinds",
        ));
    }
    Ok(Some((manifest, lock)))
}

fn parse<T: for<'de> Deserialize<'de>>(
    text: &str,
    label: &str,
    path: &Path,
) -> Result<T, OrbitError> {
    toml::from_str(text).map_err(|error| {
        invalid(format!(
            "cannot parse {label} '{}': {error}",
            path.display()
        ))
    })
}
