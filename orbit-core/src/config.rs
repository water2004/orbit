//! 全局 Orbit 配置管理。
//!
//! 包含两级配置：
//! - `config.toml` — 全局运行时配置（代理、缓存、并发等）
//! - `instances.toml` — 实例注册表
//!
//! 文件位置由 [`crate::runtime::RuntimePaths`] 注入。

use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};
use toml_edit::{DocumentMut, Item, Table, value as toml_value};

use crate::{atomic_io::write_atomic, error::OrbitError};

const GLOBAL_STATE_LOCK_FILE: &str = ".orbit-global-state.lock";

#[derive(Debug)]
struct GlobalStateMutationLock(File);

impl GlobalStateMutationLock {
    fn acquire(state_file: &Path) -> Result<Self, OrbitError> {
        let directory = state_file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(directory)?;
        let lock_path = directory.join(GLOBAL_STATE_LOCK_FILE);
        match std::fs::symlink_metadata(&lock_path) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "global state mutation lock is not a regular file: {}",
                    lock_path.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "another Orbit global-state mutation is already running: {error}"
            ))
        })?;
        Ok(Self(file))
    }
}

impl Drop for GlobalStateMutationLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

// ---------------------------------------------------------------------------
// config.toml — 全局运行时配置
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub core: CoreConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub repository: RepositoryConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RepositoryConfig {
    /// Exact root of the version repository. Each Minecraft/Loader scope owns
    /// independent remote and JAR-analysis databases below this directory.
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub default_instance: Option<String>,
    #[serde(default = "default_max_downloads")]
    pub max_concurrent_downloads: usize,
    #[serde(default)]
    pub language: LanguagePreference,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            default_instance: None,
            max_concurrent_downloads: 8,
            language: LanguagePreference::System,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LanguagePreference {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

impl LanguagePreference {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    fn parse(raw: &str, key: ConfigKey) -> Result<Self, OrbitError> {
        match raw.trim() {
            "system" => Ok(Self::System),
            "en" => Ok(Self::English),
            "zh-CN" => Ok(Self::SimplifiedChinese),
            _ => Err(invalid_value(key, "expected system, en, or zh-CN")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    pub proxy: Option<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            timeout: 30,
            max_retries: 3,
            proxy: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub curseforge_api_key: Option<String>,
    pub modrinth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    /// 自定义缓存目录。`None` 时使用运行环境解析出的缓存目录。
    pub dir: Option<String>,
    /// JAR 内容缓存的硬容量上限，单位为 MiB。
    pub capacity_mib: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            dir: None,
            capacity_mib: 5 * 1024,
        }
    }
}

impl CacheConfig {
    pub fn capacity_bytes(&self) -> Result<u64, OrbitError> {
        self.capacity_mib.checked_mul(1024 * 1024).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "cache.capacity_mib is too large to represent in bytes"
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub color: ColorMode,
    #[serde(default)]
    pub progress_bar: ProgressBarMode,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            color: ColorMode::Auto,
            progress_bar: ProgressBarMode::Modern,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    fn parse(raw: &str, key: ConfigKey) -> Result<Self, OrbitError> {
        match raw.trim() {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err(invalid_value(key, "expected auto, always, or never")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressBarMode {
    #[default]
    Modern,
    Plain,
    Off,
}

impl ProgressBarMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "modern",
            Self::Plain => "plain",
            Self::Off => "off",
        }
    }

    fn parse(raw: &str, key: ConfigKey) -> Result<Self, OrbitError> {
        match raw.trim() {
            "modern" => Ok(Self::Modern),
            "plain" => Ok(Self::Plain),
            "off" => Ok(Self::Off),
            _ => Err(invalid_value(key, "expected modern, plain, or off")),
        }
    }
}

// ---------------------------------------------------------------------------
// Typed global configuration keys
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    CoreDefaultInstance,
    CoreMaxConcurrentDownloads,
    CoreLanguage,
    NetworkTimeout,
    NetworkMaxRetries,
    NetworkProxy,
    AuthCurseforgeApiKey,
    AuthModrinthToken,
    CacheDir,
    CacheCapacityMib,
    RepositoryDir,
    UiColor,
    UiProgressBar,
}

impl ConfigKey {
    pub const ALL: [Self; 13] = [
        Self::CoreDefaultInstance,
        Self::CoreMaxConcurrentDownloads,
        Self::CoreLanguage,
        Self::NetworkTimeout,
        Self::NetworkMaxRetries,
        Self::NetworkProxy,
        Self::AuthCurseforgeApiKey,
        Self::AuthModrinthToken,
        Self::CacheDir,
        Self::CacheCapacityMib,
        Self::RepositoryDir,
        Self::UiColor,
        Self::UiProgressBar,
    ];

    pub fn parse(key: &str) -> Result<Self, OrbitError> {
        let key = match key {
            "core.default-instance" => Self::CoreDefaultInstance,
            "core.max-concurrent-downloads" => Self::CoreMaxConcurrentDownloads,
            "core.language" => Self::CoreLanguage,
            "network.timeout" => Self::NetworkTimeout,
            "network.max-retries" => Self::NetworkMaxRetries,
            "network.proxy" => Self::NetworkProxy,
            "auth.curseforge-api-key" => Self::AuthCurseforgeApiKey,
            "auth.modrinth-token" => Self::AuthModrinthToken,
            "cache.dir" => Self::CacheDir,
            "cache.capacity-mib" => Self::CacheCapacityMib,
            "repository.dir" => Self::RepositoryDir,
            "ui.color" => Self::UiColor,
            "ui.progress-bar" => Self::UiProgressBar,
            _ => {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "unknown global configuration key '{key}'; run 'orbit config list' to see supported keys"
                )));
            }
        };
        Ok(key)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CoreDefaultInstance => "core.default-instance",
            Self::CoreMaxConcurrentDownloads => "core.max-concurrent-downloads",
            Self::CoreLanguage => "core.language",
            Self::NetworkTimeout => "network.timeout",
            Self::NetworkMaxRetries => "network.max-retries",
            Self::NetworkProxy => "network.proxy",
            Self::AuthCurseforgeApiKey => "auth.curseforge-api-key",
            Self::AuthModrinthToken => "auth.modrinth-token",
            Self::CacheDir => "cache.dir",
            Self::CacheCapacityMib => "cache.capacity-mib",
            Self::RepositoryDir => "repository.dir",
            Self::UiColor => "ui.color",
            Self::UiProgressBar => "ui.progress-bar",
        }
    }

    pub const fn value_type(self) -> &'static str {
        match self {
            Self::CoreMaxConcurrentDownloads
            | Self::NetworkTimeout
            | Self::NetworkMaxRetries
            | Self::CacheCapacityMib => "integer",
            _ => "string",
        }
    }

    pub const fn is_sensitive(self) -> bool {
        matches!(self, Self::AuthCurseforgeApiKey | Self::AuthModrinthToken)
    }

    pub fn get(self, config: &GlobalConfig) -> ConfigValue {
        match self {
            Self::CoreDefaultInstance => optional_text(&config.core.default_instance),
            Self::CoreMaxConcurrentDownloads => {
                ConfigValue::Integer(config.core.max_concurrent_downloads as u64)
            }
            Self::CoreLanguage => ConfigValue::Text(config.core.language.as_str().to_string()),
            Self::NetworkTimeout => ConfigValue::Integer(config.network.timeout),
            Self::NetworkMaxRetries => ConfigValue::Integer(u64::from(config.network.max_retries)),
            Self::NetworkProxy => optional_text(&config.network.proxy),
            Self::AuthCurseforgeApiKey => optional_text(&config.auth.curseforge_api_key),
            Self::AuthModrinthToken => optional_text(&config.auth.modrinth_token),
            Self::CacheDir => optional_text(&config.cache.dir),
            Self::CacheCapacityMib => ConfigValue::Integer(config.cache.capacity_mib),
            Self::RepositoryDir => optional_text(&config.repository.dir),
            Self::UiColor => ConfigValue::Text(config.ui.color.as_str().to_string()),
            Self::UiProgressBar => ConfigValue::Text(config.ui.progress_bar.as_str().to_string()),
        }
    }

    pub fn set(self, config: &mut GlobalConfig, raw: &str) -> Result<(), OrbitError> {
        match self {
            Self::CoreDefaultInstance => {
                config.core.default_instance = Some(nonempty(raw, self)?);
            }
            Self::CoreMaxConcurrentDownloads => {
                let value = parse_toml_u64(raw, self)?;
                if value == 0 {
                    return Err(invalid_value(self, "must be greater than zero"));
                }
                config.core.max_concurrent_downloads = usize::try_from(value)
                    .map_err(|_| invalid_value(self, "does not fit this platform's usize"))?;
            }
            Self::CoreLanguage => config.core.language = LanguagePreference::parse(raw, self)?,
            Self::NetworkTimeout => {
                let value = parse_toml_u64(raw, self)?;
                if value == 0 {
                    return Err(invalid_value(self, "must be greater than zero"));
                }
                config.network.timeout = value;
            }
            Self::NetworkMaxRetries => {
                config.network.max_retries = raw
                    .parse()
                    .map_err(|_| invalid_value(self, "expected an integer from 0 to 4294967295"))?;
            }
            Self::NetworkProxy => config.network.proxy = Some(nonempty(raw, self)?),
            Self::AuthCurseforgeApiKey => {
                config.auth.curseforge_api_key = Some(nonempty(raw, self)?);
            }
            Self::AuthModrinthToken => {
                config.auth.modrinth_token = Some(nonempty(raw, self)?);
            }
            Self::CacheDir => config.cache.dir = Some(nonempty(raw, self)?),
            Self::CacheCapacityMib => {
                let capacity_mib = parse_toml_u64(raw, self)?;
                config.cache.capacity_mib = capacity_mib;
                config.cache.capacity_bytes()?;
            }
            Self::RepositoryDir => config.repository.dir = Some(nonempty(raw, self)?),
            Self::UiColor => config.ui.color = ColorMode::parse(raw, self)?,
            Self::UiProgressBar => config.ui.progress_bar = ProgressBarMode::parse(raw, self)?,
        }
        Ok(())
    }

    /// Remove an optional value or restore a required value to its schema
    /// default.
    pub fn unset(self, config: &mut GlobalConfig) {
        let defaults = GlobalConfig::default();
        match self {
            Self::CoreDefaultInstance => config.core.default_instance = None,
            Self::CoreMaxConcurrentDownloads => {
                config.core.max_concurrent_downloads = defaults.core.max_concurrent_downloads;
            }
            Self::CoreLanguage => config.core.language = defaults.core.language,
            Self::NetworkTimeout => config.network.timeout = defaults.network.timeout,
            Self::NetworkMaxRetries => {
                config.network.max_retries = defaults.network.max_retries;
            }
            Self::NetworkProxy => config.network.proxy = None,
            Self::AuthCurseforgeApiKey => config.auth.curseforge_api_key = None,
            Self::AuthModrinthToken => config.auth.modrinth_token = None,
            Self::CacheDir => config.cache.dir = None,
            Self::CacheCapacityMib => {
                config.cache.capacity_mib = defaults.cache.capacity_mib;
            }
            Self::RepositoryDir => config.repository.dir = None,
            Self::UiColor => config.ui.color = defaults.ui.color,
            Self::UiProgressBar => config.ui.progress_bar = defaults.ui.progress_bar,
        }
    }

    const fn toml_path(self) -> (&'static str, &'static str) {
        match self {
            Self::CoreDefaultInstance => ("core", "default_instance"),
            Self::CoreMaxConcurrentDownloads => ("core", "max_concurrent_downloads"),
            Self::CoreLanguage => ("core", "language"),
            Self::NetworkTimeout => ("network", "timeout"),
            Self::NetworkMaxRetries => ("network", "max_retries"),
            Self::NetworkProxy => ("network", "proxy"),
            Self::AuthCurseforgeApiKey => ("auth", "curseforge_api_key"),
            Self::AuthModrinthToken => ("auth", "modrinth_token"),
            Self::CacheDir => ("cache", "dir"),
            Self::CacheCapacityMib => ("cache", "capacity_mib"),
            Self::RepositoryDir => ("repository", "dir"),
            Self::UiColor => ("ui", "color"),
            Self::UiProgressBar => ("ui", "progress_bar"),
        }
    }

    fn toml_item(self, config: &GlobalConfig) -> Option<Item> {
        match self.get(config) {
            ConfigValue::Absent => None,
            ConfigValue::Text(value) => Some(toml_value(value)),
            ConfigValue::Integer(value) => Some(toml_value(value as i64)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    Absent,
    Text(String),
    Integer(u64),
}

fn optional_text(value: &Option<String>) -> ConfigValue {
    value
        .as_ref()
        .map(|value| ConfigValue::Text(value.clone()))
        .unwrap_or(ConfigValue::Absent)
}

fn nonempty(raw: &str, key: ConfigKey) -> Result<String, OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        Err(invalid_value(key, "must not be empty"))
    } else {
        Ok(value.to_string())
    }
}

