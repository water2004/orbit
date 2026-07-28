use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{GlobalConfig, set_minecraft_directory};
use crate::error::LauncherError;
use crate::layout::InstanceLocation;
use crate::registry::InstanceRegistry;
use crate::runtime::RuntimePaths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryMoveEvent {
    Copying { completed: u64, total: u64 },
    Verifying { completed: u64, total: u64 },
    SwitchingRegistry,
    RemovingSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct MinecraftDirectoryMove {
    pub previous: PathBuf,
    pub current: PathBuf,
    pub files: u64,
    pub copied_across_filesystems: bool,
    pub source_removed: bool,
}

pub fn move_minecraft_directory<F>(
    paths: &RuntimePaths,
    config: &GlobalConfig,
    destination: &Path,
    mut progress: F,
) -> Result<MinecraftDirectoryMove, LauncherError>
where
    F: FnMut(RepositoryMoveEvent),
{
    if !destination.is_absolute() {
        return Err(LauncherError::InvalidConfig(format!(
            "Minecraft directory destination '{}' must be absolute",
            destination.display()
        )));
    }
    let configured_source = config
        .minecraft
        .directory
        .clone()
        .unwrap_or_else(|| paths.data_dir().join("minecraft"));
    let source = canonical_or_normalized(&configured_source)?;
    let prepared_destination = prepare_destination(destination)?;
    let destination = prepared_destination.path;
    if source == destination {
        return Err(LauncherError::InvalidConfig(
            "Minecraft directory is already at the requested destination".to_string(),
        ));
    }
    if destination.starts_with(&source) || source.starts_with(&destination) {
        return Err(LauncherError::InvalidConfig(
            "Minecraft directory cannot be moved into itself or one of its parents".to_string(),
        ));
    }
    let registry_path = paths.instances_file();
    let original_registry = InstanceRegistry::load(&registry_path)?;
    validate_client_repository(&original_registry, &source)?;
    let mut updated_registry = original_registry.clone();
    rewrite_client_locations(&mut updated_registry, &source, &destination)?;

    let mut files = 0_u64;
    let mut copied_across_filesystems = false;
    let source_existed = source.is_dir();
    if source_existed {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if prepared_destination.existed_empty {
            std::fs::remove_dir(&destination)?;
        }
        match std::fs::rename(&source, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
                copied_across_filesystems = true;
                let temporary = temporary_destination(&destination)?;
                let inventory = inventory(&source)?;
                files = inventory.len() as u64;
                let copied = (|| {
                    copy_inventory(&source, &temporary, &inventory, |completed| {
                        progress(RepositoryMoveEvent::Copying {
                            completed,
                            total: files,
                        });
                    })?;
                    verify_inventory(&source, &temporary, &inventory, |completed| {
                        progress(RepositoryMoveEvent::Verifying {
                            completed,
                            total: files,
                        });
                    })?;
                    std::fs::rename(&temporary, &destination).map_err(|error| {
                        LauncherError::Transaction(format!(
                            "cannot publish copied Minecraft directory '{}': {error}",
                            destination.display()
                        ))
                    })?;
                    Ok::<(), LauncherError>(())
                })();
                if let Err(error) = copied {
                    let _ = std::fs::remove_dir_all(&temporary);
                    restore_empty_destination(&destination, prepared_destination.existed_empty)?;
                    return Err(error);
                }
            }
            Err(error) => {
                restore_empty_destination(&destination, prepared_destination.existed_empty)?;
                return Err(LauncherError::Transaction(format!(
                    "cannot move Minecraft directory '{}' to '{}': {error}",
                    source.display(),
                    destination.display()
                )));
            }
        }
    } else {
        std::fs::create_dir_all(&destination)?;
    }

    progress(RepositoryMoveEvent::SwitchingRegistry);
    if let Err(error) = updated_registry.save(&registry_path) {
        rollback_physical_move(
            &source,
            &destination,
            copied_across_filesystems,
            source_existed,
            prepared_destination.existed_empty,
        )?;
        return Err(error);
    }
    if let Err(error) = set_minecraft_directory(&paths.config_file(), &destination) {
        let registry_rollback = original_registry.save(&registry_path);
        let physical_rollback = rollback_physical_move(
            &source,
            &destination,
            copied_across_filesystems,
            source_existed,
            prepared_destination.existed_empty,
        );
        return match (registry_rollback, physical_rollback) {
            (Ok(()), Ok(())) => Err(error),
            (registry, physical) => Err(LauncherError::Transaction(format!(
                "failed to update Minecraft directory configuration: {error}; registry rollback: {registry:?}; filesystem rollback: {physical:?}"
            ))),
        };
    }

    let source_removed = if copied_across_filesystems && source_existed {
        progress(RepositoryMoveEvent::RemovingSource);
        std::fs::remove_dir_all(&source).is_ok()
    } else {
        true
    };
    Ok(MinecraftDirectoryMove {
        previous: source,
        current: destination,
        files,
        copied_across_filesystems,
        source_removed,
    })
}

