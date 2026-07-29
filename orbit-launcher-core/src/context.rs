use std::path::Path;

use serde::Serialize;

use crate::error::LauncherError;
use crate::instance::{INSTANCE_MANIFEST_FILE, InstanceManifest, ManifestFile};
use crate::registry::{InstanceRegistry, RegistryEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextIntent {
    ReadOnly,
    Sensitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    Explicit,
    CurrentDirectory,
    Default,
}

impl ContextSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::CurrentDirectory => "current_directory",
            Self::Default => "default",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedInstance {
    pub entry: RegistryEntry,
    pub manifest: InstanceManifest,
    pub source: ContextSource,
}

pub fn resolve_instance(
    registry: &InstanceRegistry,
    selector: Option<&str>,
    current_dir: &Path,
    intent: ContextIntent,
) -> Result<ResolvedInstance, LauncherError> {
    if let Some(selector) = selector {
        let entry = registry
            .find(selector)
            .ok_or_else(|| LauncherError::InstanceNotFound(selector.to_string()))?;
        return verify_registered(entry, ContextSource::Explicit);
    }

    if current_dir.join(INSTANCE_MANIFEST_FILE).is_file() {
        let current_root = dunce::canonicalize(current_dir)?;
        let manifest_file = ManifestFile::open(&current_root)?;
        let entry = registry.find_by_id(manifest_file.inner.id).ok_or_else(|| {
            LauncherError::InstanceRegistryMismatch(format!(
                "local instance '{}' is not registered; run 'orbit-launcher instance import --directory {}'",
                manifest_file.inner.name,
                current_root.display()
            ))
        })?;
        let registered_root = dunce::canonicalize(entry.instance_directory()).map_err(|error| {
            LauncherError::InstanceRegistryMismatch(format!(
                "registered path '{}' is unavailable: {error}; import the moved instance",
                entry.instance_directory().display()
            ))
        })?;
        if registered_root != current_root {
            return Err(LauncherError::InstanceRegistryMismatch(format!(
                "local instance ID '{}' is registered at '{}'; import the moved instance",
                entry.id,
                entry.instance_directory().display()
            )));
        }
        return verify_pair(entry, manifest_file.inner, ContextSource::CurrentDirectory);
    }

    let default = registry
        .default_entry()
        .ok_or(LauncherError::InstanceContextRequired)?;
    if intent == ContextIntent::Sensitive {
        return Err(LauncherError::ExplicitInstanceRequired(
            default.name.clone(),
        ));
    }
    verify_registered(default, ContextSource::Default)
}

fn verify_registered(
    entry: &RegistryEntry,
    source: ContextSource,
) -> Result<ResolvedInstance, LauncherError> {
    let manifest = ManifestFile::open(entry.instance_directory())?.inner;
    verify_pair(entry, manifest, source)
}

fn verify_pair(
    entry: &RegistryEntry,
    manifest: InstanceManifest,
    source: ContextSource,
) -> Result<ResolvedInstance, LauncherError> {
    if entry.id != manifest.id || entry.name != manifest.name || entry.kind() != manifest.kind {
        return Err(LauncherError::InstanceRegistryMismatch(format!(
            "registry entry '{}' does not match {}",
            entry.name,
            entry
                .instance_directory()
                .join(INSTANCE_MANIFEST_FILE)
                .display()
        )));
    }
    Ok(ResolvedInstance {
        entry: entry.clone(),
        manifest,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{InstanceKind, InstanceManifest, LoaderKind};
    use crate::layout::InstanceLocation;
    use crate::registry::REGISTRY_SCHEMA;
    use uuid::Uuid;

    fn registered_instance(root: &Path, name: &str) -> (InstanceRegistry, RegistryEntry) {
        let game_directory = root.join("instances").join(name);
        std::fs::create_dir_all(&game_directory).unwrap();
        let manifest = InstanceManifest::new(
            Uuid::new_v4(),
            name,
            InstanceKind::Client,
            "1.21.1",
            LoaderKind::Vanilla,
            None,
        )
        .unwrap();
        ManifestFile::new(&game_directory, manifest.clone())
            .save()
            .unwrap();
        let root = dunce::canonicalize(root).unwrap();
        let game_directory = dunce::canonicalize(game_directory).unwrap();
        let location = InstanceLocation::client(root, game_directory).unwrap();
        let entry = RegistryEntry::from_manifest(location, &manifest);
        let registry = InstanceRegistry {
            schema: REGISTRY_SCHEMA,
            default_instance: Some(entry.id),
            instances: vec![entry.clone()],
        };
        (registry, entry)
    }

    #[test]
    fn explicit_then_current_then_default_context_order_is_stable() {
        let directory = tempfile::tempdir().unwrap();
        let (registry, entry) = registered_instance(&directory.path().join("client"), "client");
        let unrelated = directory.path().join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();

        let explicit = resolve_instance(
            &registry,
            Some(&entry.id.to_string()),
            &unrelated,
            ContextIntent::Sensitive,
        )
        .unwrap();
        assert_eq!(explicit.source, ContextSource::Explicit);

        let local = resolve_instance(
            &registry,
            None,
            entry.instance_directory(),
            ContextIntent::Sensitive,
        )
        .unwrap();
        assert_eq!(local.source, ContextSource::CurrentDirectory);

        let default =
            resolve_instance(&registry, None, &unrelated, ContextIntent::ReadOnly).unwrap();
        assert_eq!(default.source, ContextSource::Default);
    }

    #[test]
    fn sensitive_command_does_not_silently_use_default_instance() {
        let directory = tempfile::tempdir().unwrap();
        let (registry, _) = registered_instance(&directory.path().join("client"), "client");
        let error = resolve_instance(&registry, None, directory.path(), ContextIntent::Sensitive)
            .unwrap_err();
        assert!(matches!(error, LauncherError::ExplicitInstanceRequired(_)));
    }
}