fn parse_toml_u64(raw: &str, key: ConfigKey) -> Result<u64, OrbitError> {
    let value = raw
        .parse()
        .map_err(|_| invalid_value(key, "expected a non-negative integer"))?;
    if value > i64::MAX as u64 {
        Err(invalid_value(key, "exceeds TOML's integer range"))
    } else {
        Ok(value)
    }
}

fn invalid_value(key: ConfigKey, reason: &str) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(
        "invalid value for '{}': {reason}",
        key.as_str()
    ))
}

// 辅助默认值函数
fn default_max_downloads() -> usize {
    8
}
fn default_timeout() -> u64 {
    30
}
fn default_max_retries() -> u32 {
    3
}

impl GlobalConfig {
    /// 分层加载：config.toml → 环境变量覆盖 → 返回
    ///
    /// 优先级：环境变量 > config.toml > 代码默认值
    pub fn load(path: &Path) -> Result<Self, OrbitError> {
        let mut config = Self::load_stored(path)?;
        config.apply_environment()?;
        config.validate()?;
        Ok(config)
    }

    /// Load only the values persisted in `config.toml`.
    ///
    /// Mutation commands must use this entry point so process environment
    /// overrides, especially credentials, are never written back to disk.
    pub fn load_stored(path: &Path) -> Result<Self, OrbitError> {
        if path.exists() {
            return Self::read_stored(path);
        }

        let _lock = GlobalStateMutationLock::acquire(path)?;
        Self::load_or_create_stored_unlocked(path)
    }