fn validate_client_repository(
    registry: &InstanceRegistry,
    source: &Path,
) -> Result<(), LauncherError> {
    for entry in &registry.instances {
        if let InstanceLocation::Client {
            minecraft_directory,
            ..
        } = &entry.location
            && minecraft_directory != source
        {
            return Err(LauncherError::InvalidRegistry(format!(
                "client instance '{}' uses '{}', but the managed Minecraft directory is '{}'; Orbit Launcher supports exactly one isolated client repository",
                entry.name,
                minecraft_directory.display(),
                source.display()
            )));
        }
    }
    Ok(())
}

fn rewrite_client_locations(
    registry: &mut InstanceRegistry,
    source: &Path,
    destination: &Path,
) -> Result<(), LauncherError> {
    for entry in &mut registry.instances {
        if let InstanceLocation::Client {
            minecraft_directory,
            game_directory,
        } = &mut entry.location
        {
            let relative = game_directory.strip_prefix(source).map_err(|_| {
                LauncherError::InvalidRegistry(format!(
                    "client game directory '{}' is outside '{}'",
                    game_directory.display(),
                    source.display()
                ))
            })?;
            *minecraft_directory = destination.to_path_buf();
            *game_directory = destination.join(relative);
        }
    }
    registry.validate()
}

struct PreparedDestination {
    path: PathBuf,
    existed_empty: bool,
}

fn prepare_destination(path: &Path) -> Result<PreparedDestination, LauncherError> {
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(LauncherError::Transaction(format!(
                "Minecraft directory destination '{}' must be a directory, not a file or symbolic link",
                path.display()
            )));
        }
        if std::fs::read_dir(path)?.next().transpose()?.is_some() {
            return Err(LauncherError::Transaction(format!(
                "Minecraft directory destination '{}' must be empty",
                path.display()
            )));
        }
        return Ok(PreparedDestination {
            path: dunce::canonicalize(path)?,
            existed_empty: true,
        });
    }
    Ok(PreparedDestination {
        path: normalize_new_destination(path)?,
        existed_empty: false,
    })
}

fn normalize_new_destination(path: &Path) -> Result<PathBuf, LauncherError> {
    let parent = path.parent().ok_or_else(|| {
        LauncherError::InvalidConfig(format!(
            "Minecraft directory destination '{}' has no parent",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;
    let parent = dunce::canonicalize(parent)?;
    let name = path.file_name().ok_or_else(|| {
        LauncherError::InvalidConfig("Minecraft directory destination has no name".to_string())
    })?;
    Ok(parent.join(name))
}

fn canonical_or_normalized(path: &Path) -> Result<PathBuf, LauncherError> {
    if path.exists() {
        dunce::canonicalize(path).map_err(Into::into)
    } else {
        normalize_new_destination(path)
    }
}

fn temporary_destination(destination: &Path) -> Result<PathBuf, LauncherError> {
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| LauncherError::InvalidConfig("destination name is not UTF-8".to_string()))?;
    Ok(destination.with_file_name(format!(".{name}.orbit-moving-{}", uuid::Uuid::new_v4())))
}

#[derive(Debug)]
struct InventoryEntry {
    relative: PathBuf,
    size: u64,
}

fn inventory(root: &Path) -> Result<Vec<InventoryEntry>, LauncherError> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                return Err(LauncherError::Transaction(format!(
                    "Minecraft directory contains unsupported symbolic link '{}'",
                    entry.path().display()
                )));
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                files.push(InventoryEntry {
                    relative: entry
                        .path()
                        .strip_prefix(root)
                        .expect("inventory entry descends from root")
                        .to_path_buf(),
                    size: metadata.len(),
                });
            } else {
                return Err(LauncherError::Transaction(format!(
                    "Minecraft directory contains unsupported filesystem entry '{}'",
                    entry.path().display()
                )));
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

fn copy_inventory<F>(
    source: &Path,
    destination: &Path,
    inventory: &[InventoryEntry],
    mut progress: F,
) -> Result<(), LauncherError>
where
    F: FnMut(u64),
{
    std::fs::create_dir_all(destination)?;
    for (index, entry) in inventory.iter().enumerate() {
        let target = destination.join(&entry.relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let copied = std::fs::copy(source.join(&entry.relative), target)?;
        if copied != entry.size {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "copied Minecraft file '{}' changed size",
                entry.relative.display()
            )));
        }
        progress(index as u64 + 1);
    }
    Ok(())
}

fn verify_inventory<F>(
    source: &Path,
    destination: &Path,
    inventory: &[InventoryEntry],
    mut progress: F,
) -> Result<(), LauncherError>
where
    F: FnMut(u64),
{
    for (index, entry) in inventory.iter().enumerate() {
        let source_hash = crate::artifact::hash_file_sha256(&source.join(&entry.relative))?;
        let destination_hash =
            crate::artifact::hash_file_sha256(&destination.join(&entry.relative))?;
        if source_hash != destination_hash {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "copied Minecraft file '{}' failed SHA-256 verification",
                entry.relative.display()
            )));
        }
        progress(index as u64 + 1);
    }
    Ok(())
}

