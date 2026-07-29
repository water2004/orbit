use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;
use crate::instance::{InstanceManifest, validate_instance_name};
use crate::layout::InstanceLocation;

pub const REGISTRY_SCHEMA: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceRegistry {
    pub schema: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_instance: Option<Uuid>,
    #[serde(default)]
    pub instances: Vec<RegistryEntry>,
}

impl Default for InstanceRegistry {
    fn default() -> Self {
        Self {
            schema: REGISTRY_SCHEMA,
            default_instance: None,
            instances: Vec::new(),
        }
    }
}

impl InstanceRegistry {
    pub fn load(path: &Path) -> Result<Self, LauncherError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let registry: Self = toml::from_str(&content).map_err(LauncherError::RegistryParse)?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn save(&self, path: &Path) -> Result<(), LauncherError> {
        self.validate()?;
        let mut persisted = self.clone();
        persisted.instances.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        let content = toml::to_string_pretty(&persisted)?;
        write_atomic(path, content.as_bytes())
    }

    pub fn validate(&self) -> Result<(), LauncherError> {
        if self.schema != REGISTRY_SCHEMA {
            return Err(LauncherError::InvalidRegistry(format!(
                "unsupported schema {}; expected {REGISTRY_SCHEMA}",
                self.schema
            )));
        }
        for (index, entry) in self.instances.iter().enumerate() {
            entry.validate()?;
            if self.instances[..index]
                .iter()
                .any(|other| other.id == entry.id)
            {
                return Err(LauncherError::InvalidRegistry(format!(
                    "duplicate instance ID '{}'",
                    entry.id
                )));
            }
            if self.instances[..index]
                .iter()
                .any(|other| other.name.eq_ignore_ascii_case(&entry.name))
            {
                return Err(LauncherError::InvalidRegistry(format!(
                    "duplicate instance name '{}'",
                    entry.name
                )));
            }
            if self.instances[..index].iter().any(|other| {
                other.location.instance_directory() == entry.location.instance_directory()
            }) {
                return Err(LauncherError::InvalidRegistry(format!(
                    "duplicate instance path '{}'",
                    entry.location.instance_directory().display()
                )));
            }
        }
        if let Some(default) = self.default_instance
            && self.find_by_id(default).is_none()
        {
            return Err(LauncherError::InvalidRegistry(format!(
                "default instance '{default}' is not registered"
            )));
        }
        Ok(())
    }

    pub fn find(&self, selector: &str) -> Option<&RegistryEntry> {
        Uuid::parse_str(selector)
            .ok()
            .and_then(|id| self.find_by_id(id))
            .or_else(|| {
                self.instances
                    .iter()
                    .find(|entry| entry.name.eq_ignore_ascii_case(selector))
            })
    }

    pub fn find_by_id(&self, id: Uuid) -> Option<&RegistryEntry> {
        self.instances.iter().find(|entry| entry.id == id)
    }

    pub fn default_entry(&self) -> Option<&RegistryEntry> {
        self.default_instance.and_then(|id| self.find_by_id(id))
    }

    pub(crate) fn ensure_available(
        &self,
        id: Uuid,
        name: &str,
        instance_directory: &Path,
    ) -> Result<(), LauncherError> {
        if self.instances.iter().any(|entry| entry.id == id) {
            return Err(LauncherError::DuplicateInstanceId(id));
        }
        if self
            .instances
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(name))
        {
            return Err(LauncherError::DuplicateInstanceName(name.to_string()));
        }
        if self
            .instances
            .iter()
            .any(|entry| entry.location.instance_directory() == instance_directory)
        {
            return Err(LauncherError::DuplicateInstancePath(
                instance_directory.to_path_buf(),
            ));
        }
        Ok(())
    }

    pub(crate) fn push(&mut self, entry: RegistryEntry) {
        self.instances.push(entry);
    }

    pub(crate) fn remove(&mut self, id: Uuid) -> Option<RegistryEntry> {
        let index = self.instances.iter().position(|entry| entry.id == id)?;
        let removed = self.instances.remove(index);
        if self.default_instance == Some(id) {
            self.default_instance = None;
        }
        Some(removed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub id: Uuid,
    pub name: String,
    pub location: InstanceLocation,
}

impl RegistryEntry {
    pub fn from_manifest(location: InstanceLocation, manifest: &InstanceManifest) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name.clone(),
            location,
        }
    }

    pub fn kind(&self) -> crate::instance::InstanceKind {
        self.location.kind()
    }

    pub fn instance_directory(&self) -> &Path {
        self.location.instance_directory()
    }

    pub fn validate(&self) -> Result<(), LauncherError> {
        if self.id.is_nil() {
            return Err(LauncherError::InvalidRegistry(
                "instance ID cannot be nil".to_string(),
            ));
        }
        validate_instance_name(&self.name)
            .map_err(|error| LauncherError::InvalidRegistry(error.to_string()))?;
        self.location
            .validate()
            .map_err(LauncherError::InvalidRegistry)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(root: PathBuf, name: &str) -> RegistryEntry {
        RegistryEntry {
            id: Uuid::new_v4(),
            name: name.to_string(),
            location: InstanceLocation::server(root).unwrap(),
        }
    }

    #[test]
    fn registry_roundtrip_supports_id_and_case_insensitive_name_lookup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("instances.toml");
        let instance = entry(directory.path().join("client"), "Main Client");
        let id = instance.id;
        let registry = InstanceRegistry {
            schema: REGISTRY_SCHEMA,
            default_instance: Some(id),
            instances: vec![instance],
        };
        registry.save(&path).unwrap();

        let loaded = InstanceRegistry::load(&path).unwrap();
        assert_eq!(loaded.find(&id.to_string()).unwrap().id, id);
        assert_eq!(loaded.find("main client").unwrap().id, id);
        assert_eq!(loaded.default_entry().unwrap().id, id);
    }

    #[test]
    fn duplicate_names_are_rejected_case_insensitively() {
        let directory = tempfile::tempdir().unwrap();
        let registry = InstanceRegistry {
            schema: REGISTRY_SCHEMA,
            default_instance: None,
            instances: vec![
                entry(directory.path().join("one"), "Server"),
                entry(directory.path().join("two"), "server"),
            ],
        };
        assert!(registry.validate().is_err());
    }
}
