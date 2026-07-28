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
    pub java_policy: String,
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
    pub icon_path: Option<String>,
    #[serde(default)]
    pub remotes: Vec<String>,
    pub configured_environment: Option<String>,
    pub environment: String,
    pub root: bool,
    pub optional: bool,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub bundled: Vec<BundledPackage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BundledPackage {
    pub mod_id: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct PackageList {
    pub packages: Vec<InstalledPackage>,
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

#[derive(Debug, Clone, Default)]
pub struct AuditSummary {
    pub readiness: String,
    pub artifacts: usize,
    pub warnings: usize,
    pub coverage_gaps: usize,
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
    pub command: String,
    pub state: TaskState,
    pub phase: Option<ProgressPhase>,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub status_line: String,
    pub log: Vec<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingInteraction {
    pub task_id: u64,
    pub envelope: InteractionEnvelope<Value>,
}

impl TaskView {
    pub fn running(id: u64, label: String, command: String) -> Self {
        Self {
            id,
            label,
            command,
            state: TaskState::Running,
            phase: None,
            completed: None,
            total: None,
            status_line: tr!("Starting…").into_owned(),
            log: Vec::new(),
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
    Mutated { refresh_packages: bool },
    RuntimeMutated,
    RuntimeConfiguredForInstall,
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
