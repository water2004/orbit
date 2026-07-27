use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::LauncherError;
use crate::instance::{
    INSTANCE_MANIFEST_FILE, InstanceKind, InstanceManifest, LoaderKind, ManifestFile,
    validate_instance_name,
};
use crate::registry::{InstanceRegistry, RegistryEntry};
use crate::runtime::RuntimePaths;

pub fn resolve_instance_root(
    current_dir: &Path,
    requested: Option<&Path>,
) -> Result<PathBuf, LauncherError> {
    if !current_dir.is_absolute() {
        return Err(LauncherError::RelativeInstanceRoot(
            current_dir.to_path_buf(),
        ));
    }
    let root = requested.unwrap_or(current_dir);
    if root.is_absolute() {
        Ok(root.to_path_buf())
    } else {
        Ok(current_dir.join(root))
    }
}

#[derive(Debug, Clone)]
pub struct CreateInstanceRequest {
    pub root: PathBuf,
    pub name: String,
    pub kind: InstanceKind,
    pub minecraft_requirement: String,
    pub loader_kind: LoaderKind,
    pub loader_requirement: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateInstanceResult {
    pub entry: RegistryEntry,
    pub manifest: InstanceManifest,
}

pub fn create_instance(
    paths: &RuntimePaths,
    request: CreateInstanceRequest,
) -> Result<CreateInstanceResult, LauncherError> {
    if !request.root.is_absolute() {
        return Err(LauncherError::RelativeInstanceRoot(request.root));
    }
    let manifest = InstanceManifest::new(
        Uuid::new_v4(),
        request.name,
        request.kind,
        request.minecraft_requirement,
        request.loader_kind,
        request.loader_requirement,
    )?;
    let registry_path = paths.instances_file();
    let mut registry = InstanceRegistry::load(&registry_path)?;
    let root_created = !request.root.exists();
    if root_created {
        std::fs::create_dir_all(&request.root)?;
    } else if !request.root.is_dir() {
        return Err(LauncherError::InstanceRootNotDirectory(request.root));
    }
    let root = dunce::canonicalize(&request.root)?;
    let manifest_path = root.join(INSTANCE_MANIFEST_FILE);
    if manifest_path.exists() {
        cleanup_created_root(&root, root_created);
        return Err(LauncherError::InvalidManifest(format!(
            "{} already exists; import the instance or use its local context",
            manifest_path.display()
        )));
    }

    let entry = RegistryEntry::from_manifest(root.clone(), &manifest);
    if let Err(error) = registry.ensure_available(entry.id, &entry.name, &entry.root) {
        cleanup_created_root(&root, root_created);
        return Err(error);
    }

    if let Err(error) = ManifestFile::new(&root, manifest.clone()).save() {
        cleanup_created_root(&root, root_created);
        return Err(error);
    }
    registry.push(entry.clone());
    if let Err(error) = registry.save(&registry_path) {
        let cleanup_result = std::fs::remove_file(&manifest_path);
        cleanup_created_root(&root, root_created);
        return match cleanup_result {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(LauncherError::Transaction(format!(
                "failed to register instance: {error}; also failed to remove uncommitted manifest '{}': {cleanup_error}",
                manifest_path.display()
            ))),
        };
    }
    Ok(CreateInstanceResult { entry, manifest })
}

fn cleanup_created_root(root: &Path, root_created: bool) {
    if root_created {
        let _ = std::fs::remove_dir(root);
    }
}

#[derive(Debug, Clone)]
pub struct ImportInstanceResult {
    pub entry: RegistryEntry,
    pub newly_registered: bool,
    pub moved: bool,
}

pub fn import_instance(
    paths: &RuntimePaths,
    root: &Path,
) -> Result<ImportInstanceResult, LauncherError> {
    if !root.is_absolute() {
        return Err(LauncherError::RelativeInstanceRoot(root.to_path_buf()));
    }
    if !root.is_dir() {
        return Err(LauncherError::InstanceRootNotDirectory(root.to_path_buf()));
    }
    let root = dunce::canonicalize(root)?;
    let manifest = ManifestFile::open(&root)?.inner;
    let entry = RegistryEntry::from_manifest(root.clone(), &manifest);
    let registry_path = paths.instances_file();
    let mut registry = InstanceRegistry::load(&registry_path)?;

    for other in registry
        .instances
        .iter()
        .filter(|other| other.id != entry.id)
    {
        if other.name.eq_ignore_ascii_case(&entry.name) {
            return Err(LauncherError::DuplicateInstanceName(entry.name));
        }
        if other.root == root {
            return Err(LauncherError::DuplicateInstancePath(root));
        }
    }

    let mut newly_registered = true;
    let mut moved = false;
    if let Some(existing) = registry
        .instances
        .iter_mut()
        .find(|existing| existing.id == entry.id)
    {
        newly_registered = false;
        if existing.root != root {
            if existing.root.join(INSTANCE_MANIFEST_FILE).is_file() {
                return Err(LauncherError::DuplicateInstanceId(entry.id));
            }
            moved = true;
        }
        *existing = entry.clone();
    } else {
        registry.push(entry.clone());
    }
    registry.save(&registry_path)?;
    Ok(ImportInstanceResult {
        entry,
        newly_registered,
        moved,
    })
}

#[derive(Debug, Clone)]
pub struct RemoveInstanceResult {
    pub entry: RegistryEntry,
    pub default_cleared: bool,
}

pub fn remove_instance(
    paths: &RuntimePaths,
    selector: &str,
) -> Result<RemoveInstanceResult, LauncherError> {
    let registry_path = paths.instances_file();
    let mut registry = InstanceRegistry::load(&registry_path)?;
    let entry = registry
        .find(selector)
        .cloned()
        .ok_or_else(|| LauncherError::InstanceNotFound(selector.to_string()))?;
    let default_cleared = registry.default_instance == Some(entry.id);
    registry.remove(entry.id);
    registry.save(&registry_path)?;
    Ok(RemoveInstanceResult {
        entry,
        default_cleared,
    })
}

#[derive(Debug, Clone)]
pub struct RenameInstanceResult {
    pub id: Uuid,
    pub old_name: String,
    pub new_name: String,
}

pub fn rename_instance(
    paths: &RuntimePaths,
    selector: &str,
    new_name: &str,
) -> Result<RenameInstanceResult, LauncherError> {
    validate_instance_name(new_name)?;
    let registry_path = paths.instances_file();
    let mut registry = InstanceRegistry::load(&registry_path)?;
    let current = registry
        .find(selector)
        .cloned()
        .ok_or_else(|| LauncherError::InstanceNotFound(selector.to_string()))?;
    if registry
        .instances
        .iter()
        .any(|entry| entry.id != current.id && entry.name.eq_ignore_ascii_case(new_name))
    {
        return Err(LauncherError::DuplicateInstanceName(new_name.to_string()));
    }
    let mut manifest_file = ManifestFile::open(&current.root)?;
    if manifest_file.inner.id != current.id {
        return Err(LauncherError::InstanceRegistryMismatch(format!(
            "registry entry '{}' and manifest have different IDs",
            current.name
        )));
    }

    let old_manifest = manifest_file.inner.clone();
    manifest_file.inner.name = new_name.to_string();
    registry
        .instances
        .iter_mut()
        .find(|entry| entry.id == current.id)
        .expect("entry was resolved from the same registry")
        .name = new_name.to_string();

    manifest_file.save()?;
    if let Err(registry_error) = registry.save(&registry_path) {
        let rollback = ManifestFile::new(&current.root, old_manifest).save();
        return match rollback {
            Ok(()) => Err(registry_error),
            Err(rollback_error) => Err(LauncherError::Transaction(format!(
                "failed to update registry: {registry_error}; also failed to roll back manifest: {rollback_error}"
            ))),
        };
    }
    Ok(RenameInstanceResult {
        id: current.id,
        old_name: current.name,
        new_name: new_name.to_string(),
    })
}

pub fn set_default_instance(
    paths: &RuntimePaths,
    selector: Option<&str>,
) -> Result<Option<RegistryEntry>, LauncherError> {
    let registry_path = paths.instances_file();
    let mut registry = InstanceRegistry::load(&registry_path)?;
    let selected = selector
        .map(|selector| {
            registry
                .find(selector)
                .cloned()
                .ok_or_else(|| LauncherError::InstanceNotFound(selector.to_string()))
        })
        .transpose()?;
    registry.default_instance = selected.as_ref().map(|entry| entry.id);
    registry.save(&registry_path)?;
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimePathOptions;

    fn paths(directory: &Path) -> RuntimePaths {
        RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(directory.join("config")),
            data_dir: Some(directory.join("data")),
            cache_dir: Some(directory.join("cache")),
        })
        .unwrap()
    }

    fn request(root: PathBuf, name: &str) -> CreateInstanceRequest {
        CreateInstanceRequest {
            root,
            name: name.to_string(),
            kind: InstanceKind::Server,
            minecraft_requirement: "1.21.1".to_string(),
            loader_kind: LoaderKind::Fabric,
            loader_requirement: Some("stable".to_string()),
        }
    }

    #[test]
    fn create_register_rename_default_and_remove_preserve_instance_files() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let root = directory.path().join("server");
        let created = create_instance(&paths, request(root.clone(), "server")).unwrap();
        assert!(root.join(INSTANCE_MANIFEST_FILE).is_file());

        let renamed = rename_instance(&paths, "server", "production").unwrap();
        assert_eq!(renamed.id, created.entry.id);
        assert_eq!(ManifestFile::open(&root).unwrap().inner.name, "production");

        let default = set_default_instance(&paths, Some("production"))
            .unwrap()
            .unwrap();
        assert_eq!(default.id, created.entry.id);
        let removed = remove_instance(&paths, "production").unwrap();
        assert!(removed.default_cleared);
        assert!(root.join(INSTANCE_MANIFEST_FILE).is_file());
        assert!(
            InstanceRegistry::load(&paths.instances_file())
                .unwrap()
                .instances
                .is_empty()
        );
    }

    #[test]
    fn importing_a_moved_instance_updates_only_the_registry_path() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let first = directory.path().join("first");
        let created = create_instance(&paths, request(first.clone(), "server")).unwrap();
        let moved = directory.path().join("moved");
        std::fs::rename(&first, &moved).unwrap();

        let imported = import_instance(&paths, &moved).unwrap();
        assert!(imported.moved);
        assert!(!imported.newly_registered);
        assert_eq!(imported.entry.id, created.entry.id);
    }

    #[test]
    fn importing_a_copy_with_the_same_id_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let first = directory.path().join("first");
        create_instance(&paths, request(first.clone(), "server")).unwrap();
        let copy = directory.path().join("copy");
        std::fs::create_dir_all(&copy).unwrap();
        std::fs::copy(
            first.join(INSTANCE_MANIFEST_FILE),
            copy.join(INSTANCE_MANIFEST_FILE),
        )
        .unwrap();

        assert!(matches!(
            import_instance(&paths, &copy),
            Err(LauncherError::DuplicateInstanceId(_))
        ));
    }
}