    fn read_stored(path: &Path) -> Result<Self, OrbitError> {
        let content = std::fs::read_to_string(path).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!("failed to read config.toml: {error}"))
        })?;
        let config = toml::from_str(&content).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!("failed to parse config.toml: {error}"))
        })?;
        Self::validate(&config)?;
        Ok(config)
    }

    fn load_or_create_stored_unlocked(path: &Path) -> Result<Self, OrbitError> {
        if path.exists() {
            return Self::read_stored(path);
        }
        let config = Self::default();
        config.save_unlocked(path)?;
        Ok(config)
    }

    fn apply_environment(&mut self) -> Result<(), OrbitError> {
        if let Ok(v) = std::env::var("ORBIT_PROXY") {
            self.network.proxy = Some(v);
        }
        if let Ok(value) = std::env::var("ORBIT_TIMEOUT") {
            self.network.timeout = value.parse().map_err(|_| {
                OrbitError::Other(anyhow::anyhow!(
                    "invalid ORBIT_TIMEOUT value '{value}': expected a positive integer"
                ))
            })?;
        }
        if let Ok(value) = std::env::var("ORBIT_RETRIES") {
            self.network.max_retries = value.parse().map_err(|_| {
                OrbitError::Other(anyhow::anyhow!(
                    "invalid ORBIT_RETRIES value '{value}': expected an integer from 0 to {}",
                    u32::MAX
                ))
            })?;
        }
        if let Ok(v) = std::env::var("ORBIT_LANGUAGE") {
            self.core.language = LanguagePreference::parse(&v, ConfigKey::CoreLanguage)?;
        }
        if let Ok(v) = std::env::var("ORBIT_CURSEFORGE_API_KEY") {
            self.auth.curseforge_api_key = Some(v);
        }
        if let Ok(v) = std::env::var("ORBIT_MODRINTH_TOKEN") {
            self.auth.modrinth_token = Some(v);
        }
        Ok(())
    }

    /// Validate values that can otherwise create a stalled or ambiguous
    /// runtime. Deserialization alone cannot express these numeric and URL
    /// invariants.
    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.core.max_concurrent_downloads == 0 {
            return Err(invalid_value(
                ConfigKey::CoreMaxConcurrentDownloads,
                "must be greater than zero",
            ));
        }
        if self.network.timeout == 0 {
            return Err(invalid_value(
                ConfigKey::NetworkTimeout,
                "must be greater than zero",
            ));
        }
        if let Some(proxy) = self.network.proxy.as_deref() {
            if proxy.trim().is_empty() {
                return Err(invalid_value(ConfigKey::NetworkProxy, "must not be empty"));
            }
            reqwest::Proxy::all(proxy).map_err(|error| {
                invalid_value(
                    ConfigKey::NetworkProxy,
                    &format!("expected a valid proxy URL: {error}"),
                )
            })?;
        }
        for (key, value) in [
            (
                ConfigKey::AuthCurseforgeApiKey,
                self.auth.curseforge_api_key.as_deref(),
            ),
            (
                ConfigKey::AuthModrinthToken,
                self.auth.modrinth_token.as_deref(),
            ),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(invalid_value(key, "must not be empty when configured"));
            }
        }
        self.cache.capacity_bytes()?;
        Ok(())
    }

    /// 保存到 config.toml
    pub fn save(&self, path: &Path) -> Result<(), OrbitError> {
        let _lock = GlobalStateMutationLock::acquire(path)?;
        self.save_unlocked(path)
    }

    fn save_unlocked(&self, path: &Path) -> Result<(), OrbitError> {
        self.validate()?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to serialize config.toml: {e}"))
        })?;
        write_atomic(path, content.as_bytes())
    }

    /// 写入默认配置（首次使用时）
    pub fn init_default(path: &Path) -> Result<Self, OrbitError> {
        let config = Self::default();
        config.save(path)?;
        Ok(config)
    }
}

