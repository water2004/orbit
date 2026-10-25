//! Foreign launcher metadata is interpreted only at the import boundary.
//! Runtime installation and launch never consult these adapters.
mod directories;
mod managed;
mod modrinth;
mod profile;
#[cfg(test)]
mod tests;
mod transfer;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{InstanceKind, LauncherError, LoaderKind};
pub use transfer::{
    ExternalImportEvent, ExternalImportPlan, ExternalImportRequest, ExternalImportResult,
    import_external_instance, plan_external_import,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalRuntime {
    pub minecraft: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalInstance {
    /// Stable within this source; never a display name or scan-order index.
    pub id: String,
    pub name: String,
    pub format: String,
    pub kind: InstanceKind,
    pub profile_directory: PathBuf,
    pub game_directory: Option<PathBuf>,
    pub runtime: Option<ExternalRuntime>,
    pub directory_evidence: String,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExternalInstanceCatalog {
    pub directory: PathBuf,
    pub instances: Vec<ExternalInstance>,
}

pub fn inspect_external_instances(
    directory: &Path,
) -> Result<ExternalInstanceCatalog, LauncherError> {
    let directory = dunce::canonicalize(directory)?;
    if !directory.is_dir() || directory.parent().is_none() {
        return Err(invalid(
            "select a Minecraft repository or instance directory",
        ));
    }
    let mut instances = Vec::new();
    if directory.join(crate::INSTANCE_MANIFEST_FILE).is_file() {
        // Inspection never triggers storage upgrades or writes to the source.
        // The registration operation runs the normal ordered upgrader later.
        let manifest: crate::InstanceManifest = toml::from_str(&std::fs::read_to_string(
            directory.join(crate::INSTANCE_MANIFEST_FILE),
        )?)
        .map_err(LauncherError::ManifestParse)?;
        instances.push(ExternalInstance {
            id: manifest.id.to_string(),
            name: manifest.name,
            format: "orbit".into(),
            kind: manifest.kind,
            profile_directory: directory.clone(),
            game_directory: Some(directory.clone()),
            runtime: Some(ExternalRuntime {
                minecraft: manifest.minecraft.requirement,
                loader: manifest.loader.kind,
                loader_version: manifest.loader.requirement,
            }),
            directory_evidence: "Orbit instance manifest".into(),
            warnings: Vec::new(),
            error: None,
        });
    } else if let Some(found) = modrinth::probe(&directory)? {
        instances = found;
    } else {
        // A selected instance is exact: do not enumerate its sibling instances.
        let component_root = if directory.join("mmc-pack.json").is_file() {
            Some(directory.clone())
        } else {
            directory
                .parent()
                .filter(|p| p.join("mmc-pack.json").is_file())
                .map(Path::to_path_buf)
        };
        let isolated_root = if directory.file_name().and_then(|v| v.to_str()) == Some("instance")
            && directory
                .parent()
                .is_some_and(|p| p.join("instance.json").is_file())
        {
            directory.parent().unwrap_or(&directory)
        } else {
            &directory
        };
        if let Some(root) = component_root {
            instances.push(profile::prism(&root)?);
        } else if let Some(instance) = managed::probe(isolated_root)? {
            instances.push(instance);
        } else {
            let root = if directory.join(".minecraft/versions").is_dir() {
                directory.join(".minecraft")
            } else {
                directory.clone()
            };
            let exact_version = root
                .parent()
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
                == Some("versions");
            if root.join("versions").is_dir() && !exact_version {
                for entry in std::fs::read_dir(root.join("versions"))? {
                    let path = entry?.path();
                    if path.is_dir()
                        && !crate::instance_lifecycle::is_redirected(&std::fs::symlink_metadata(
                            &path,
                        )?)
                        && let Some(id) = path.file_name().and_then(|v| v.to_str())
                        && path.join(format!("{id}.json")).is_file()
                    {
                        instances.push(profile::standard(&root, &path, id));
                    }
                }
            } else if root
                .parent()
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
                == Some("versions")
            {
                let repository = root
                    .parent()
                    .and_then(Path::parent)
                    .ok_or_else(|| invalid("version has no repository"))?;
                let id = root
                    .file_name()
                    .and_then(|v| v.to_str())
                    .ok_or_else(|| invalid("non-UTF-8 version ID"))?;
                instances.push(profile::standard(repository, &root, id));
            } else {
                // A launcher instance-library folder is a flat collection, not
                // an unbounded recursive search of the user's disk.
                for child in std::fs::read_dir(&root)? {
                    let child = child?.path();
                    if !child.is_dir()
                        || crate::instance_lifecycle::is_redirected(&std::fs::symlink_metadata(
                            &child,
                        )?)
                    {
                        continue;
                    }
                    let mut candidate = if child.join("mmc-pack.json").is_file() {
                        Some(profile::prism(&child)?)
                    } else {
                        managed::probe(&child)?
                    };
                    if let Some(instance) = &mut candidate {
                        instance.id = child
                            .file_name()
                            .ok_or_else(|| invalid("invalid instance directory"))?
                            .to_string_lossy()
                            .into_owned();
                    }
                    if let Some(instance) = candidate {
                        instances.push(instance);
                    }
                }
            }
        }
    }
    instances.sort_by(|a, b| a.id.cmp(&b.id));
    if instances.is_empty() {
        return Err(invalid("no installed version profiles found"));
    }
    Ok(ExternalInstanceCatalog {
        directory,
        instances,
    })
}

pub(super) fn invalid(message: impl Into<String>) -> LauncherError {
    LauncherError::UnsupportedRequirement(format!("external instance import: {}", message.into()))
}

pub(super) fn read_json(path: &Path) -> Result<serde_json::Value, LauncherError> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > 16 * 1024 * 1024 {
        return Err(invalid("launcher metadata exceeds 16 MiB"));
    }
    serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|e| invalid(format!("{}: {e}", path.display())))
}