fn rollback_physical_move(
    source: &Path,
    destination: &Path,
    copied: bool,
    source_existed: bool,
    destination_existed_empty: bool,
) -> Result<(), LauncherError> {
    if destination.exists() {
        if copied || !source_existed {
            std::fs::remove_dir_all(destination)?;
        } else {
            std::fs::rename(destination, source)?;
        }
    }
    restore_empty_destination(destination, destination_existed_empty)?;
    Ok(())
}

fn restore_empty_destination(
    destination: &Path,
    destination_existed_empty: bool,
) -> Result<(), LauncherError> {
    if destination_existed_empty && !destination.exists() {
        std::fs::create_dir(destination)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{InstanceKind, LoaderKind};
    use crate::operations::{CreateInstanceRequest, create_instance};
    use crate::runtime::RuntimePathOptions;

    #[test]
    fn empty_default_repository_can_be_relocated() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(directory.path().join("config")),
            data_dir: Some(directory.path().join("data")),
            cache_dir: Some(directory.path().join("cache")),
        })
        .unwrap();
        let destination = directory.path().join("custom-minecraft");
        let moved =
            move_minecraft_directory(&paths, &GlobalConfig::default(), &destination, |_| {})
                .unwrap();
        assert_eq!(moved.current, dunce::canonicalize(&destination).unwrap());
        assert_eq!(
            GlobalConfig::load(&paths.config_file())
                .unwrap()
                .minecraft
                .directory,
            Some(moved.current)
        );
    }

    #[test]
    fn relocation_moves_client_files_and_rewrites_every_registered_location() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(directory.path().join("config")),
            data_dir: Some(directory.path().join("data")),
            cache_dir: Some(directory.path().join("cache")),
        })
        .unwrap();
        let source = paths.data_dir().join("minecraft");
        let created = create_instance(
            &paths,
            CreateInstanceRequest {
                directory: source.clone(),
                name: "client".to_string(),
                kind: InstanceKind::Client,
                minecraft_requirement: "1.21.1".to_string(),
                loader_kind: LoaderKind::Fabric,
                loader_requirement: Some("stable".to_string()),
            },
        )
        .unwrap();
        std::fs::write(
            created.entry.instance_directory().join("marker.txt"),
            b"preserved",
        )
        .unwrap();
        let destination = directory.path().join("relocated-minecraft");

        let moved =
            move_minecraft_directory(&paths, &GlobalConfig::default(), &destination, |_| {})
                .unwrap();
        let registry = InstanceRegistry::load(&paths.instances_file()).unwrap();
        let entry = registry.find("client").unwrap();

        assert!(!source.exists());
        assert_eq!(
            entry.location.minecraft_directory(),
            Some(moved.current.as_path())
        );
        assert_eq!(
            std::fs::read(entry.instance_directory().join("marker.txt")).unwrap(),
            b"preserved"
        );
    }

    #[test]
    fn relocation_accepts_an_existing_empty_destination_selected_by_the_gui() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(directory.path().join("config")),
            data_dir: Some(directory.path().join("data")),
            cache_dir: Some(directory.path().join("cache")),
        })
        .unwrap();
        let source = paths.data_dir().join("minecraft");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("marker.txt"), b"preserved").unwrap();
        let destination = directory.path().join("selected-empty-directory");
        std::fs::create_dir(&destination).unwrap();

        let moved =
            move_minecraft_directory(&paths, &GlobalConfig::default(), &destination, |_| {})
                .unwrap();

        assert_eq!(moved.current, dunce::canonicalize(&destination).unwrap());
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(destination.join("marker.txt")).unwrap(),
            b"preserved"
        );
    }

    #[test]
    fn relocation_rejects_a_nonempty_destination() {
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(directory.path().join("config")),
            data_dir: Some(directory.path().join("data")),
            cache_dir: Some(directory.path().join("cache")),
        })
        .unwrap();
        let destination = directory.path().join("nonempty-directory");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("unrelated.txt"), b"keep").unwrap();

        let error =
            move_minecraft_directory(&paths, &GlobalConfig::default(), &destination, |_| {})
                .unwrap_err();

        assert!(error.to_string().contains("must be empty"));
        assert_eq!(
            std::fs::read(destination.join("unrelated.txt")).unwrap(),
            b"keep"
        );
    }
}
