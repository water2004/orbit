//! Resolve an Orbit target independently of CLI presentation and process globals.

use crate::{InstancesRegistry, OrbitError};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSource {
    Explicit(String),
    Local,
    Default(String),
    Uninitialized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceContext {
    pub directory: PathBuf,
    pub source: WorkspaceSource,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceContextError {
    #[error(transparent)]
    Registry(#[from] OrbitError),
    #[error("unknown instance '{0}'; run 'orbit instance list'")]
    UnknownInstance(String),
    #[error(
        "refusing to modify default instance '{name}' outside its project directory; select it explicitly or change to {path}"
    )]
    ImplicitMutation { name: String, path: PathBuf },
}

impl WorkspaceContext {
    /// A default is a read-only convenience, never implicit write authority.
    pub fn require_mutation_target(&self) -> Result<(), WorkspaceContextError> {
        if let WorkspaceSource::Default(name) = &self.source {
            return Err(WorkspaceContextError::ImplicitMutation {
                name: name.clone(),
                path: self.directory.clone(),
            });
        }
        Ok(())
    }
}

pub fn resolve_workspace_context(
    registry_path: &Path,
    selector: Option<&str>,
    current_dir: &Path,
) -> Result<WorkspaceContext, WorkspaceContextError> {
    if let Some(name) = selector {
        let registry = InstancesRegistry::load(registry_path)?;
        let entry = registry
            .find(name)
            .ok_or_else(|| WorkspaceContextError::UnknownInstance(name.into()))?;
        return Ok(WorkspaceContext {
            directory: PathBuf::from(&entry.path),
            source: WorkspaceSource::Explicit(entry.name.clone()),
        });
    }
    if current_dir.join("orbit.toml").is_file() {
        return Ok(WorkspaceContext {
            directory: current_dir.into(),
            source: WorkspaceSource::Local,
        });
    }
    let registry = InstancesRegistry::load(registry_path)?;
    if let Some(entry) = registry.default_instance() {
        return Ok(WorkspaceContext {
            directory: PathBuf::from(&entry.path),
            source: WorkspaceSource::Default(entry.name.clone()),
        });
    }
    Ok(WorkspaceContext {
        directory: current_dir.into(),
        source: WorkspaceSource::Uninitialized,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_local_and_default_contexts_have_distinct_authority() {
        let temp = tempfile::tempdir().unwrap();
        let registry_path = temp.path().join("instances.toml");
        let local = temp.path().join("local");
        std::fs::create_dir(&local).unwrap();
        std::fs::write(local.join("orbit.toml"), "").unwrap();
        let remote = temp.path().join("other");
        InstancesRegistry {
            instances: vec![crate::InstanceEntry {
                name: "Test".into(),
                path: remote.to_string_lossy().into_owned(),
                mc_version: "1.21.1".into(),
                modloader: "fabric".into(),
                is_default: true,
            }],
        }
        .save(&registry_path)
        .unwrap();
        let explicit = resolve_workspace_context(&registry_path, Some("test"), &local).unwrap();
        assert_eq!(explicit.directory, remote);
        assert!(explicit.require_mutation_target().is_ok());
        let local = resolve_workspace_context(&registry_path, None, &local).unwrap();
        assert_eq!(local.source, WorkspaceSource::Local);
        assert!(local.require_mutation_target().is_ok());
        let default = resolve_workspace_context(&registry_path, None, temp.path()).unwrap();
        assert_eq!(default.source, WorkspaceSource::Default("Test".into()));
        assert!(matches!(
            default.require_mutation_target(),
            Err(WorkspaceContextError::ImplicitMutation { .. })
        ));
        assert!(matches!(
            resolve_workspace_context(&registry_path, Some("missing"), temp.path()),
            Err(WorkspaceContextError::UnknownInstance(_))
        ));
    }

    #[test]
    fn uninitialized_directory_is_not_replaced_or_silently_repaired() {
        let temp = tempfile::tempdir().unwrap();
        let result =
            resolve_workspace_context(&temp.path().join("absent.toml"), None, temp.path()).unwrap();
        assert_eq!(result.source, WorkspaceSource::Uninitialized);
        assert_eq!(result.directory, temp.path());
        assert!(result.require_mutation_target().is_ok());
        assert!(crate::ManifestFile::open(&result.directory).is_err());
    }
}
