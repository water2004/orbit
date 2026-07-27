use std::path::PathBuf;

use orbit_launcher_core::{ContextSource, InstanceManifest, RegistryEntry};
use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct SuccessEnvelope<T> {
    pub schema_version: u32,
    pub command: &'static str,
    pub ok: bool,
    pub result: T,
}

impl<T> SuccessEnvelope<T> {
    pub fn new(command: &'static str, result: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            ok: true,
            result,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorEnvelope<'a> {
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub command: &'a str,
    pub code: &'a str,
    pub message: &'a str,
}

impl<'a> ErrorEnvelope<'a> {
    pub fn new(command: &'a str, code: &'a str, message: &'a str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: "error",
            command,
            code,
            message,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceView {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub kind: String,
    pub is_default: bool,
}

impl InstanceView {
    pub fn from_entry(entry: &RegistryEntry, default: Option<uuid::Uuid>) -> Self {
        Self {
            id: entry.id.to_string(),
            name: entry.name.clone(),
            root: entry.root.clone(),
            kind: entry.kind.as_str().to_string(),
            is_default: default == Some(entry.id),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InstanceListView {
    pub instances: Vec<InstanceView>,
}

#[derive(Debug, Serialize)]
pub struct InstanceDetailView {
    #[serde(flatten)]
    pub instance: InstanceView,
    pub context: ContextSource,
    pub desired: DesiredRuntimeView,
}

impl InstanceDetailView {
    pub fn new(
        entry: &RegistryEntry,
        manifest: &InstanceManifest,
        default: Option<uuid::Uuid>,
        context: ContextSource,
    ) -> Self {
        Self {
            instance: InstanceView::from_entry(entry, default),
            context,
            desired: DesiredRuntimeView {
                minecraft: manifest.minecraft.requirement.clone(),
                loader: manifest.loader.kind.as_str().to_string(),
                loader_version: manifest.loader.requirement.clone(),
                java_policy: manifest.java.policy.as_str().to_string(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DesiredRuntimeView {
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub java_policy: String,
}

#[derive(Debug, Serialize)]
pub struct InstanceMutationView {
    pub instance: InstanceView,
    pub action: InstanceMutationAction,
    pub files_deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceMutationAction {
    Created,
    Imported,
    Removed,
}

impl InstanceMutationAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Imported => "imported",
            Self::Removed => "removed",
        }
    }

    pub const fn command_name(self) -> &'static str {
        match self {
            Self::Created => "instance.create",
            Self::Imported => "instance.import",
            Self::Removed => "instance.remove",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RenameView {
    pub id: String,
    pub old_name: String,
    pub new_name: String,
}

#[derive(Debug, Serialize)]
pub struct DefaultView {
    pub instance: Option<InstanceView>,
}

#[derive(Debug, Serialize)]
pub struct ConfigPathView {
    pub path: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct ConfigListView {
    pub settings: Vec<ConfigEntryView>,
}

#[derive(Debug, Serialize)]
pub struct ConfigEntryView {
    pub key: &'static str,
    pub value: Option<String>,
    pub explicit: bool,
}

impl From<orbit_launcher_core::ConfigEntry> for ConfigEntryView {
    fn from(entry: orbit_launcher_core::ConfigEntry) -> Self {
        Self {
            key: entry.key.as_str(),
            value: entry.value,
            explicit: entry.explicit,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConfigMutationView {
    pub key: &'static str,
    pub previous: Option<String>,
    pub current: Option<String>,
    pub explicit: bool,
    pub action: ConfigMutationAction,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigMutationAction {
    Set,
    Unset,
}

impl ConfigMutationAction {
    pub const fn command_name(self) -> &'static str {
        match self {
            Self::Set => "config.set",
            Self::Unset => "config.unset",
        }
    }
}

impl ConfigMutationView {
    pub fn new(
        mutation: orbit_launcher_core::ConfigMutation,
        action: ConfigMutationAction,
    ) -> Self {
        Self {
            key: mutation.key.as_str(),
            previous: mutation.previous,
            current: mutation.current,
            explicit: mutation.explicit,
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_launcher_core::InstanceKind;

    #[test]
    fn error_envelope_has_stable_gui_fields() {
        let envelope = ErrorEnvelope::new("instance.show", "instance_not_found", "missing");
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], "instance_not_found");
    }

    #[test]
    fn instance_view_exposes_stable_id_instead_of_using_path_as_identity() {
        let id = uuid::Uuid::new_v4();
        let entry = RegistryEntry {
            id,
            name: "server".to_string(),
            root: PathBuf::from("/srv/minecraft"),
            kind: InstanceKind::Server,
        };
        let json = serde_json::to_value(InstanceView::from_entry(&entry, Some(id))).unwrap();
        assert_eq!(json["id"], id.to_string());
        assert_eq!(json["is_default"], true);
    }
}