/// Persist exactly one typed field while preserving unrelated TOML comments
/// and formatting.
pub fn persist_config_field(
    path: &Path,
    key: ConfigKey,
    config: &GlobalConfig,
) -> Result<(), OrbitError> {
    let _lock = GlobalStateMutationLock::acquire(path)?;
    let rendered = render_config_field(path, key, config)?;
    write_atomic(path, rendered.as_bytes())
}

fn render_config_field(
    path: &Path,
    key: ConfigKey,
    config: &GlobalConfig,
) -> Result<String, OrbitError> {
    let content = std::fs::read_to_string(path).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!("failed to read config.toml: {error}"))
    })?;
    let mut document = content.parse::<DocumentMut>().map_err(|error| {
        OrbitError::Other(anyhow::anyhow!("failed to edit config.toml: {error}"))
    })?;
    let (section, field) = key.toml_path();
    if !document.contains_key(section) {
        document[section] = Item::Table(Table::new());
    }
    let table = document[section].as_table_mut().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "config.toml field '{section}' must be a table"
        ))
    })?;
    match key.toml_item(config) {
        Some(mut item) => {
            if let Some(decor) = table
                .get(field)
                .and_then(Item::as_value)
                .map(|value| value.decor().clone())
                && let Some(value) = item.as_value_mut()
            {
                *value.decor_mut() = decor;
            }
            table.insert(field, item);
        }
        None => {
            table.remove(field);
        }
    }

    let rendered = document.to_string();
    let validated: GlobalConfig = toml::from_str(&rendered).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "updated config.toml failed schema validation: {error}"
        ))
    })?;
    validated.validate()?;
    Ok(rendered)
}

