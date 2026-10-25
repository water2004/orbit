use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::instance_lifecycle::InstanceLease;
use crate::{InstanceRegistry, LauncherError, ManifestFile, RegistryEntry, RuntimePaths};

#[derive(Debug, Serialize)]
pub struct InstanceDeletionPlan {
    pub instance_id: uuid::Uuid,
    pub name: String,
    pub directory: PathBuf,
    pub includes_worlds_and_mods: bool,
}

pub fn plan_instance_deletion(
    paths: &RuntimePaths,
    selector: &str,
) -> Result<InstanceDeletionPlan, LauncherError> {
    let registry = InstanceRegistry::load(&paths.instances_file())?;
    let entry = registry
        .find(selector)
        .ok_or_else(|| LauncherError::InstanceNotFound(selector.into()))?;
    validate(&registry, entry, paths)?;
    Ok(InstanceDeletionPlan {
        instance_id: entry.id,
        name: entry.name.clone(),
        directory: entry.instance_directory().into(),
        includes_worlds_and_mods: true,
    })
}

fn validate(
    registry: &InstanceRegistry,
    entry: &RegistryEntry,
    paths: &RuntimePaths,
) -> Result<(), LauncherError> {
    let root = entry.instance_directory();
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir()
        || crate::instance_lifecycle::is_redirected(&metadata)
        || dunce::canonicalize(root)? != root
        || root.parent().is_none()
    {
        return Err(LauncherError::Transaction(
            "refusing to delete a redirected or non-canonical instance root".into(),
        ));
    }
    if ManifestFile::open(root)?.inner.id != entry.id {
        return Err(LauncherError::InstanceRegistryMismatch(
            "instance deletion manifest ID differs".into(),
        ));
    }
    if registry
        .instances
        .iter()
        .any(|e| e.id != entry.id && e.instance_directory().starts_with(root))
        || [paths.config_dir(), paths.data_dir(), paths.cache_dir()]
            .iter()
            .any(|p| p.starts_with(root))
    {
        return Err(LauncherError::Transaction("instance contains another registered instance or Launcher global storage; refusing complete deletion".into()));
    }
    if let Some(repository) = entry.location.minecraft_directory()
        && (root == repository || !root.starts_with(repository.join("instances")))
    {
        return Err(LauncherError::Transaction(
            "instance deletion cannot include shared runtime files".into(),
        ));
    }
    Ok(())
}

/// Move the entire instance to the OS recycle bin/trash; never silently fall
/// back to permanent recursive removal if trash is unavailable (e.g. a server).
pub fn delete_instance(
    paths: &RuntimePaths,
    plan: &InstanceDeletionPlan,
) -> Result<RegistryEntry, LauncherError> {
    delete_with(paths, plan, |path| {
        trash::delete(path).map_err(|e| {
            LauncherError::Transaction(format!("cannot move instance to the system trash: {e}"))
        })
    })
}

