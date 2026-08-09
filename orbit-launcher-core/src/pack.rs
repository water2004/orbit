use std::path::Path;

use orbit_bundle_format::{BundleArchive, InstanceTarget, LauncherContent, MrpackArchive};

use crate::error::LauncherError;
use crate::instance::{InstanceKind, LoaderKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPackFormat {
    Orbit,
    Mrpack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPackOptionalFile {
    pub path: String,
    pub targets: Vec<InstanceKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPackRequirement {
    pub format: InstallPackFormat,
    pub name: String,
    pub version: String,
    pub targets: Vec<InstanceKind>,
    pub minecraft: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
    pub launcher_state: bool,
    pub orbit_content: bool,
    pub optional_files: Vec<InstallPackOptionalFile>,
}

pub fn inspect_install_pack(source: &Path) -> Result<InstallPackRequirement, LauncherError> {
    match source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("orbitbundle") => inspect_orbit(source),
        Some("mrpack") => inspect_mrpack(source),
        _ => Err(LauncherError::InvalidRemoteData(
            "install --from expects an .orbitbundle or .mrpack package".to_string(),
        )),
    }
}

fn inspect_orbit(source: &Path) -> Result<InstallPackRequirement, LauncherError> {
    let pack = BundleArchive::open(source)?;
    let launcher = pack.manifest.launcher.as_ref().ok_or_else(|| {
        LauncherError::InvalidRemoteData(
            "Orbit bundle has no Launcher projection and cannot create a runtime".to_string(),
        )
    })?;
    let targets = pack
        .manifest
        .targets
        .iter()
        .copied()
        .map(map_target)
        .collect();
    let loader = pack.manifest.runtime.loader.parse::<LoaderKind>()?;
    Ok(InstallPackRequirement {
        format: InstallPackFormat::Orbit,
        name: pack.manifest.name,
        version: pack.manifest.version,
        targets,
        minecraft: pack.manifest.runtime.minecraft,
        loader,
        loader_version: pack.manifest.runtime.loader_version,
        launcher_state: launcher.content == LauncherContent::RuntimeAndState,
        orbit_content: pack.manifest.orbit.is_some(),
        optional_files: Vec::new(),
    })
}

fn inspect_mrpack(source: &Path) -> Result<InstallPackRequirement, LauncherError> {
    let pack = MrpackArchive::open(source)?;
    let runtime = pack.runtime()?;
    let loader = runtime.loader.parse::<LoaderKind>()?;
    Ok(InstallPackRequirement {
        format: InstallPackFormat::Mrpack,
        name: pack.index.name.clone(),
        version: pack.index.version_id.clone(),
        targets: vec![InstanceKind::Client, InstanceKind::Server],
        minecraft: runtime.minecraft,
        loader,
        loader_version: runtime.loader_version,
        launcher_state: false,
        orbit_content: true,
        optional_files: pack
            .index
            .files
            .iter()
            .filter_map(|file| {
                let mut targets = Vec::new();
                if file.env.client == orbit_bundle_format::MrpackSideRequirement::Optional {
                    targets.push(InstanceKind::Client);
                }
                if file.env.server == orbit_bundle_format::MrpackSideRequirement::Optional {
                    targets.push(InstanceKind::Server);
                }
                (!targets.is_empty()).then(|| InstallPackOptionalFile {
                    path: file.path.clone(),
                    targets,
                })
            })
            .collect(),
    })
}

const fn map_target(target: InstanceTarget) -> InstanceKind {
    match target {
        InstanceTarget::Client => InstanceKind::Client,
        InstanceTarget::Server => InstanceKind::Server,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn mrpack_inspection_exposes_exact_optional_entries_without_installing() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("pack.mrpack");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&source).unwrap());
        archive
            .start_file(
                orbit_bundle_format::MRPACK_INDEX_PATH,
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive
            .write_all(
                serde_json::to_string(&serde_json::json!({
                    "formatVersion": 1,
                    "game": "minecraft",
                    "versionId": "2.0",
                    "name": "Example",
                    "files": [{
                        "path": "mods/map.jar",
                        "hashes": { "sha1": "0".repeat(40), "sha512": "0".repeat(128) },
                        "env": { "client": "optional", "server": "unsupported" },
                        "downloads": ["https://cdn.modrinth.com/data/test/map.jar"],
                        "fileSize": 1
                    }],
                    "dependencies": {
                        "minecraft": "1.21.1",
                        "fabric-loader": "0.16.14"
                    }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        archive.finish().unwrap();

        let inspected = inspect_install_pack(&source).unwrap();
        assert_eq!(inspected.name, "Example");
        assert_eq!(inspected.version, "2.0");
        assert_eq!(inspected.loader, LoaderKind::Fabric);
        assert_eq!(
            inspected.optional_files,
            [InstallPackOptionalFile {
                path: "mods/map.jar".to_string(),
                targets: vec![InstanceKind::Client],
            }]
        );
        assert!(!directory.path().join("mods/map.jar").exists());
    }
}