// ---------------------------------------------------------------------------
// instances.toml — 实例注册表
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceEntry {
    pub name: String,
    pub path: String,
    pub mc_version: String,
    pub modloader: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstancesRegistry {
    pub instances: Vec<InstanceEntry>,
}

impl InstancesRegistry {
    pub fn load(path: &Path) -> Result<Self, OrbitError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|_| OrbitError::Other(anyhow::anyhow!("failed to read instances.toml")))?;
        let registry: Self = toml::from_str(&content).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to parse instances.toml: {e}"))
        })?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn save(&self, path: &Path) -> Result<(), OrbitError> {
        let _lock = GlobalStateMutationLock::acquire(path)?;
        self.save_unlocked(path)
    }

    fn save_unlocked(&self, path: &Path) -> Result<(), OrbitError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(path, &self.serialize()?)
    }

    fn serialize(&self) -> Result<Vec<u8>, OrbitError> {
        self.validate()?;
        let mut persisted = self.clone();
        persisted.instances.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.path.cmp(&right.path))
        });
        toml::to_string_pretty(&persisted)
            .map(String::into_bytes)
            .map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "failed to serialize instances.toml: {error}"
                ))
            })
    }

    pub fn find(&self, name: &str) -> Option<&InstanceEntry> {
        self.instances
            .iter()
            .find(|instance| instance.name.eq_ignore_ascii_case(name))
    }

    pub fn default_instance(&self) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.is_default)
    }

    pub fn upsert(&mut self, mut entry: InstanceEntry) -> Result<(), OrbitError> {
        validate_instance_entry(&entry)?;
        if self.instances.iter().any(|existing| {
            !existing.name.eq_ignore_ascii_case(&entry.name)
                && paths_equal(&existing.path, &entry.path)
        }) {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "another registered instance already uses path '{}'",
                entry.path
            )));
        }
        if let Some(existing) = self
            .instances
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(&entry.name))
        {
            entry.name = existing.name.clone();
            entry.is_default = existing.is_default || entry.is_default;
            *existing = entry;
        } else {
            self.instances.push(entry);
        }
        self.instances
            .sort_by_key(|entry| entry.name.to_lowercase());
        self.validate()
    }

    pub fn set_default(&mut self, name: &str) -> Option<InstanceEntry> {
        let selected = self.find(name)?.clone();
        for instance in &mut self.instances {
            instance.is_default = instance.name.eq_ignore_ascii_case(name);
        }
        Some(selected)
    }

    pub fn remove(&mut self, name: &str) -> Option<InstanceEntry> {
        let index = self
            .instances
            .iter()
            .position(|instance| instance.name.eq_ignore_ascii_case(name))?;
        Some(self.instances.remove(index))
    }

    fn validate(&self) -> Result<(), OrbitError> {
        for (index, entry) in self.instances.iter().enumerate() {
            validate_instance_entry(entry)?;
            if self.instances[..index].iter().any(|other| {
                other.name.eq_ignore_ascii_case(&entry.name)
                    || paths_equal(&other.path, &entry.path)
            }) {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "instances.toml contains a duplicate name or path for '{}'",
                    entry.name
                )));
            }
        }
        if self
            .instances
            .iter()
            .filter(|entry| entry.is_default)
            .count()
            > 1
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "instances.toml contains more than one default instance"
            )));
        }
        Ok(())
    }
}

fn validate_instance_entry(entry: &InstanceEntry) -> Result<(), OrbitError> {
    if entry.name.trim().is_empty()
        || entry.name.trim() != entry.name
        || entry.name.chars().any(char::is_control)
        || entry.path.trim().is_empty()
        || entry.path.chars().any(char::is_control)
        || entry.mc_version.trim().is_empty()
        || entry.mc_version.trim() != entry.mc_version
        || !matches!(
            entry.modloader.as_str(),
            "vanilla" | "fabric" | "quilt" | "forge" | "neoforge"
        )
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "invalid registered instance '{}'",
            entry.name
        )));
    }
    Ok(())
}

fn paths_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

pub fn register_instance(
    paths: &crate::runtime::RuntimePaths,
    entry: InstanceEntry,
) -> Result<(), OrbitError> {
    let _lock = GlobalStateMutationLock::acquire(paths.config_file())?;
    let mut registry = InstancesRegistry::load(paths.instances_file())?;
    registry.upsert(entry)?;
    registry.save_unlocked(paths.instances_file())
}

/// Register a pre-existing Orbit workspace without discovering or rewriting it.
///
/// This is used when another transactional command (for example migration
/// export) created the workspace. Both state files must already exist and
/// describe the same platform; registration never guesses missing metadata.
pub fn register_existing_instance(
    paths: &crate::runtime::RuntimePaths,
    name: &str,
    instance_dir: &Path,
) -> Result<InstanceEntry, OrbitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "instance name must not be empty"
        )));
    }
    let instance_dir = instance_dir.canonicalize().map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "cannot resolve Orbit instance '{}': {error}",
            instance_dir.display()
        ))
    })?;
    if !instance_dir.is_dir() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "Orbit instance is not a directory: {}",
            instance_dir.display()
        )));
    }
    let manifest = crate::workspace::ManifestFile::open(&instance_dir)?;
    let lockfile = crate::workspace::Lockfile::open(&instance_dir)?;
    if lockfile.inner.meta.mc_version != manifest.inner.project.mc_version
        || lockfile.inner.meta.modloader != manifest.inner.project.modloader
        || lockfile.inner.meta.modloader_version != manifest.inner.project.modloader_version
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "orbit.toml and orbit.lock describe different Minecraft or Loader platforms"
        )));
    }
    let entry = InstanceEntry {
        name: name.to_string(),
        path: instance_dir.to_string_lossy().into_owned(),
        mc_version: manifest.inner.project.mc_version,
        modloader: manifest.inner.project.modloader,
        is_default: false,
    };
    register_instance(paths, entry.clone())?;
    Ok(entry)
}

pub fn set_default_instance(
    paths: &crate::runtime::RuntimePaths,
    name: &str,
) -> Result<InstanceEntry, OrbitError> {
    let _lock = GlobalStateMutationLock::acquire(paths.config_file())?;
    let mut registry = InstancesRegistry::load(paths.instances_file())?;
    let selected = registry.set_default(name).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "instance '{name}' not found; run 'orbit instances list' to see registered instances"
        ))
    })?;
    let mut config = GlobalConfig::load_or_create_stored_unlocked(paths.config_file())?;
    ConfigKey::CoreDefaultInstance.set(&mut config, &selected.name)?;
    save_registry_and_config_field_unlocked(
        paths,
        &registry,
        ConfigKey::CoreDefaultInstance,
        &config,
    )?;
    Ok(selected)
}

