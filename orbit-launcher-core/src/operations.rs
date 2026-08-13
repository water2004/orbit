use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::LauncherError;
use crate::instance::{
    ClientConfig, ClientResolution, INSTANCE_MANIFEST_FILE, InstanceKind, InstanceManifest,
    LoaderKind, ManifestFile, validate_instance_name,
};
use crate::layout::{InstanceLocation, validate_directory_name};
use crate::registry::{InstanceRegistry, RegistryEntry};
use crate::runtime::RuntimePaths;

pub fn resolve_directory(
    current_dir: &Path,
    requested: Option<&Path>,
) -> Result<PathBuf, LauncherError> {
    if !current_dir.is_absolute() {
        return Err(LauncherError::RelativeInstanceDirectory(
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
    /// Minecraft repository directory for clients, dedicated server directory for servers.
    pub directory: PathBuf,
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
    if !request.directory.is_absolute() {
        return Err(LauncherError::RelativeInstanceDirectory(request.directory));
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
    if request.kind == InstanceKind::Client {
        validate_directory_name(&manifest.name).map_err(LauncherError::InvalidManifest)?;
    }
    let requested_directory = request.directory;
    let directory_created = !requested_directory.exists();
    if directory_created {
        std::fs::create_dir_all(&requested_directory)?;
    } else if !requested_directory.is_dir() {
        return Err(LauncherError::InstancePathNotDirectory(requested_directory));
    }
    let directory = dunce::canonicalize(&requested_directory)?;
    let instance_directory_created;
    let location = match manifest.kind {
        InstanceKind::Client => {
            let game_directory = directory.join("instances").join(&manifest.name);
            instance_directory_created = !game_directory.exists();
            std::fs::create_dir_all(&game_directory)?;
            let game_directory = dunce::canonicalize(game_directory)?;
            InstanceLocation::client(directory.clone(), game_directory)?
        }
        InstanceKind::Server => {
            instance_directory_created = directory_created;
            InstanceLocation::server(directory.clone())?
        }
    };
    let root = location.instance_directory().to_path_buf();
    let manifest_path = root.join(INSTANCE_MANIFEST_FILE);
    if manifest_path.exists() {
        cleanup_created_directories(
            &directory,
            &root,
            directory_created,
            instance_directory_created,
        );
        return Err(LauncherError::InvalidManifest(format!(
            "{} already exists; import the instance or use its local context",
            manifest_path.display()
        )));
    }

    let entry = RegistryEntry::from_manifest(location, &manifest);
    if let Err(error) = registry.ensure_available(entry.id, &entry.name, entry.instance_directory())
    {
        cleanup_created_directories(
            &directory,
            &root,
            directory_created,
            instance_directory_created,
        );
        return Err(error);
    }

    if let Err(error) = ManifestFile::new(&root, manifest.clone()).save() {
        cleanup_created_directories(
            &directory,
            &root,
            directory_created,
            instance_directory_created,
        );
        return Err(error);
    }
    registry.push(entry.clone());
    if let Err(error) = registry.save(&registry_path) {
        let cleanup_result = std::fs::remove_file(&manifest_path);
        cleanup_created_directories(
            &directory,
            &root,
            directory_created,
            instance_directory_created,
        );
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

fn cleanup_created_directories(base: &Path, root: &Path, base_created: bool, root_created: bool) {
    if root_created {
        let _ = std::fs::remove_dir(root);
    }
    if base_created && base != root {
        let _ = std::fs::remove_dir(base.join("instances"));
        let _ = std::fs::remove_dir(base);
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
        return Err(LauncherError::RelativeInstanceDirectory(root.to_path_buf()));
    }
    if !root.is_dir() {
        return Err(LauncherError::InstancePathNotDirectory(root.to_path_buf()));
    }
    let root = dunce::canonicalize(root)?;
    let manifest = ManifestFile::open(&root)?.inner;
    let location = InstanceLocation::import(&root, manifest.kind)?;
    let entry = RegistryEntry::from_manifest(location, &manifest);
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
        if other.instance_directory() == root {
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
        if existing.instance_directory() != root {
            if existing
                .instance_directory()
                .join(INSTANCE_MANIFEST_FILE)
                .is_file()
            {
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

#[derive(Debug, Clone, Default)]
pub struct ConfigureInstanceRequest {
    pub minecraft_requirement: Option<String>,
    pub loader_kind: Option<LoaderKind>,
    pub loader_requirement: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfigureInstanceResult {
    pub entry: RegistryEntry,
    pub manifest: InstanceManifest,
}

pub fn set_client_resolution(
    paths: &RuntimePaths,
    selector: &str,
    resolution: Option<ClientResolution>,
) -> Result<ConfigureInstanceResult, LauncherError> {
    if let Some(resolution) = resolution {
        resolution.validate()?;
    }
    let registry = InstanceRegistry::load(&paths.instances_file())?;
    let entry = registry
        .find(selector)
        .cloned()
        .ok_or_else(|| LauncherError::InstanceNotFound(selector.to_string()))?;
    let mut manifest_file = ManifestFile::open(entry.instance_directory())?;
    if manifest_file.inner.id != entry.id {
        return Err(LauncherError::InstanceRegistryMismatch(format!(
            "registry entry '{}' and manifest have different IDs",
            entry.name
        )));
    }
    if manifest_file.inner.kind != InstanceKind::Client {
        return Err(LauncherError::InvalidManifest(
            "server instances do not have a client window resolution".to_string(),
        ));
    }
    manifest_file.inner.client = resolution.map(|resolution| ClientConfig {
        resolution: Some(resolution),
    });
    manifest_file.save()?;
    Ok(ConfigureInstanceResult {
        entry,
        manifest: manifest_file.inner,
    })
}

pub fn configure_instance(
    paths: &RuntimePaths,
    selector: &str,
    request: ConfigureInstanceRequest,
) -> Result<ConfigureInstanceResult, LauncherError> {
    let registry = InstanceRegistry::load(&paths.instances_file())?;
    let entry = registry
        .find(selector)
        .cloned()
        .ok_or_else(|| LauncherError::InstanceNotFound(selector.to_string()))?;
    let mut manifest_file = ManifestFile::open(entry.instance_directory())?;
    if manifest_file.inner.id != entry.id {
        return Err(LauncherError::InstanceRegistryMismatch(format!(
            "registry entry '{}' and manifest have different IDs",
            entry.name
        )));
    }

    if let Some(requirement) = request.minecraft_requirement {
        manifest_file.inner.minecraft.requirement = requirement;
    }
    if let Some(loader) = request.loader_kind {
        let changed_kind = loader != manifest_file.inner.loader.kind;
        if changed_kind && loader != LoaderKind::Vanilla && request.loader_requirement.is_none() {
            return Err(LauncherError::InvalidManifest(format!(
                "switching to {} requires an explicit loader version requirement",
                loader.as_str()
            )));
        }
        manifest_file.inner.loader.kind = loader;
        manifest_file.inner.loader.requirement = match loader {
            LoaderKind::Vanilla => None,
            _ => request.loader_requirement.or_else(|| {
                (!changed_kind)
                    .then(|| manifest_file.inner.loader.requirement.clone())
                    .flatten()
            }),
        };
    } else if let Some(requirement) = request.loader_requirement {
        if manifest_file.inner.loader.kind == LoaderKind::Vanilla {
            return Err(LauncherError::InvalidManifest(
                "vanilla instances cannot have a loader version requirement".to_string(),
            ));
        }
        manifest_file.inner.loader.requirement = Some(requirement);
    }
    manifest_file.inner.validate()?;
    manifest_file.save()?;
    Ok(ConfigureInstanceResult {
        entry,
        manifest: manifest_file.inner,
    })
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
    let mut manifest_file = ManifestFile::open(current.instance_directory())?;
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
        let rollback = ManifestFile::new(current.instance_directory(), old_manifest).save();
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

/// Roll back the manifest and registry entry created by an unsuccessful
/// bootstrap. Runtime or user files are deliberately not inferred or removed.
pub fn rollback_created_instance(
    paths: &RuntimePaths,
    selector: &str,
) -> Result<RegistryEntry, LauncherError> {
    let registry = InstanceRegistry::load(&paths.instances_file())?;
    let entry = registry
        .find(selector)
        .cloned()
        .ok_or_else(|| LauncherError::InstanceNotFound(selector.to_string()))?;
    let manifest = ManifestFile::open(entry.instance_directory())?;
    if manifest.inner.id != entry.id {
        return Err(LauncherError::InstanceRegistryMismatch(format!(
            "cannot roll back instance '{}' because its manifest ID changed",
            entry.name
        )));
    }
    let removed = remove_instance(paths, selector)?.entry;
    std::fs::remove_file(entry.instance_directory().join(INSTANCE_MANIFEST_FILE)).map_err(|error| {
        LauncherError::Transaction(format!(
            "unregistered failed bootstrap '{}' but could not remove its provisional manifest: {error}",
            entry.name
        ))
    })?;
    Ok(removed)
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
            directory: root,
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
    fn client_creation_uses_one_repository_and_an_isolated_instance_directory() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let repository = directory.path().join("minecraft");
        let created = create_instance(
            &paths,
            CreateInstanceRequest {
                directory: repository.clone(),
                name: "fabric-1.21.1".to_string(),
                kind: InstanceKind::Client,
                minecraft_requirement: "1.21.1".to_string(),
                loader_kind: LoaderKind::Fabric,
                loader_requirement: Some("stable".to_string()),
            },
        )
        .unwrap();

        let repository = dunce::canonicalize(repository).unwrap();
        let game_directory = repository.join("instances/fabric-1.21.1");
        assert_eq!(created.entry.instance_directory(), game_directory);
        assert_eq!(
            created.entry.location.minecraft_directory(),
            Some(repository.as_path())
        );
        assert!(game_directory.join(INSTANCE_MANIFEST_FILE).is_file());
        assert!(!repository.join(INSTANCE_MANIFEST_FILE).exists());
        assert!(!repository.join("mods").exists());
    }

    #[test]
    fn configure_updates_desired_runtime_without_touching_registry_identity() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let root = directory.path().join("server");
        let created = create_instance(&paths, request(root.clone(), "server")).unwrap();

        let configured = configure_instance(
            &paths,
            "server",
            ConfigureInstanceRequest {
                minecraft_requirement: Some("1.22".to_string()),
                loader_kind: Some(LoaderKind::Neoforge),
                loader_requirement: Some("latest".to_string()),
            },
        )
        .unwrap();

        assert_eq!(configured.entry.id, created.entry.id);
        assert_eq!(configured.manifest.minecraft.requirement, "1.22");
        assert_eq!(configured.manifest.loader.kind, LoaderKind::Neoforge);
        assert_eq!(
            configured.manifest.loader.requirement.as_deref(),
            Some("latest")
        );
        assert_eq!(
            InstanceRegistry::load(&paths.instances_file())
                .unwrap()
                .find("server")
                .unwrap()
                .id,
            created.entry.id
        );
    }

    #[test]
    fn client_resolution_is_instance_local_and_can_be_cleared() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let repository = directory.path().join("minecraft");
        create_instance(
            &paths,
            CreateInstanceRequest {
                directory: repository,
                name: "client".to_string(),
                kind: InstanceKind::Client,
                minecraft_requirement: "1.21.1".to_string(),
                loader_kind: LoaderKind::Vanilla,
                loader_requirement: None,
            },
        )
        .unwrap();

        let configured = set_client_resolution(
            &paths,
            "client",
            Some(ClientResolution {
                width: 1920,
                height: 1080,
            }),
        )
        .unwrap();
        assert_eq!(
            configured.manifest.client.unwrap().resolution,
            Some(ClientResolution {
                width: 1920,
                height: 1080,
            })
        );
        assert!(
            set_client_resolution(&paths, "client", None)
                .unwrap()
                .manifest
                .client
                .is_none()
        );
    }

    #[test]
    fn server_rejects_client_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let root = directory.path().join("server");
        create_instance(&paths, request(root, "server")).unwrap();
        assert!(
            set_client_resolution(
                &paths,
                "server",
                Some(ClientResolution {
                    width: 1280,
                    height: 720,
                })
            )
            .is_err()
        );
    }

    #[test]
    fn configure_requires_a_version_when_switching_to_a_non_vanilla_loader() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let root = directory.path().join("server");
        create_instance(
            &paths,
            CreateInstanceRequest {
                directory: root,
                name: "server".to_string(),
                kind: InstanceKind::Server,
                minecraft_requirement: "1.21.1".to_string(),
                loader_kind: LoaderKind::Vanilla,
                loader_requirement: None,
            },
        )
        .unwrap();

        let error = configure_instance(
            &paths,
            "server",
            ConfigureInstanceRequest {
                loader_kind: Some(LoaderKind::Fabric),
                ..ConfigureInstanceRequest::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("loader version requirement"));
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

    #[test]
    fn failed_bootstrap_rollback_removes_only_registration_and_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let root = directory.path().join("server");
        let created = create_instance(
            &paths,
            CreateInstanceRequest {
                directory: root.clone(),
                name: "server".to_string(),
                kind: InstanceKind::Server,
                minecraft_requirement: "1.21.1".to_string(),
                loader_kind: LoaderKind::Vanilla,
                loader_requirement: None,
            },
        )
        .unwrap();
        std::fs::write(root.join("user-note.txt"), b"keep").unwrap();

        rollback_created_instance(&paths, &created.entry.id.to_string()).unwrap();
        assert!(!root.join(INSTANCE_MANIFEST_FILE).exists());
        assert!(root.join("user-note.txt").exists());
        assert!(
            InstanceRegistry::load(&paths.instances_file())
                .unwrap()
                .instances
                .is_empty()
        );
    }
}
