//! 全局 Orbit 配置管理。
//!
//! 包含两级配置：
//! - `config.toml` — 全局运行时配置（代理、缓存、并发等）
//! - `instances.toml` — 实例注册表
//!
//! 两个文件同存放于 `orbit/` 数据目录下。

use serde::{Deserialize, Serialize};

use crate::error::OrbitError;

// ---------------------------------------------------------------------------
// 数据目录路径
// ---------------------------------------------------------------------------

/// Orbit 全局数据目录。
///
/// | 平台     | 路径                                      |
/// |----------|-------------------------------------------|
/// | Windows  | `%APPDATA%\orbit\`                        |
/// | Linux    | `~/.orbit/`                                |
/// | macOS    | `~/Library/Application Support/orbit/`     |
pub fn orbit_data_dir() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(base).join("orbit")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("orbit")
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(home).join(".orbit")
    }
}

pub fn config_path() -> std::path::PathBuf {
    orbit_data_dir().join("config.toml")
}

pub fn instances_path() -> std::path::PathBuf {
    orbit_data_dir().join("instances.toml")
}

pub fn default_cache_dir() -> std::path::PathBuf {
    orbit_data_dir().join("cache")
}

// ---------------------------------------------------------------------------
// config.toml — 全局运行时配置
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub default_instance: Option<String>,
    #[serde(default = "default_max_downloads")]
    pub max_concurrent_downloads: usize,
    #[serde(default = "default_language")]
    pub language: String,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self { default_instance: None, max_concurrent_downloads: 8, language: "en".into() }
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
        Self { timeout: 30, max_retries: 3, proxy: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub curseforge_token: Option<String>,
    pub modrinth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
    /// 自定义缓存目录。`None` 时使用 `default_cache_dir()`
    pub dir: Option<String>,
    #[serde(default = "default_eviction_policy")]
    pub eviction_policy: String,
    #[serde(default = "default_max_size")]
    pub max_size_gb: f64,
}

impl CacheConfig {
    /// 解析后的缓存目录——自定义路径优先，否则回退默认路径
    pub fn resolved_dir(&self) -> std::path::PathBuf {
        self.dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(default_cache_dir)
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { enable: true, dir: None, eviction_policy: "size".into(), max_size_gb: 5.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_progress_bar")]
    pub progress_bar: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { color: "auto".into(), progress_bar: "modern".into() }
    }
}

// 辅助默认值函数
fn default_max_downloads() -> usize { 8 }
fn default_language() -> String { "en".into() }
fn default_timeout() -> u64 { 30 }
fn default_max_retries() -> u32 { 3 }
fn default_true() -> bool { true }
fn default_eviction_policy() -> String { "size".into() }
fn default_max_size() -> f64 { 5.0 }
fn default_color() -> String { "auto".into() }
fn default_progress_bar() -> String { "modern".into() }

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            core: CoreConfig::default(),
            network: NetworkConfig::default(),
            auth: AuthConfig::default(),
            cache: CacheConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

impl GlobalConfig {
    /// 分层加载：config.toml → 环境变量覆盖 → 返回
    ///
    /// 优先级：环境变量 > config.toml > 代码默认值
    pub fn load() -> Result<Self, OrbitError> {
        let path = config_path();

        // Layer 1: 文件（如果存在）
        let mut config = if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!("failed to read config.toml: {e}"))
            })?;
            toml::from_str(&content).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!("failed to parse config.toml: {e}"))
            })?
        } else {
            Self::default()
        };

        // Layer 2: 环境变量覆盖
        if let Ok(v) = std::env::var("ORBIT_PROXY") {
            config.network.proxy = Some(v);
        }
        if let Ok(v) = std::env::var("ORBIT_TIMEOUT") {
            if let Ok(n) = v.parse() { config.network.timeout = n; }
        }
        if let Ok(v) = std::env::var("ORBIT_RETRIES") {
            if let Ok(n) = v.parse() { config.network.max_retries = n; }
        }
        if let Ok(v) = std::env::var("ORBIT_LANGUAGE") {
            config.core.language = v;
        }
        if let Ok(v) = std::env::var("ORBIT_CURSEFORGE_TOKEN") {
            config.auth.curseforge_token = Some(v);
        }
        if let Ok(v) = std::env::var("ORBIT_MODRINTH_TOKEN") {
            config.auth.modrinth_token = Some(v);
        }

        Ok(config)
    }

    /// 保存到 config.toml
    pub fn save(&self) -> Result<(), OrbitError> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to serialize config.toml: {e}"))
        })?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// 写入默认配置（首次使用时）
    pub fn init_default() -> Result<Self, OrbitError> {
        let config = Self::default();
        config.save()?;
        Ok(config)
    }
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
    pub fn load() -> Result<Self, OrbitError> {
        let path = instances_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path).map_err(|_| {
            OrbitError::Other(anyhow::anyhow!("failed to read instances.toml"))
        })?;
        let registry: Self = toml::from_str(&content).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to parse instances.toml: {e}"))
        })?;
        Ok(registry)
    }

    pub fn save(&self) -> Result<(), OrbitError> {
        let path = instances_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to serialize instances.toml: {e}"))
        })?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.name == name)
    }

    pub fn default_instance(&self) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.is_default)
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sensible_values() {
        let config = GlobalConfig::default();
        assert_eq!(config.core.max_concurrent_downloads, 8);
        assert_eq!(config.core.language, "en");
        assert_eq!(config.network.timeout, 30);
        assert_eq!(config.network.max_retries, 3);
        assert!(config.cache.enable);
        assert!(config.cache.dir.is_none());
        assert_eq!(config.cache.eviction_policy, "size");
        assert_eq!(config.cache.max_size_gb, 5.0);
        assert_eq!(config.ui.color, "auto");
        assert_eq!(config.ui.progress_bar, "modern");
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
        assert_eq!(config.core.language, "zh-CN");
        assert_eq!(config.network.proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(config.network.timeout, 30);      // 未指定 → 默认值
        assert!(config.cache.enable);                 // 未指定 → 默认值
    }

    #[test]
    fn custom_cache_dir() {
        let toml_str = r#"
[cache]
enable = true
dir = "D:/Games/OrbitCache"
"#;
        let config: GlobalConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.cache.dir.as_deref(), Some("D:/Games/OrbitCache"));
        assert_eq!(
            config.cache.resolved_dir(),
            std::path::PathBuf::from("D:/Games/OrbitCache")
        );
    }

    #[test]
    fn config_roundtrip() {
        let config = GlobalConfig::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: GlobalConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.core.max_concurrent_downloads, 8);
        assert_eq!(deserialized.cache.max_size_gb, 5.0);
    }
}