fn delete_with(
    paths: &RuntimePaths,
    plan: &InstanceDeletionPlan,
    dispose: impl FnOnce(&Path) -> Result<(), LauncherError>,
) -> Result<RegistryEntry, LauncherError> {
    let registry_path = paths.instances_file();
    let _registry_guard = crate::registry::mutation_lock(&registry_path)?;
    let mut registry = InstanceRegistry::load(&registry_path)?;
    let entry = registry
        .find_by_id(plan.instance_id)
        .cloned()
        .ok_or_else(|| LauncherError::InstanceNotFound(plan.instance_id.to_string()))?;
    validate(&registry, &entry, paths)?;
    if plan.directory != entry.instance_directory() {
        return Err(LauncherError::Transaction(
            "instance directory changed after confirmation".into(),
        ));
    }
    let _lease = InstanceLease::acquire(&plan.directory, true)?;
    let staged = plan
        .directory
        .with_file_name(format!(".orbit-deleting-{}", entry.id));
    if staged.exists() {
        return Err(LauncherError::Transaction(format!(
            "deletion recovery directory '{}' already exists",
            staged.display()
        )));
    }
    std::fs::rename(&plan.directory, &staged)?;
    let original = registry.clone();
    registry.remove(entry.id);
    if let Err(error) = registry.save(&registry_path) {
        std::fs::rename(&staged, &plan.directory).map_err(|e| {
            LauncherError::Transaction(format!(
                "registry update failed: {error}; cannot restore '{}': {e}",
                staged.display()
            ))
        })?;
        return Err(error);
    }
    if let Err(error) = dispose(&staged) {
        let physical = std::fs::rename(&staged, &plan.directory);
        let registered = original.save(&registry_path);
        if physical.is_err() || registered.is_err() {
            return Err(LauncherError::Transaction(format!(
                "deletion failed: {error}; directory rollback: {physical:?}; registry rollback: {registered:?}; recovery path '{}'",
                staged.display()
            )));
        }
        return Err(error);
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CreateInstanceRequest, InstanceKind, LoaderKind, RuntimePathOptions, create_instance,
        set_default_instance,
    };

    fn fixture() -> (tempfile::TempDir, RuntimePaths, RegistryEntry) {
        let temp = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(temp.path().join("config")),
            data_dir: Some(temp.path().join("data")),
            cache_dir: Some(temp.path().join("cache")),
        })
        .unwrap();
        let created = create_instance(
            &paths,
            CreateInstanceRequest {
                directory: temp.path().join("minecraft"),
                name: "Test".into(),
                kind: InstanceKind::Client,
                minecraft_requirement: "1.21.1".into(),
                loader_kind: LoaderKind::Vanilla,
                loader_requirement: None,
            },
        )
        .unwrap();
        std::fs::create_dir_all(created.entry.instance_directory().join("saves/world")).unwrap();
        std::fs::write(
            created
                .entry
                .instance_directory()
                .join("saves/world/level.dat"),
            "world",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("minecraft/libraries")).unwrap();
        std::fs::write(temp.path().join("minecraft/libraries/shared"), "library").unwrap();
        (temp, paths, created.entry)
    }

    #[test]
    fn full_delete_includes_worlds_clears_default_and_preserves_shared_runtime() {
        let (temp, paths, entry) = fixture();
        set_default_instance(&paths, Some("Test")).unwrap();
        let plan = plan_instance_deletion(&paths, "Test").unwrap();
        assert!(entry.instance_directory().exists()); // planning is not removal
        delete_with(&paths, &plan, |staged| {
            assert!(staged.join("saves/world/level.dat").is_file());
            std::fs::remove_dir_all(staged)?;
            Ok(())
        })
        .unwrap();
        assert!(!entry.instance_directory().exists());
        let registry = InstanceRegistry::load(&paths.instances_file()).unwrap();
        assert!(registry.instances.is_empty());
        assert!(registry.default_instance.is_none());
        assert!(temp.path().join("minecraft/libraries/shared").is_file());
    }
    #[test]
    fn failed_trash_restores_directory_registry_and_default() {
        let (_temp, paths, entry) = fixture();
        set_default_instance(&paths, Some("Test")).unwrap();
        let original = std::fs::read(paths.instances_file()).unwrap();
        let plan = plan_instance_deletion(&paths, "Test").unwrap();
        assert!(
            delete_with(&paths, &plan, |_| Err(LauncherError::Transaction(
                "trash unavailable".into()
            )))
            .is_err()
        );
        assert!(
            entry
                .instance_directory()
                .join("saves/world/level.dat")
                .is_file()
        );
        assert_eq!(original, std::fs::read(paths.instances_file()).unwrap());
    }
    #[test]
    fn running_or_installing_instance_cannot_be_deleted() {
        let (_temp, paths, entry) = fixture();
        let plan = plan_instance_deletion(&paths, "Test").unwrap();
        let _lease = InstanceLease::acquire(entry.instance_directory(), false).unwrap();
        assert!(delete_with(&paths, &plan, |_| panic!("cannot reach deletion")).is_err());
        assert!(
            InstanceRegistry::load(&paths.instances_file())
                .unwrap()
                .find("Test")
                .is_some()
        );
    }
    #[test]
    fn changed_identity_after_confirmation_cannot_delete_another_instance() {
        let (_temp, paths, entry) = fixture();
        let plan = plan_instance_deletion(&paths, "Test").unwrap();
        let mut file = ManifestFile::open(entry.instance_directory()).unwrap();
        file.inner.id = uuid::Uuid::new_v4();
        file.save().unwrap();
        assert!(delete_with(&paths, &plan, |_| panic!("cannot reach deletion")).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn locked_file_error_does_not_unregister_or_remove_user_data() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_temp, paths, entry) = fixture();
        let plan = plan_instance_deletion(&paths, "Test").unwrap();
        let _occupied = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(entry.instance_directory().join("saves/world/level.dat"))
            .unwrap();
        assert!(
            delete_with(&paths, &plan, |staged| {
                std::fs::remove_dir_all(staged)?;
                Ok(())
            })
            .is_err()
        );
        assert!(
            entry
                .instance_directory()
                .join("saves/world/level.dat")
                .is_file()
        );
        assert!(
            InstanceRegistry::load(&paths.instances_file())
                .unwrap()
                .find("Test")
                .is_some()
        );
    }
}
