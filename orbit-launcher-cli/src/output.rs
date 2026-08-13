use std::path::{Path, PathBuf};

use orbit_launcher_core::{
    AccountAuthenticationState, AccountMetadata, AccountProvider, ArtifactTransferEvent,
    ContextSource, InstallProgressEvent, InstallerOutputStream, InstallerSide, InstanceManifest,
    JavaProgressEvent, LaunchOutputStream, LaunchPlanSummary, LaunchPreparationEvent,
    LaunchProcessEvent, LaunchResult, LoaderInstallerEvent, MicrosoftDeviceSession,
    MicrosoftLoginProgressEvent, RegistryEntry, RepositoryMoveEvent, StateArchiveProgressEvent,
    SupervisorEvent, SupervisorResult,
};
use serde::Serialize;

use crate::supervisor_ipc::SupervisorState;
pub use orbit_machine_protocol::{ErrorEnvelope, ProgressEnvelope, ProgressPhase, SuccessEnvelope};

#[derive(Debug, Clone, Serialize)]
pub struct InstanceView {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minecraft_directory: Option<PathBuf>,
    pub kind: String,
    pub is_default: bool,
}

impl InstanceView {
    pub fn from_entry(entry: &RegistryEntry, default: Option<uuid::Uuid>) -> Self {
        Self {
            id: entry.id.to_string(),
            name: entry.name.clone(),
            directory: entry.instance_directory().to_path_buf(),
            minecraft_directory: entry.location.minecraft_directory().map(Path::to_path_buf),
            kind: entry.kind().as_str().to_string(),
            is_default: default == Some(entry.id),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MinecraftDirectoryView {
    pub directory: PathBuf,
    pub explicit: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MinecraftDirectoryMoveView {
    pub previous: PathBuf,
    pub current: PathBuf,
    pub files: u64,
    pub copied_across_filesystems: bool,
    pub source_removed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageRequirementView {
    pub format: String,
    pub name: String,
    pub version: String,
    pub targets: Vec<String>,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub launcher_state: bool,
    pub orbit_content: bool,
    pub optional_files: Vec<PackageOptionalFileView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageOptionalFileView {
    pub path: String,
    pub targets: Vec<String>,
}

impl From<orbit_launcher_core::InstallPackRequirement> for PackageRequirementView {
    fn from(value: orbit_launcher_core::InstallPackRequirement) -> Self {
        Self {
            format: match value.format {
                orbit_launcher_core::InstallPackFormat::Orbit => "orbit",
                orbit_launcher_core::InstallPackFormat::Mrpack => "mrpack",
            }
            .to_string(),
            name: value.name,
            version: value.version,
            targets: value
                .targets
                .into_iter()
                .map(|target| target.as_str().to_string())
                .collect(),
            minecraft: value.minecraft,
            loader: value.loader.as_str().to_string(),
            loader_version: value.loader_version,
            launcher_state: value.launcher_state,
            orbit_content: value.orbit_content,
            optional_files: value
                .optional_files
                .into_iter()
                .map(|file| PackageOptionalFileView {
                    path: file.path,
                    targets: file
                        .targets
                        .into_iter()
                        .map(|target| target.as_str().to_string())
                        .collect(),
                })
                .collect(),
        }
    }
}

impl From<orbit_launcher_core::MinecraftDirectoryMove> for MinecraftDirectoryMoveView {
    fn from(value: orbit_launcher_core::MinecraftDirectoryMove) -> Self {
        Self {
            previous: value.previous,
            current: value.current,
            files: value.files,
            copied_across_filesystems: value.copied_across_filesystems,
            source_removed: value.source_removed,
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
    pub installed: Option<InstalledRuntimeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_resolution: Option<ClientResolutionView>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ClientResolutionView {
    pub width: u32,
    pub height: u32,
}

impl InstanceDetailView {
    pub fn new(
        entry: &RegistryEntry,
        manifest: &InstanceManifest,
        installed: Option<&orbit_launcher_core::LauncherLock>,
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
            },
            installed: installed.map(InstalledRuntimeView::from),
            selected_account_id: manifest.launch.account.map(|account| account.to_string()),
            client_resolution: manifest
                .client
                .as_ref()
                .and_then(|client| client.resolution)
                .map(|resolution| ClientResolutionView {
                    width: resolution.width,
                    height: resolution.height,
                }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledRuntimeView {
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub java: Option<InstalledJavaView>,
}

impl From<&orbit_launcher_core::LauncherLock> for InstalledRuntimeView {
    fn from(lock: &orbit_launcher_core::LauncherLock) -> Self {
        Self {
            minecraft: lock.minecraft.version.clone(),
            loader: lock.loader.kind.as_str().to_string(),
            loader_version: lock.loader.version.clone(),
            java: lock.java.as_ref().map(InstalledJavaView::from),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledJavaView {
    pub provider: String,
    pub version: String,
    pub major: u32,
    pub platform: String,
}

impl From<&orbit_launcher_core::LockedJavaRuntime> for InstalledJavaView {
    fn from(java: &orbit_launcher_core::LockedJavaRuntime) -> Self {
        Self {
            provider: java.provider.clone(),
            version: java.version.clone(),
            major: java.major,
            platform: java.platform.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DesiredRuntimeView {
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MinecraftVersionCatalogView {
    pub latest_release: String,
    pub latest_snapshot: String,
    pub versions: Vec<MinecraftVersionView>,
}

impl From<orbit_launcher_core::MinecraftVersionCatalog> for MinecraftVersionCatalogView {
    fn from(catalog: orbit_launcher_core::MinecraftVersionCatalog) -> Self {
        Self {
            latest_release: catalog.latest_release,
            latest_snapshot: catalog.latest_snapshot,
            versions: catalog.versions.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MinecraftVersionView {
    pub id: String,
    pub version_type: String,
    pub release_time: String,
    pub latest_release: bool,
    pub latest_snapshot: bool,
}

impl From<orbit_launcher_core::MinecraftVersion> for MinecraftVersionView {
    fn from(version: orbit_launcher_core::MinecraftVersion) -> Self {
        Self {
            id: version.id,
            version_type: version.version_type,
            release_time: version.release_time,
            latest_release: version.latest_release,
            latest_snapshot: version.latest_snapshot,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LoaderVersionCatalogView {
    pub loader: String,
    pub minecraft: String,
    pub versions: Vec<LoaderVersionView>,
}

#[derive(Debug, Serialize)]
pub struct LoaderVersionView {
    pub version: String,
    pub stable: bool,
    pub recommended: bool,
    pub latest: bool,
    pub minimum_java_major: Option<u32>,
}

impl From<orbit_launcher_core::LoaderVersion> for LoaderVersionView {
    fn from(version: orbit_launcher_core::LoaderVersion) -> Self {
        Self {
            version: version.version,
            stable: version.stable,
            recommended: version.recommended,
            latest: version.latest,
            minimum_java_major: version.minimum_java_major,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JavaRequirementView {
    pub minecraft: String,
    pub required: bool,
    pub component: Option<String>,
    pub major: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JavaRuntimeView {
    pub runtime_id: String,
    pub provider: String,
    pub component: String,
    pub platform: String,
    pub version: String,
    pub major: u32,
    pub root: PathBuf,
    pub executable: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub verified: Option<bool>,
}

impl From<orbit_launcher_core::InstalledJavaRuntime> for JavaRuntimeView {
    fn from(runtime: orbit_launcher_core::InstalledJavaRuntime) -> Self {
        Self {
            runtime_id: runtime.runtime_id,
            provider: runtime.provider,
            component: runtime.component,
            platform: runtime.platform,
            version: runtime.version,
            major: runtime.major,
            root: runtime.root,
            executable: runtime.executable,
            files: runtime.files,
            bytes: runtime.bytes,
            verified: runtime.verified,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JavaRuntimeListView {
    pub verification_requested: bool,
    pub runtimes: Vec<JavaRuntimeView>,
}

#[derive(Debug, Serialize)]
pub struct JavaRuntimeMutationView {
    pub action: &'static str,
    pub runtime: JavaRuntimeView,
}

#[derive(Debug, Serialize)]
pub struct InstanceMutationView {
    pub instance: InstanceView,
    pub action: InstanceMutationAction,
    pub files_deleted: bool,
}

#[derive(Debug, Serialize)]
pub struct LauncherStateExportView {
    pub path: PathBuf,
    pub kind: String,
    pub minecraft_version: String,
    pub files: usize,
    pub bytes: u64,
    pub world_files: usize,
}

impl From<orbit_launcher_core::LauncherStateExportReport> for LauncherStateExportView {
    fn from(report: orbit_launcher_core::LauncherStateExportReport) -> Self {
        Self {
            path: report.path,
            kind: report.kind.as_str().to_string(),
            minecraft_version: report.minecraft_version,
            files: report.files,
            bytes: report.bytes,
            world_files: report.world_files,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LauncherStateRestoreView {
    pub kind: String,
    pub source_minecraft_version: String,
    pub target_minecraft_version: String,
    pub files: usize,
    pub bytes: u64,
    pub world_files: usize,
    pub restored_properties: usize,
    pub skipped_properties: Vec<String>,
}

impl From<orbit_launcher_core::LauncherStateRestoreReport> for LauncherStateRestoreView {
    fn from(report: orbit_launcher_core::LauncherStateRestoreReport) -> Self {
        Self {
            kind: report.kind.as_str().to_string(),
            source_minecraft_version: report.source_minecraft_version,
            target_minecraft_version: report.target_minecraft_version,
            files: report.files,
            bytes: report.bytes,
            world_files: report.world_files,
            restored_properties: report.restored_properties,
            skipped_properties: report.skipped_properties,
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<LauncherStateRestoreView>,
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

#[derive(Debug, Serialize)]
pub struct ServerStartView {
    pub state: SupervisorState,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct ServerStatusView {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<SupervisorState>,
}

#[derive(Debug, Serialize)]
pub struct ServerControlView {
    pub action: &'static str,
    pub accepted: bool,
    pub message: String,
    pub state: SupervisorState,
}

#[derive(Debug, Serialize)]
pub struct SupervisorResultView {
    pub instance_id: String,
    pub generations: u32,
    pub restarts: u32,
    pub final_exit_code: Option<i32>,
    pub final_success: bool,
    pub stopped_by_request: bool,
    pub restart_limit_reached: bool,
}

impl From<SupervisorResult> for SupervisorResultView {
    fn from(result: SupervisorResult) -> Self {
        Self {
            instance_id: result.instance_id.to_string(),
            generations: result.generations,
            restarts: result.restarts,
            final_exit_code: result.final_exit_code,
            final_success: result.final_success,
            stopped_by_request: result.stopped_by_request,
            restart_limit_reached: result.restart_limit_reached,
        }
    }
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
    pub authentication_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_path: Option<String>,
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
        paths: &orbit_launcher_core::runtime::RuntimePaths,
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
            authentication_state: match account.authentication_state {
                AccountAuthenticationState::Active => "active",
                AccountAuthenticationState::ReauthenticationRequired => "reauthentication-required",
            }
            .to_string(),
            avatar_path: orbit_launcher_core::account::account_avatar_path(paths, account)
                .filter(|path| path.is_file())
                .map(|path| path.to_string_lossy().into_owned()),
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
    ServerSettingsInitialized {
        properties: usize,
    },
    StateArchiveStarted {
        files: usize,
        completed: u64,
        total: u64,
    },
    StateArchiveAdvanced {
        completed: u64,
        total: u64,
    },
    StateArchiveFinished {
        files: usize,
        completed: u64,
        total: u64,
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
    LaunchNativesPrepared {
        files: usize,
    },
    LaunchPlanReady,
    RepositoryCopying {
        completed: u64,
        total: u64,
    },
    RepositoryVerifying {
        completed: u64,
        total: u64,
    },
    RepositorySwitching,
    RepositoryRemovingSource,
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
    SupervisorSpawned {
        pid: u32,
        generation: u32,
    },
    SupervisorCommandSent {
        command: String,
    },
    SupervisorStopRequested,
    SupervisorExited {
        exit_code: Option<i32>,
        success: bool,
        expected: bool,
        uptime_milliseconds: u128,
    },
    SupervisorBackoff {
        delay_seconds: u64,
        restart_attempt: u32,
    },
    SupervisorRestarting {
        generation: u32,
    },
    SupervisorRestartLimitReached {
        attempts: u32,
        window_seconds: u64,
    },
    SupervisorStopped,
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
            InstallProgressEvent::ServerSettingsInitialized { properties } => {
                Self::ServerSettingsInitialized { properties }
            }
            InstallProgressEvent::StagingVerified => Self::StagingVerified,
            InstallProgressEvent::Committed => Self::Committed,
        }
    }
}

impl ProgressData {
    pub fn from_state_archive(event: StateArchiveProgressEvent) -> Self {
        match event {
            StateArchiveProgressEvent::Started { files, total_bytes } => {
                Self::StateArchiveStarted {
                    files,
                    completed: 0,
                    total: total_bytes,
                }
            }
            StateArchiveProgressEvent::Advanced {
                completed_bytes,
                total_bytes,
            } => Self::StateArchiveAdvanced {
                completed: completed_bytes,
                total: total_bytes,
            },
            StateArchiveProgressEvent::Finished { files, total_bytes } => {
                Self::StateArchiveFinished {
                    files,
                    completed: total_bytes,
                    total: total_bytes,
                }
            }
        }
    }

    pub const fn phase(&self) -> ProgressPhase {
        match self {
            Self::MetadataStarted | Self::MinecraftResolved { .. } => ProgressPhase::Metadata,
            Self::EulaChecked { .. } => ProgressPhase::Eula,
            Self::ArtifactStarted { .. }
            | Self::ArtifactBytes { .. }
            | Self::ArtifactCached { .. }
            | Self::ArtifactFinished { .. } => ProgressPhase::Download,
            Self::JavaManifestStarted
            | Self::JavaRuntimeResolved { .. }
            | Self::JavaMaterialized { .. }
            | Self::JavaRuntimeVerified { .. }
            | Self::JavaRuntimeCached { .. } => ProgressPhase::Java,
            Self::LoaderInstallerStarted { .. }
            | Self::LoaderInstallerOutput { .. }
            | Self::LoaderInstallerOutputSuppressed { .. }
            | Self::LoaderInstallerFinished { .. } => ProgressPhase::Loader,
            Self::ServerSettingsInitialized { .. } => ProgressPhase::Metadata,
            Self::StateArchiveStarted { .. }
            | Self::StateArchiveAdvanced { .. }
            | Self::StateArchiveFinished { .. } => ProgressPhase::Export,
            Self::StagingVerified | Self::Committed => ProgressPhase::Apply,
            Self::MicrosoftAuthorizationPolling { .. }
            | Self::MicrosoftAuthorizationReceived
            | Self::XboxAuthenticated
            | Self::MinecraftAuthenticated
            | Self::AccountSessionStored { .. } => ProgressPhase::Authentication,
            Self::LaunchArtifactVerified { .. }
            | Self::LaunchJavaVerified { .. }
            | Self::LaunchNativesPrepared { .. }
            | Self::LaunchPlanReady => ProgressPhase::Launch,
            Self::RepositoryCopying { .. }
            | Self::RepositoryVerifying { .. }
            | Self::RepositorySwitching
            | Self::RepositoryRemovingSource => ProgressPhase::Apply,
            Self::ProcessSpawned { .. }
            | Self::ProcessOutput { .. }
            | Self::ProcessExited { .. } => ProgressPhase::Process,
            Self::SupervisorSpawned { .. }
            | Self::SupervisorCommandSent { .. }
            | Self::SupervisorStopRequested
            | Self::SupervisorExited { .. }
            | Self::SupervisorBackoff { .. }
            | Self::SupervisorRestarting { .. }
            | Self::SupervisorRestartLimitReached { .. }
            | Self::SupervisorStopped => ProgressPhase::Supervisor,
        }
    }

    pub fn from_launch_preparation(event: LaunchPreparationEvent) -> Self {
        match event {
            LaunchPreparationEvent::ArtifactVerified { completed, total } => {
                Self::LaunchArtifactVerified { completed, total }
            }
            LaunchPreparationEvent::JavaVerified { runtime_id } => {
                Self::LaunchJavaVerified { runtime_id }
            }
            LaunchPreparationEvent::NativesPrepared { files } => {
                Self::LaunchNativesPrepared { files }
            }
            LaunchPreparationEvent::PlanReady => Self::LaunchPlanReady,
        }
    }

    pub fn from_repository(event: RepositoryMoveEvent) -> Self {
        match event {
            RepositoryMoveEvent::Copying { completed, total } => {
                Self::RepositoryCopying { completed, total }
            }
            RepositoryMoveEvent::Verifying { completed, total } => {
                Self::RepositoryVerifying { completed, total }
            }
            RepositoryMoveEvent::SwitchingRegistry => Self::RepositorySwitching,
            RepositoryMoveEvent::RemovingSource => Self::RepositoryRemovingSource,
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

    pub fn from_supervisor(event: SupervisorEvent) -> Self {
        match event {
            SupervisorEvent::Spawned { pid, generation } => {
                Self::SupervisorSpawned { pid, generation }
            }
            SupervisorEvent::Output { stream, line } => Self::ProcessOutput { stream, line },
            SupervisorEvent::CommandSent { command } => Self::SupervisorCommandSent { command },
            SupervisorEvent::StopRequested => Self::SupervisorStopRequested,
            SupervisorEvent::Exited {
                exit_code,
                success,
                expected,
                uptime_milliseconds,
            } => Self::SupervisorExited {
                exit_code,
                success,
                expected,
                uptime_milliseconds,
            },
            SupervisorEvent::Backoff {
                delay_seconds,
                restart_attempt,
            } => Self::SupervisorBackoff {
                delay_seconds,
                restart_attempt,
            },
            SupervisorEvent::Restarting { generation } => Self::SupervisorRestarting { generation },
            SupervisorEvent::RestartLimitReached {
                attempts,
                window_seconds,
            } => Self::SupervisorRestartLimitReached {
                attempts,
                window_seconds,
            },
            SupervisorEvent::Stopped => Self::SupervisorStopped,
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
    use orbit_launcher_core::{
        ContextSource, InstanceKind, InstanceLocation, InstanceManifest, LoaderKind,
    };

    fn absolute(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"C:\{path}"))
        } else {
            PathBuf::from(format!("/{path}"))
        }
    }

    #[test]
    fn error_envelope_has_stable_gui_fields() {
        let envelope = ErrorEnvelope::new("instance.show", "instance_not_found", "missing");
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(
            json["schema_version"],
            orbit_machine_protocol::SCHEMA_VERSION
        );
        assert_eq!(json["type"], "error");
        assert_eq!(json["command"], "instance.show");
        assert_eq!(json["ok"], false);
        assert_eq!(json["code"], "instance_not_found");
    }

    #[test]
    fn instance_view_exposes_stable_id_instead_of_using_path_as_identity() {
        let id = uuid::Uuid::new_v4();
        let entry = RegistryEntry {
            id,
            name: "server".to_string(),
            location: InstanceLocation::server(absolute("srv/minecraft")).unwrap(),
        };
        let json = serde_json::to_value(InstanceView::from_entry(&entry, Some(id))).unwrap();
        assert_eq!(json["id"], id.to_string());
        assert_eq!(json["is_default"], true);
    }

    #[test]
    fn instance_detail_exposes_the_exact_selected_account() {
        let id = uuid::Uuid::new_v4();
        let account = uuid::Uuid::new_v4();
        let entry = RegistryEntry {
            id,
            name: "client".to_string(),
            location: InstanceLocation::client(
                absolute("games/minecraft"),
                absolute("games/minecraft/instances/client"),
            )
            .unwrap(),
        };
        let mut manifest = InstanceManifest::new(
            id,
            "client",
            InstanceKind::Client,
            "1.21.1",
            LoaderKind::Vanilla,
            None,
        )
        .unwrap();
        manifest.launch.account = Some(account);

        let detail =
            InstanceDetailView::new(&entry, &manifest, None, None, ContextSource::Explicit);
        let json = serde_json::to_value(detail).unwrap();
        assert_eq!(json["selected_account_id"], account.to_string());
    }
}
