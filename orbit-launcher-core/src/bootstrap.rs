//! Fresh-instance installation boundary. Owns package validation, allocation, and rollback.
use crate::{
    CreateInstanceRequest, CreateInstanceResult, InstallPackFormat, InstanceKind, LauncherError,
    LoaderKind, RuntimeContext, RuntimePaths, create_instance, inspect_install_pack,
    resolve_directory, rollback_created_instance,
};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct NewInstallRequest {
    pub name: String,
    pub server_directory: Option<PathBuf>,
    pub kind: Option<InstanceKind>,
    pub minecraft: Option<String>,
    pub loader: Option<LoaderKind>,
    pub loader_version: Option<String>,
    pub from: Option<PathBuf>,
}

#[derive(Debug)]
pub struct PreparedNewInstall {
    pub created: CreateInstanceResult,
    pub state_archive: Option<PathBuf>,
    fresh_package_install: bool,
}

pub fn prepare_new_install(
    runtime: &RuntimeContext,
    current_dir: &Path,
    request: NewInstallRequest,
) -> Result<PreparedNewInstall, LauncherError> {
    let package = request
        .from
        .as_deref()
        .map(inspect_install_pack)
        .transpose()?;
    let fresh_package_install = package.is_some();
    let name = request.name;
    let kind: InstanceKind = match (request.kind, package.as_ref()) {
        (Some(kind), _) => kind,
        (None, Some(package)) if package.targets.len() == 1 => package.targets[0],
        (None, Some(_)) => {
            return Err(LauncherError::InvalidManifest(
                "install --from requires --kind when the package supports both client and server"
                    .to_string(),
            ));
        }
        (None, None) => {
            return Err(LauncherError::InvalidManifest(
                "install --new requires --kind".to_string(),
            ));
        }
    };
    if package
        .as_ref()
        .is_some_and(|package| !package.targets.contains(&kind))
    {
        return Err(LauncherError::InvalidManifest(format!(
            "the package does not support a {} instance",
            kind.as_str()
        )));
    }
    let minecraft = exact_package_value(
        "Minecraft",
        request.minecraft,
        package.as_ref().map(|package| package.minecraft.clone()),
        package
            .as_ref()
            .is_some_and(|package| package.format == InstallPackFormat::Mrpack),
    )?;
    let requested_loader: Option<crate::LoaderKind> = request.loader;
    let loader = match (requested_loader, package.as_ref()) {
        (Some(loader), Some(package))
            if package.format == InstallPackFormat::Mrpack && loader != package.loader =>
        {
            return Err(LauncherError::InvalidManifest(format!(
                "--loader {} conflicts with package Loader {}",
                loader.as_str(),
                package.loader.as_str()
            )));
        }
        (Some(loader), _) => loader,
        (None, Some(package)) => package.loader,
        (None, None) => crate::LoaderKind::Vanilla,
    };
    let loader_version = exact_package_value_optional(
        "Loader",
        request.loader_version,
        package
            .as_ref()
            .and_then(|package| package.loader_version.clone()),
        package
            .as_ref()
            .is_some_and(|package| package.format == InstallPackFormat::Mrpack),
    )?;
    let directory = match kind {
        InstanceKind::Client => {
            if request.server_directory.is_some() {
                return Err(LauncherError::InvalidManifest(
                        "--server-directory is invalid for client instances; use 'minecraft move' to relocate the managed repository"
                            .to_string(),
                    ));
            }
            runtime.minecraft_directory()
        }
        InstanceKind::Server => {
            resolve_directory(current_dir, request.server_directory.as_deref())?
        }
    };
    if fresh_package_install {
        let target = match kind {
            InstanceKind::Client => directory.join("instances").join(&name),
            InstanceKind::Server => directory.clone(),
        };
        if target.exists() {
            return Err(LauncherError::InvalidManifest(format!(
                "install --from requires a new instance directory; '{}' already exists",
                target.display()
            )));
        }
    }

    let created = create_instance(
        runtime.paths(),
        CreateInstanceRequest {
            directory,
            name,
            kind,
            minecraft_requirement: minecraft,
            loader_kind: loader,
            loader_requirement: loader_version,
        },
    )?;
    Ok(PreparedNewInstall {
        created,
        state_archive: request.from,
        fresh_package_install,
    })
}