pub fn clear_default_instance(paths: &crate::runtime::RuntimePaths) -> Result<(), OrbitError> {
    let _lock = GlobalStateMutationLock::acquire(paths.config_file())?;
    let mut registry = InstancesRegistry::load(paths.instances_file())?;
    for instance in &mut registry.instances {
        instance.is_default = false;
    }
    let mut config = GlobalConfig::load_or_create_stored_unlocked(paths.config_file())?;
    ConfigKey::CoreDefaultInstance.unset(&mut config);
    save_registry_and_config_field_unlocked(
        paths,
        &registry,
        ConfigKey::CoreDefaultInstance,
        &config,
    )
}

pub fn remove_instance(
    paths: &crate::runtime::RuntimePaths,
    name: &str,
) -> Result<InstanceEntry, OrbitError> {
    let _lock = GlobalStateMutationLock::acquire(paths.config_file())?;
    let mut registry = InstancesRegistry::load(paths.instances_file())?;
    let removed = registry
        .remove(name)
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("instance '{name}' not found")))?;
    let mut config = GlobalConfig::load_or_create_stored_unlocked(paths.config_file())?;
    if config
        .core
        .default_instance
        .as_deref()
        .is_some_and(|default| default.eq_ignore_ascii_case(&removed.name))
    {
        ConfigKey::CoreDefaultInstance.unset(&mut config);
        save_registry_and_config_field_unlocked(
            paths,
            &registry,
            ConfigKey::CoreDefaultInstance,
            &config,
        )?;
    } else {
        registry.save_unlocked(paths.instances_file())?;
    }
    Ok(removed)
}

fn save_registry_and_config_field_unlocked(
    paths: &crate::runtime::RuntimePaths,
    registry: &InstancesRegistry,
    key: ConfigKey,
    config: &GlobalConfig,
) -> Result<(), OrbitError> {
    let registry_document = registry.serialize()?;
    let config_document = render_config_field(paths.config_file(), key, config)?;
    let original_registry = read_optional_state(paths.instances_file())?;
    let original_config = read_optional_state(paths.config_file())?;
    let result = (|| {
        write_atomic(paths.instances_file(), &registry_document)?;
        write_atomic(paths.config_file(), config_document.as_bytes())?;
        Ok(())
    })();
    let Err(error) = result else {
        return Ok(());
    };

    let mut rollback_failures = Vec::new();
    if let Err(rollback) =
        restore_optional_state(paths.instances_file(), original_registry.as_deref())
    {
        rollback_failures.push(format!("instances.toml: {rollback}"));
    }
    if let Err(rollback) = restore_optional_state(paths.config_file(), original_config.as_deref()) {
        rollback_failures.push(format!("config.toml: {rollback}"));
    }
    if rollback_failures.is_empty() {
        Err(error)
    } else {
        Err(OrbitError::Other(anyhow::anyhow!(
            "global state update failed: {error}; rollback also failed: {}",
            rollback_failures.join("; ")
        )))
    }
}

