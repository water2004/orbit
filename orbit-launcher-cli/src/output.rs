use std::path::PathBuf;

use orbit_launcher_core::{
    AccountMetadata, AccountProvider, ArtifactTransferEvent, ContextSource, InstallProgressEvent,
    InstallerOutputStream, InstallerSide, InstanceManifest, JavaProgressEvent, LaunchOutputStream,
    LaunchPlanSummary, LaunchPreparationEvent, LaunchProcessEvent, LaunchResult,
    LoaderInstallerEvent, MicrosoftDeviceSession, MicrosoftLoginProgressEvent, RegistryEntry,
};
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
pub struct YggdrasilProviderView {
    pub id: String,
    pub api_root: String,
    pub allow_insecure_http: bool,
}

impl From<orbit_launcher_core::YggdrasilProviderConfig> for YggdrasilProviderView {
    fn from(provider: orbit_launcher_core::YggdrasilProviderConfig) -> Self {
        Self {
            id: provider.id,
            api_root: provider.api_root,
            allow_insecure_http: provider.allow_insecure_http,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct YggdrasilProviderListView {
    pub providers: Vec<YggdrasilProviderView>,
}

#[derive(Debug, Serialize)]
pub struct YggdrasilProviderMutationView {
    pub action: &'static str,
    pub provider: YggdrasilProviderView,
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

#[derive(Debug, Serialize)]
pub struct EulaDocumentView {
    pub instance_id: String,
    pub url: String,
    pub digest_sha256: String,
    pub fetched_at_unix_seconds: u64,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct EulaAcceptanceView {
    pub instance_id: String,
    pub url: String,
    pub digest_sha256: String,
    pub accepted_at_unix_seconds: u64,
    pub method: &'static str,
}

#[derive(Debug, Serialize)]
pub struct InstallView {
    pub instance_id: String,
    pub kind: String,
    pub minecraft_version: String,
    pub loader: String,
    pub java_runtime_id: String,
    pub java_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eula_digest_sha256: Option<String>,
    pub downloaded_artifacts: usize,
    pub cached_artifacts: usize,
}

#[derive(Debug, Serialize)]
pub struct LaunchPlanView {
    pub instance_id: String,
    pub kind: String,
    pub executable: PathBuf,
    pub working_directory: PathBuf,
    pub arguments: Vec<String>,
    pub redacted: bool,
}

impl From<LaunchPlanSummary> for LaunchPlanView {
    fn from(plan: LaunchPlanSummary) -> Self {
        Self {
            instance_id: plan.instance_id.to_string(),
            kind: plan.kind.as_str().to_string(),
            executable: plan.executable,
            working_directory: plan.working_directory,
            arguments: plan.arguments,
            redacted: true,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LaunchResultView {
    pub instance_id: String,
    pub kind: String,
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub elapsed_milliseconds: u128,
}

impl From<LaunchResult> for LaunchResultView {
    fn from(result: LaunchResult) -> Self {
        Self {
            instance_id: result.instance_id.to_string(),
            kind: result.kind.as_str().to_string(),
            pid: result.pid,
            exit_code: result.exit_code,
            success: result.success,
            elapsed_milliseconds: result.elapsed_milliseconds,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub id: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub profile_id: String,
    pub profile_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_name: Option<String>,
    pub is_default: bool,
    pub secret_backend: String,
    pub created_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_authenticated_at_unix_seconds: Option<u64>,
}

impl AccountView {
    pub fn new(
        account: &AccountMetadata,
        default: Option<uuid::Uuid>,
        secret_backend: &str,
    ) -> Self {
        let provider_id = match &account.provider {
            AccountProvider::ExternalYggdrasil { provider_id } => Some(provider_id.clone()),
            _ => None,
        };
        Self {
            id: account.id.to_string(),
            provider: account.provider.as_str().to_string(),
            provider_id,
            profile_id: account.profile_id.to_string(),
            profile_name: account.profile_name.clone(),
            login_name: account.login_name.clone(),
            is_default: default == Some(account.id),
            secret_backend: secret_backend.to_string(),
            created_at_unix_seconds: account.created_at_unix_seconds,
            last_authenticated_at_unix_seconds: account.last_authenticated_at_unix_seconds,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AccountListView {
    pub accounts: Vec<AccountView>,
}

#[derive(Debug, Serialize)]
pub struct AccountLoginView {
    pub method: &'static str,
    #[serde(flatten)]
    pub account: AccountView,
}

#[derive(Debug, Serialize)]
pub struct AccountSelectionView {
    pub scope: &'static str,
    pub account: Option<AccountView>,
}

#[derive(Debug, Serialize)]
pub struct AccountLogoutView {
    pub account: AccountView,
    pub local_secret_deleted: bool,
}

#[derive(Debug, Serialize)]
pub struct MicrosoftDeviceSessionView {
    pub login_session_id: String,
    pub verification_uri: String,
    pub user_code: String,
    pub expires_at_unix_seconds: u64,
    pub polling_interval_seconds: u64,
    pub message: Option<String>,
}

impl From<MicrosoftDeviceSession> for MicrosoftDeviceSessionView {
    fn from(session: MicrosoftDeviceSession) -> Self {
        Self {
            login_session_id: session.id.to_string(),
            verification_uri: session.verification_uri,
            user_code: session.user_code,
            expires_at_unix_seconds: session.expires_at_unix_seconds,
            polling_interval_seconds: session.polling_interval_seconds,
            message: session.message,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ProgressEnvelope {
    pub schema_version: u32,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub command: &'static str,
    pub sequence: u64,
    pub data: ProgressData,
}

impl ProgressEnvelope {
    pub fn new(command: &'static str, sequence: u64, data: ProgressData) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: "progress",
            command,
            sequence,
            data,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProgressData {
    MetadataStarted,
    MinecraftResolved {
        version: String,
        total_artifacts: usize,
    },
    EulaChecked {
        digest_sha256: String,
        accepted: bool,
    },
    ArtifactStarted {
        logical_name: String,
        total_bytes: Option<u64>,
    },
    ArtifactBytes {
        logical_name: String,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    ArtifactCached {
        logical_name: String,
        size: u64,
    },
    ArtifactFinished {
        logical_name: String,
        size: u64,
    },
    JavaManifestStarted,
    JavaRuntimeResolved {
        runtime_id: String,
        artifacts: usize,
        total_bytes: u64,
    },
    JavaMaterialized {
        completed: usize,
        total: usize,
    },
    JavaRuntimeVerified {
        runtime_id: String,
    },
    JavaRuntimeCached {
        runtime_id: String,
    },
    LoaderInstallerStarted {
        loader: String,
        version: String,
        side: String,
    },
    LoaderInstallerOutput {
        stream: String,
        line: String,
    },
    LoaderInstallerOutputSuppressed {
        maximum_lines: usize,
    },
    LoaderInstallerFinished {
        loader: String,
        version: String,
    },
    StagingVerified,
    Committed,
    MicrosoftAuthorizationPolling {
        attempt: u64,
        elapsed_seconds: u64,
        expires_at_unix_seconds: u64,
    },
    MicrosoftAuthorizationReceived,
    XboxAuthenticated,
    MinecraftAuthenticated,
    AccountSessionStored {
        account_id: String,
    },
    LaunchArtifactVerified {
        completed: usize,
        total: usize,
    },
    LaunchJavaVerified {
        runtime_id: String,
    },
    LaunchPlanReady,
    ProcessSpawned {
        pid: u32,
    },
    ProcessOutput {
        stream: LaunchOutputStream,
        line: String,
    },
    ProcessExited {
        exit_code: Option<i32>,
        success: bool,
    },
}

impl From<InstallProgressEvent> for ProgressData {
    fn from(event: InstallProgressEvent) -> Self {
        match event {
            InstallProgressEvent::MetadataStarted => Self::MetadataStarted,
            InstallProgressEvent::MinecraftResolved {
                version,
                total_artifacts,
            } => Self::MinecraftResolved {
                version,
                total_artifacts,
            },
            InstallProgressEvent::EulaChecked {
                digest_sha256,
                accepted,
            } => Self::EulaChecked {
                digest_sha256,
                accepted,
            },
            InstallProgressEvent::Artifact(event) => Self::from_artifact(event),
            InstallProgressEvent::Java(event) => Self::from_java(event),
            InstallProgressEvent::LoaderInstaller(event) => Self::from_installer(event),
            InstallProgressEvent::StagingVerified => Self::StagingVerified,
            InstallProgressEvent::Committed => Self::Committed,
        }
    }
}

impl ProgressData {
    pub fn from_launch_preparation(event: LaunchPreparationEvent) -> Self {
        match event {
            LaunchPreparationEvent::ArtifactVerified { completed, total } => {
                Self::LaunchArtifactVerified { completed, total }
            }
            LaunchPreparationEvent::JavaVerified { runtime_id } => {
                Self::LaunchJavaVerified { runtime_id }
            }
            LaunchPreparationEvent::PlanReady => Self::LaunchPlanReady,
        }
    }

    pub fn from_launch_process(event: LaunchProcessEvent) -> Self {
        match event {
            LaunchProcessEvent::Spawned { pid } => Self::ProcessSpawned { pid },
            LaunchProcessEvent::Output { stream, line } => Self::ProcessOutput { stream, line },
            LaunchProcessEvent::Exited { exit_code, success } => {
                Self::ProcessExited { exit_code, success }
            }
        }
    }

    pub fn from_microsoft(event: MicrosoftLoginProgressEvent) -> Self {
        match event {
            MicrosoftLoginProgressEvent::Polling {
                attempt,
                elapsed_seconds,
                expires_at_unix_seconds,
            } => Self::MicrosoftAuthorizationPolling {
                attempt,
                elapsed_seconds,
                expires_at_unix_seconds,
            },
            MicrosoftLoginProgressEvent::AuthorizationReceived => {
                Self::MicrosoftAuthorizationReceived
            }
            MicrosoftLoginProgressEvent::XboxAuthenticated => Self::XboxAuthenticated,
            MicrosoftLoginProgressEvent::MinecraftAuthenticated => Self::MinecraftAuthenticated,
            MicrosoftLoginProgressEvent::SessionStored { account_id } => {
                Self::AccountSessionStored {
                    account_id: account_id.to_string(),
                }
            }
        }
    }

    fn from_artifact(event: ArtifactTransferEvent) -> Self {
        match event {
            ArtifactTransferEvent::Started {
                logical_name,
                total_bytes,
            } => Self::ArtifactStarted {
                logical_name,
                total_bytes,
            },
            ArtifactTransferEvent::Bytes {
                logical_name,
                downloaded_bytes,
                total_bytes,
            } => Self::ArtifactBytes {
                logical_name,
                downloaded_bytes,
                total_bytes,
            },
            ArtifactTransferEvent::Cached { logical_name, size } => {
                Self::ArtifactCached { logical_name, size }
            }
            ArtifactTransferEvent::Finished { logical_name, size } => {
                Self::ArtifactFinished { logical_name, size }
            }
        }
    }

    fn from_java(event: JavaProgressEvent) -> Self {
        match event {
            JavaProgressEvent::ManifestStarted => Self::JavaManifestStarted,
            JavaProgressEvent::RuntimeResolved {
                runtime_id,
                artifacts,
                total_bytes,
            } => Self::JavaRuntimeResolved {
                runtime_id,
                artifacts,
                total_bytes,
            },
            JavaProgressEvent::Artifact(event) => Self::from_artifact(event),
            JavaProgressEvent::Materialized { completed, total } => {
                Self::JavaMaterialized { completed, total }
            }
            JavaProgressEvent::RuntimeVerified { runtime_id } => {
                Self::JavaRuntimeVerified { runtime_id }
            }
            JavaProgressEvent::RuntimeCached { runtime_id } => {
                Self::JavaRuntimeCached { runtime_id }
            }
        }
    }

    fn from_installer(event: LoaderInstallerEvent) -> Self {
        match event {
            LoaderInstallerEvent::Started {
                kind,
                version,
                side,
            } => Self::LoaderInstallerStarted {
                loader: kind.as_str().to_string(),
                version,
                side: match side {
                    InstallerSide::Client => "client",
                    InstallerSide::Server => "server",
                }
                .to_string(),
            },
            LoaderInstallerEvent::Output { stream, line } => Self::LoaderInstallerOutput {
                stream: match stream {
                    InstallerOutputStream::Stdout => "stdout",
                    InstallerOutputStream::Stderr => "stderr",
                }
                .to_string(),
                line,
            },
            LoaderInstallerEvent::OutputSuppressed { maximum_lines } => {
                Self::LoaderInstallerOutputSuppressed { maximum_lines }
            }
            LoaderInstallerEvent::Finished { kind, version } => Self::LoaderInstallerFinished {
                loader: kind.as_str().to_string(),
                version,
            },
        }
    }
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