impl PreparedNewInstall {
    pub fn rollback(self, paths: &RuntimePaths) -> Result<(), LauncherError> {
        let directory = self.created.entry.instance_directory().to_path_buf();
        rollback_created_instance(paths, &self.created.entry.id.to_string())?;
        // Only a package bootstrap requires and owns a previously nonexistent directory.
        if self.fresh_package_install && directory.exists() {
            std::fs::remove_dir_all(&directory).map_err(|error| {
                LauncherError::Transaction(format!(
                    "failed to remove fresh bootstrap directory '{}': {error}",
                    directory.display()
                ))
            })?;
        }
        Ok(())
    }
}

fn exact_package_value(
    name: &str,
    requested: Option<String>,
    packaged: Option<String>,
    strict: bool,
) -> Result<String, LauncherError> {
    match (requested, packaged) {
        (Some(requested), Some(packaged)) if strict && requested != packaged => {
            Err(LauncherError::InvalidManifest(format!(
                "--{} {requested} conflicts with package value {packaged}",
                name.to_ascii_lowercase()
            )))
        }
        (Some(requested), _) => Ok(requested),
        (None, Some(packaged)) => Ok(packaged),
        (None, None) => Err(LauncherError::InvalidManifest(format!(
            "install --new requires --{}",
            name.to_ascii_lowercase()
        ))),
    }
}

fn exact_package_value_optional(
    name: &str,
    requested: Option<String>,
    packaged: Option<String>,
    strict: bool,
) -> Result<Option<String>, LauncherError> {
    match (requested, packaged) {
        (Some(requested), Some(packaged)) if strict && requested != packaged => {
            Err(LauncherError::InvalidManifest(format!(
                "--{}-version {requested} conflicts with package value {packaged}",
                name.to_ascii_lowercase()
            )))
        }
        (Some(requested), _) => Ok(Some(requested)),
        (None, packaged) => Ok(packaged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_mrpack_requirements_cannot_disagree_with_package_metadata() {
        assert!(
            exact_package_value(
                "Minecraft",
                Some("1.21".into()),
                Some("1.20.1".into()),
                true
            )
            .is_err()
        );
        assert_eq!(
            exact_package_value("Minecraft", None, Some("1.20.1".into()), true).unwrap(),
            "1.20.1"
        );
        assert!(
            exact_package_value_optional("Loader", Some("a".into()), Some("b".into()), true)
                .is_err()
        );
    }
    #[test]
    fn new_install_is_allocated_and_rolled_back_without_network_or_gui_logic() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = RuntimeContext::load(crate::RuntimePathOptions {
            config_dir: Some(temp.path().join("config")),
            data_dir: Some(temp.path().join("data")),
            cache_dir: Some(temp.path().join("cache")),
        })
        .unwrap();
        let prepared = prepare_new_install(
            &runtime,
            temp.path(),
            NewInstallRequest {
                name: "test".into(),
                server_directory: None,
                kind: Some(InstanceKind::Client),
                minecraft: Some("1.21.1".into()),
                loader: None,
                loader_version: None,
                from: None,
            },
        )
        .unwrap();
        let id = prepared.created.entry.id.to_string();
        assert!(
            crate::InstanceRegistry::load(&runtime.paths().instances_file())
                .unwrap()
                .find(&id)
                .is_some()
        );
        prepared.rollback(runtime.paths()).unwrap();
        assert!(
            crate::InstanceRegistry::load(&runtime.paths().instances_file())
                .unwrap()
                .find(&id)
                .is_none()
        );
    }
}