fn read_optional_state(path: &Path) -> Result<Option<Vec<u8>>, OrbitError> {
    match std::fs::read(path) {
        Ok(document) => Ok(Some(document)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn restore_optional_state(path: &Path, document: Option<&[u8]>) -> Result<(), OrbitError> {
    if read_optional_state(path)?.as_deref() == document {
        return Ok(());
    }
    if let Some(document) = document {
        write_atomic(path, document)
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_paths(root: &Path) -> crate::runtime::RuntimePaths {
        crate::runtime::RuntimePaths::resolve(&crate::runtime::RuntimePathOptions {
            config_file: Some(root.join("global").join("config.toml")),
            cache_dir: Some(root.join("cache")),
            ..Default::default()
        })
        .unwrap()
    }

    fn write_workspace(root: &Path, lock_loader_version: &str) {
        let manifest = crate::manifest::OrbitManifest {
            project: crate::manifest::ProjectMeta {
                name: "managed".to_string(),
                mc_version: "1.21.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16.10".to_string(),
                description: None,
                authors: None,
                version: None,
            },
            platform: crate::manifest::PlatformSnapshot {
                minecraft_jar: crate::manifest::PlatformArtifact {
                    path: "minecraft.jar".to_string(),
                    sha256: "minecraft-hash".to_string(),
                },
                loader_jar: crate::manifest::PlatformArtifact {
                    path: "fabric-loader.jar".to_string(),
                    sha256: "loader-hash".to_string(),
                },
                runtime_jars: Vec::new(),
                physical_environment: crate::metadata::Environment::Client,
            },
            resolver: crate::manifest::ResolverConfig::default(),
            packages: Default::default(),
            groups: Default::default(),
        };
        crate::workspace::ManifestFile::new(root, manifest)
            .save()
            .unwrap();
        crate::workspace::Lockfile::new(
            root,
            crate::lockfile::OrbitLockfile {
                meta: crate::lockfile::LockMeta {
                    mc_version: "1.21.1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: lock_loader_version.to_string(),
                },
                packages: Vec::new(),
            },
        )
        .save()
        .unwrap();
    }

    #[test]
    fn default_config_has_sensible_values() {
        let config = GlobalConfig::default();
        assert_eq!(config.core.max_concurrent_downloads, 8);
        assert_eq!(config.core.language, LanguagePreference::System);
        assert_eq!(config.network.timeout, 30);
        assert_eq!(config.network.max_retries, 3);
        assert!(config.cache.dir.is_none());
        assert!(config.repository.dir.is_none());
        assert_eq!(config.cache.capacity_mib, 5 * 1024);
        assert_eq!(
            config.cache.capacity_bytes().unwrap(),
            5 * 1024 * 1024 * 1024
        );
        assert_eq!(config.ui.color, ColorMode::Auto);
        assert_eq!(config.ui.progress_bar, ProgressBarMode::Modern);
    }

    #[test]
    fn parse_minimal_config() {
        let toml_str = r#"
[core]
language = "zh-CN"

[network]
proxy = "http://127.0.0.1:7890"
"#;
        let config: GlobalConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.core.language, LanguagePreference::SimplifiedChinese);
        assert_eq!(
            config.network.proxy.as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(config.network.timeout, 30); // 未指定 → 默认值
        assert_eq!(config.cache.capacity_mib, 5 * 1024);
    }

    #[test]
    fn custom_cache_dir() {
        let toml_str = r#"
[cache]
dir = "D:/Games/OrbitCache"
capacity_mib = 2048
"#;
        let config: GlobalConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.cache.dir.as_deref(), Some("D:/Games/OrbitCache"));
        assert_eq!(config.cache.capacity_mib, 2048);
    }

    #[test]
    fn custom_version_repository_dir() {
        let config: GlobalConfig = toml::from_str(
            r#"
[repository]
dir = "D:/Games/OrbitRepository"
"#,
        )
        .unwrap();
        assert_eq!(
            config.repository.dir.as_deref(),
            Some("D:/Games/OrbitRepository")
        );
    }

    #[test]
    fn obsolete_cache_schema_is_rejected() {
        let error = toml::from_str::<GlobalConfig>(
            r#"
[cache]
capacity_mib = 5120
eviction_policy = "size"
"#,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unknown field `eviction_policy`")
        );
    }

    #[test]
    fn explicit_cache_section_requires_a_capacity() {
        let error = toml::from_str::<GlobalConfig>(
            r#"
[cache]
dir = "D:/Games/OrbitCache"
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("missing field `capacity_mib`"));
    }

    #[test]
    fn config_roundtrip() {
        let config = GlobalConfig::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("language = \"system\""));
        assert!(serialized.contains("color = \"auto\""));
        assert!(serialized.contains("progress_bar = \"modern\""));
        let deserialized: GlobalConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.core.max_concurrent_downloads, 8);
        assert_eq!(deserialized.cache.capacity_mib, 5 * 1024);
    }

    #[test]
    fn typed_config_keys_validate_values_and_reset_defaults() {
        let mut config = GlobalConfig::default();

        ConfigKey::CacheCapacityMib
            .set(&mut config, "2048")
            .unwrap();
        ConfigKey::UiProgressBar.set(&mut config, "plain").unwrap();
        ConfigKey::NetworkProxy
            .set(&mut config, "http://127.0.0.1:7890")
            .unwrap();
        assert_eq!(
            ConfigKey::CacheCapacityMib.get(&config),
            ConfigValue::Integer(2048)
        );
        assert_eq!(
            ConfigKey::NetworkProxy.get(&config),
            ConfigValue::Text("http://127.0.0.1:7890".to_string())
        );

        assert!(
            ConfigKey::CoreMaxConcurrentDownloads
                .set(&mut config, "0")
                .is_err()
        );
        assert!(ConfigKey::UiProgressBar.set(&mut config, "fast").is_err());
        assert!(ConfigKey::UiColor.set(&mut config, "sometimes").is_err());
        assert!(ConfigKey::CoreLanguage.set(&mut config, "fr").is_err());
        assert!(
            ConfigKey::CacheCapacityMib
                .set(&mut config, "9223372036854775808")
                .is_err()
        );

        ConfigKey::CacheCapacityMib.unset(&mut config);
        ConfigKey::NetworkProxy.unset(&mut config);
        assert_eq!(config.cache.capacity_mib, 5 * 1024);
        assert_eq!(ConfigKey::NetworkProxy.get(&config), ConfigValue::Absent);
    }

    #[test]
    fn stored_presentation_values_are_schema_checked() {
        for invalid in [
            "[core]\nlanguage = \"fr\"\n",
            "[ui]\ncolor = \"sometimes\"\n",
            "[ui]\nprogress_bar = \"fast\"\n",
        ] {
            assert!(
                toml::from_str::<GlobalConfig>(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn runtime_validation_rejects_values_that_would_stall_network_work() {
        let mut config = GlobalConfig::default();
        config.core.max_concurrent_downloads = 0;
        assert!(config.validate().is_err());

        let mut config = GlobalConfig::default();
        config.network.timeout = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn runtime_validation_rejects_an_invalid_proxy() {
        let mut config = GlobalConfig::default();
        config.network.proxy = Some("://invalid".to_string());
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("network.proxy"));
    }

    #[test]
    fn runtime_validation_rejects_blank_credentials() {
        let mut config = GlobalConfig::default();
        config.auth.modrinth_token = Some("  ".to_string());
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("auth.modrinth-token"));
    }

    #[test]
    fn config_key_names_are_canonical_and_do_not_accept_toml_spelling() {
        assert_eq!(
            ConfigKey::parse("cache.capacity-mib").unwrap(),
            ConfigKey::CacheCapacityMib
        );
        assert!(ConfigKey::parse("cache.capacity_mib").is_err());
    }

    #[test]
    fn field_updates_preserve_unrelated_comments_and_formatting() {
        let directory =
            std::env::temp_dir().join(format!("orbit-config-edit-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        std::fs::write(
            &path,
            "# keep this comment\n[cache]\ncapacity_mib = 5120 # keep inline\n",
        )
        .unwrap();
        let mut config = GlobalConfig::load_stored(&path).unwrap();
        ConfigKey::CacheCapacityMib
            .set(&mut config, "2048")
            .unwrap();

        persist_config_field(&path, ConfigKey::CacheCapacityMib, &config).unwrap();

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("# keep this comment"));
        assert!(updated.contains("capacity_mib = 2048 # keep inline"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn config_can_use_a_bare_relative_filename() {
        let path = std::path::PathBuf::from(format!(
            "orbit-config-relative-test-{}.toml",
            std::process::id()
        ));

        GlobalConfig::default().save(&path).unwrap();

        assert!(path.is_file());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(GLOBAL_STATE_LOCK_FILE).unwrap();
    }

    fn instance(name: &str, is_default: bool) -> InstanceEntry {
        InstanceEntry {
            name: name.to_string(),
            path: format!("/instances/{name}"),
            mc_version: "1.21.1".to_string(),
            modloader: "fabric".to_string(),
            is_default,
        }
    }

    #[test]
    fn registry_upsert_replaces_metadata_without_losing_default() {
        let mut registry = InstancesRegistry {
            instances: vec![instance("alpha", true)],
        };
        let mut updated = instance("alpha", false);
        updated.mc_version = "1.21.5".to_string();

        registry.upsert(updated).unwrap();

        assert_eq!(registry.instances.len(), 1);
        assert_eq!(registry.instances[0].mc_version, "1.21.5");
        assert!(registry.instances[0].is_default);
    }

    #[test]
    fn registry_default_is_unique_and_remove_returns_entry() {
        let mut registry = InstancesRegistry {
            instances: vec![instance("alpha", true), instance("beta", false)],
        };

        let selected = registry.set_default("beta").unwrap();

        assert_eq!(selected.name, "beta");
        assert!(!registry.find("alpha").unwrap().is_default);
        assert!(registry.find("beta").unwrap().is_default);
        assert_eq!(registry.remove("beta").unwrap().name, "beta");
        assert!(registry.default_instance().is_none());
    }

    #[test]
    fn global_state_mutations_are_exclusive() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let first = GlobalStateMutationLock::acquire(&config).unwrap();

        let error = GlobalStateMutationLock::acquire(&config).unwrap_err();

        assert!(error.to_string().contains("already running"));
        drop(first);
        GlobalStateMutationLock::acquire(&config).unwrap();
    }

    #[test]
    fn default_instance_updates_both_global_documents() {
        let directory = tempfile::tempdir().unwrap();
        let paths = runtime_paths(directory.path());
        GlobalConfig::default().save(paths.config_file()).unwrap();
        InstancesRegistry {
            instances: vec![instance("alpha", true), instance("beta", false)],
        }
        .save(paths.instances_file())
        .unwrap();

        set_default_instance(&paths, "beta").unwrap();

        let registry = InstancesRegistry::load(paths.instances_file()).unwrap();
        let config = GlobalConfig::load_stored(paths.config_file()).unwrap();
        assert_eq!(registry.default_instance().unwrap().name, "beta");
        assert_eq!(config.core.default_instance.as_deref(), Some("beta"));

        clear_default_instance(&paths).unwrap();

        let registry = InstancesRegistry::load(paths.instances_file()).unwrap();
        let config = GlobalConfig::load_stored(paths.config_file()).unwrap();
        assert!(registry.default_instance().is_none());
        assert!(config.core.default_instance.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn failed_default_update_rolls_back_the_registry() {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = tempfile::tempdir().unwrap();
        let paths = runtime_paths(directory.path());
        GlobalConfig::default().save(paths.config_file()).unwrap();
        InstancesRegistry {
            instances: vec![instance("alpha", true), instance("beta", false)],
        }
        .save(paths.instances_file())
        .unwrap();
        let registry_before = std::fs::read(paths.instances_file()).unwrap();
        let config_before = std::fs::read(paths.config_file()).unwrap();
        let _locked_config = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(paths.config_file())
            .unwrap();

        let error = set_default_instance(&paths, "beta").unwrap_err();

        assert!(matches!(error, OrbitError::Io(_)));
        assert_eq!(
            std::fs::read(paths.instances_file()).unwrap(),
            registry_before
        );
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), config_before);
    }

    #[test]
    fn existing_workspace_registration_requires_matching_state_files() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("instance");
        std::fs::create_dir_all(&workspace).unwrap();
        let paths = runtime_paths(directory.path());

        write_workspace(&workspace, "0.16.10");
        let registered = register_existing_instance(&paths, "fabric-1.21.1", &workspace).unwrap();

        assert_eq!(registered.name, "fabric-1.21.1");
        assert_eq!(registered.mc_version, "1.21.1");
        assert_eq!(registered.modloader, "fabric");
        assert_eq!(
            Path::new(&registered.path).canonicalize().unwrap(),
            workspace.canonicalize().unwrap()
        );
        assert_eq!(
            InstancesRegistry::load(paths.instances_file())
                .unwrap()
                .instances
                .len(),
            1
        );

        write_workspace(&workspace, "0.16.11");
        let error = register_existing_instance(&paths, "invalid", &workspace).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("describe different Minecraft or Loader platforms")
        );
        assert!(
            InstancesRegistry::load(paths.instances_file())
                .unwrap()
                .find("invalid")
                .is_none()
        );
    }
}
