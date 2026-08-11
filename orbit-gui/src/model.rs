use std::path::PathBuf;

use orbit_machine_protocol::InteractionEnvelope;
use orbit_machine_protocol::ProgressPhase;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    #[default]
    Home,
    Library,
    Discover,
    Audit,
    Runtime,
    Accounts,
    Server,
    Settings,
}

impl Page {
    pub const ALL: [Self; 8] = [
        Self::Home,
        Self::Library,
        Self::Discover,
        Self::Audit,
        Self::Runtime,
        Self::Accounts,
        Self::Server,
        Self::Settings,
    ];

    pub fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Home => tr!("Home"),
            Self::Library => tr!("Mods"),
            Self::Discover => tr!("Browse"),
            Self::Audit => tr!("Compatibility"),
            Self::Runtime => tr!("Installations"),
            Self::Accounts => tr!("Accounts"),
            Self::Server => tr!("Server"),
            Self::Settings => tr!("Settings"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeInstance {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub minecraft_directory: Option<PathBuf>,
    pub kind: String,
    pub is_default: bool,
}

#[derive(Debug, Deserialize)]
pub struct RuntimeInstanceList {
    pub instances: Vec<RuntimeInstance>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LauncherInstallResult {
    pub instance_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MigrationResult {
    pub subcommand: String,
    pub target_directory: PathBuf,
    pub source_mc_version: String,
    pub target_mc_version: String,
    pub target_loader: String,
    pub target_loader_version: String,
    pub summary: MigrationSummary,
    #[serde(default)]
    pub changes: Vec<PackageChange>,
    #[serde(default)]
    pub diagnostics: Vec<ResolutionDiagnostic>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub export: Option<MigrationExportResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MigrationSummary {
    pub selected_packages: usize,
    pub installs: usize,
    pub upgrades: usize,
    pub downgrades: usize,
    pub replacements: usize,
    pub removals: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageChange {
    pub package: String,
    pub kind: String,
    pub current_version: Option<String>,
    pub selected_version: Option<String>,
    pub selected_description: Option<String>,
    #[serde(default)]
    pub selected_artifact: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MigrationExportResult {
    pub applied: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LauncherConfigEntry {
    pub key: String,
    pub value: Option<String>,
    pub explicit: bool,
}

#[derive(Debug, Deserialize)]
pub struct LauncherConfigList {
    pub settings: Vec<LauncherConfigEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrbitConfigEntry {
    pub key: String,
    pub sensitive: bool,
    pub value: Option<Value>,
}

impl OrbitConfigEntry {
    pub fn display_value(&self) -> String {
        match self.value.as_ref() {
            Some(Value::String(value)) => value.clone(),
            Some(Value::Number(value)) => value.to_string(),
            Some(value) => value.to_string(),
            None => String::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct OrbitConfigList {
    pub config_path: PathBuf,
    pub entries: Vec<OrbitConfigEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MinecraftDirectory {
    pub directory: PathBuf,
    pub explicit: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DesiredRuntime {
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeInstanceDetail {
    #[serde(flatten)]
    pub instance: RuntimeInstance,
    pub context: String,
    pub desired: DesiredRuntime,
    pub installed: Option<InstalledRuntime>,
    pub selected_account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstalledRuntime {
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub java: Option<InstalledJava>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstalledJava {
    pub provider: String,
    pub version: String,
    pub major: u32,
    pub platform: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MinecraftVersion {
    pub id: String,
    pub version_type: String,
    pub release_time: String,
    pub latest_release: bool,
    pub latest_snapshot: bool,
}

#[derive(Debug, Deserialize)]
pub struct MinecraftVersionCatalog {
    pub latest_release: String,
    pub latest_snapshot: String,
    pub versions: Vec<MinecraftVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
    pub recommended: bool,
    pub latest: bool,
    pub minimum_java_major: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct LoaderVersionCatalog {
    pub loader: String,
    pub minecraft: String,
    pub versions: Vec<LoaderVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaRequirement {
    pub minecraft: String,
    pub major: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstalledPackage {
    pub mod_id: String,
    pub version: String,
    pub enabled: bool,
    pub icon_path: Option<String>,
    #[serde(default)]
    pub remotes: Vec<String>,
    pub configured_environment: Option<String>,
    pub environment: String,
    pub optional: bool,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub bundled_count: usize,
}

#[derive(Debug, Deserialize)]
pub struct PackageList {
    pub packages: Vec<InstalledPackage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageOwnership {
    pub mod_id: String,
    pub artifacts: Vec<OwnedPackageArtifact>,
    pub data: Vec<OwnedPathRoot>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OwnedPackageArtifact {
    pub path: String,
    pub scope: String,
    pub present: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OwnedPathRoot {
    pub path: String,
    pub scope: String,
    pub kind: String,
    #[serde(default)]
    pub preserved: Vec<OwnershipPath>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OwnershipPath {
    pub path: String,
    pub scope: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageVersions {
    pub package: String,
    pub string: String,
    pub policy: PackageVersionPolicy,
    pub selected_version: Option<String>,
    pub candidates: Vec<PackageVersionCandidate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageVersionPolicy {
    Any,
    Comparison {
        operator: PackageVersionOperator,
        version: String,
    },
    Range {
        lower: String,
        upper: String,
        include_lower: bool,
        include_upper: bool,
    },
    Custom {
        requirement: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum PackageVersionOperator {
    #[serde(rename = "=")]
    Exact,
    #[serde(rename = ">")]
    GreaterThan,
    #[serde(rename = ">=")]
    AtLeast,
    #[serde(rename = "<")]
    LessThan,
    #[serde(rename = "<=")]
    AtMost,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageVersionCandidate {
    pub version: String,
    pub numeric_core: Option<String>,
    #[serde(default)]
    pub string_tokens: Vec<String>,
    pub numeric_filterable: bool,
    pub numeric_error: Option<String>,
    pub sources: Vec<String>,
    pub details: String,
    pub selected: bool,
    pub matches_constraint: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    pub name: String,
    pub project_id: String,
    pub platform: String,
    pub description: String,
    pub latest_version: String,
    pub downloads: u64,
    #[serde(default)]
    pub categories: Vec<String>,
    pub icon_url: Option<String>,
    pub compatible: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutdatedPackage {
    pub mod_id: String,
    pub current_version: String,
    pub new_version: String,
}

#[derive(Debug, Deserialize)]
pub struct OutdatedResults {
    pub updates: Vec<OutdatedPackage>,
    #[serde(default)]
    pub diagnostics: Vec<ResolutionDiagnostic>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolutionDiagnostic {
    pub package: String,
    pub selected_version: String,
    pub candidate_version: String,
    pub kind: String,
    #[serde(default)]
    pub facts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub id: String,
    pub provider: String,
    pub provider_id: Option<String>,
    pub profile_name: String,
    pub login_name: Option<String>,
    pub authentication_state: String,
    pub avatar_path: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Deserialize)]
pub struct AccountList {
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MicrosoftDeviceSession {
    pub login_session_id: String,
    pub verification_uri: String,
    pub user_code: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct YggdrasilProvider {
    pub id: String,
    pub api_root: String,
    pub allow_insecure_http: bool,
}

#[derive(Debug, Deserialize)]
pub struct YggdrasilProviderList {
    pub providers: Vec<YggdrasilProvider>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaRuntime {
    pub runtime_id: String,
    pub provider: String,
    pub component: String,
    pub platform: String,
    pub version: String,
    pub major: u32,
    pub files: usize,
    pub bytes: u64,
    pub verified: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct JavaRuntimeList {
    pub verification_requested: bool,
    pub runtimes: Vec<JavaRuntime>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallPackageRequirement {
    pub format: String,
    pub name: String,
    pub version: String,
    pub targets: Vec<String>,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub launcher_state: bool,
    pub orbit_content: bool,
    pub optional_files: Vec<InstallPackageOptionalFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallPackageOptionalFile {
    pub path: String,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServerStatus {
    pub running: bool,
    pub state: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct AuditFinding {
    pub packages: String,
    pub rule: String,
    pub reason: String,
    pub risk: u8,
    pub severity: String,
    pub confidence: String,
}

#[derive(Debug, Clone)]
pub struct AuditNotice {
    pub artifact: Option<String>,
    pub scope: String,
    pub kind: String,
    pub detail: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct AuditSummary {
    pub readiness: String,
    pub readiness_message: String,
    pub loader: Option<String>,
    pub runtime_namespace: Option<String>,
    pub capabilities: Vec<String>,
    pub artifacts: usize,
    pub warnings: Vec<AuditNotice>,
    pub coverage_gaps: Vec<AuditNotice>,
    pub findings: Vec<AuditFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: u64,
    pub label: String,
    pub state: TaskState,
    pub phase: Option<ProgressPhase>,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub status_line: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingInteraction {
    pub task_id: u64,
    pub envelope: InteractionEnvelope<Value>,
}

impl TaskView {
    pub fn running(id: u64, label: String) -> Self {
        Self {
            id,
            label,
            state: TaskState::Running,
            phase: None,
            completed: None,
            total: None,
            status_line: tr!("Starting…").into_owned(),
            error_code: None,
            error_message: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Intent {
    LauncherInstances,
    LauncherInstanceDetail,
    Packages,
    PackageVersions {
        package: String,
    },
    PackageOwnership {
        package: String,
    },
    Search,
    Outdated,
    Audit,
    Accounts,
    YggdrasilProviders,
    JavaRuntimes,
    MinecraftVersions,
    LoaderVersions,
    JavaRequirement,
    ServerStatus,
    MicrosoftBegin,
    EulaShow,
    LauncherConfig,
    OrbitConfig,
    MinecraftDirectory,
    LauncherConfigMutated,
    OrbitConfigMutated,
    MinecraftDirectoryMoved,
    Mutated {
        refresh_packages: bool,
    },
    RuntimeMutated,
    InstallPackageInspected {
        source: PathBuf,
    },
    ImportPackageInspected {
        source: PathBuf,
        target_side: String,
        target: PathBuf,
    },
    RuntimeCreatedFromPackage {
        source: PathBuf,
        orbit_content: bool,
        optional_files: Vec<String>,
    },
    PackageRuntimeResolved {
        source: PathBuf,
        target_id: String,
        orbit_content: bool,
        optional_files: Vec<String>,
    },
    PackageOrbitInitialized {
        source: PathBuf,
        target: PathBuf,
        target_id: String,
        optional_files: Vec<String>,
    },
    PackageInstalled {
        target_id: String,
    },
    MigrationSourceExported {
        source_pack: PathBuf,
        source_id: String,
        launcher_args: Vec<String>,
    },
    MigrationBundleComposed {
        source_pack: PathBuf,
        launcher_args: Vec<String>,
    },
    RuntimeCreatedForMigration {
        source_pack: PathBuf,
    },
    MigrationTargetResolved {
        source_pack: PathBuf,
        target_id: String,
    },
    MigrationChecked {
        source_pack: PathBuf,
        target: PathBuf,
        target_id: String,
        target_name: String,
    },
    MigrationExported {
        target: PathBuf,
        target_id: String,
        target_name: String,
    },
    MigrationInstalled {
        target_id: String,
    },
    MigrationRegistered {
        target_id: String,
        target: PathBuf,
    },
    RuntimeConfiguredForInstall {
        target_id: String,
        target: PathBuf,
        sync_orbit: bool,
    },
    RuntimeInstalledAfterUpdate {
        target_id: String,
        target: PathBuf,
        sync_orbit: bool,
    },
    ModpackImported,
    AccountMutated,
    YggdrasilProviderMutated,
    JavaRuntimeMutated,
    ServerMutated,
    Generic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub page: Page,
    pub orbit_binary: PathBuf,
    pub launcher_binary: PathBuf,
    pub selected_instance: Option<String>,
    #[serde(default)]
    pub language: orbit_i18n::LanguageMode,
    #[serde(default)]
    pub theme_mode: crate::theme::ThemeMode,
    #[serde(default)]
    pub accent_theme: crate::theme::AccentTheme,
}
