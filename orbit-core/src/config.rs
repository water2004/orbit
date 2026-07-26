//! 全局 Orbit 配置管理。
//!
//! 包含两级配置：
//! - `config.toml` — 全局运行时配置（代理、缓存、并发等）
//! - `instances.toml` — 实例注册表
//!
//! 文件位置由 [`crate::runtime::RuntimePaths`] 注入。

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::OrbitError;

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
        Self {
            default_instance: None,
            max_concurrent_downloads: 8,
            language: "en".into(),
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
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_progress_bar")]
    pub progress_bar: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            color: "auto".into(),
            progress_bar: "modern".into(),
        }
    }
}

// 辅助默认值函数
fn default_max_downloads() -> usize {
    8
}
fn default_language() -> String {
    "en".into()
}
fn default_timeout() -> u64 {
    30
}
fn default_max_retries() -> u32 {
    3
}
fn default_color() -> String {
    "auto".into()
}
fn default_progress_bar() -> String {
    "modern".into()
}

impl GlobalConfig {
    /// 分层加载：config.toml → 环境变量覆盖 → 返回
    ///
    /// 优先级：环境变量 > config.toml > 代码默认值
    pub fn load(path: &Path) -> Result<Self, OrbitError> {
        // Layer 1: 文件（如果存在）
        let mut config = if path.exists() {
            let content = std::fs::read_to_string(path).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!("failed to read config.toml: {e}"))
            })?;
            toml::from_str(&content).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!("failed to parse config.toml: {e}"))
            })?
        } else {
            let cfg = Self::default();
            // 首次运行时自动写入默认配置
            cfg.save(path)?;
            cfg
        };

        // Layer 2: 环境变量覆盖
        if let Ok(v) = std::env::var("ORBIT_PROXY") {
            config.network.proxy = Some(v);
        }
        if let Ok(v) = std::env::var("ORBIT_TIMEOUT")
            && let Ok(n) = v.parse()
        {
            config.network.timeout = n;
        }
        if let Ok(v) = std::env::var("ORBIT_RETRIES")
            && let Ok(n) = v.parse()
        {
            config.network.max_retries = n;
        }
        if let Ok(v) = std::env::var("ORBIT_LANGUAGE") {
            config.core.language = v;
        }
        if let Ok(v) = std::env::var("ORBIT_CURSEFORGE_API_KEY") {
            config.auth.curseforge_api_key = Some(v);
        }
        if let Ok(v) = std::env::var("ORBIT_MODRINTH_TOKEN") {
            config.auth.modrinth_token = Some(v);
        }

        Ok(config)
    }

    /// 保存到 config.toml
    pub fn save(&self, path: &Path) -> Result<(), OrbitError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to serialize config.toml: {e}"))
        })?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// 写入默认配置（首次使用时）
    pub fn init_default(path: &Path) -> Result<Self, OrbitError> {
        let config = Self::default();
        config.save(path)?;
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
    pub fn load(path: &Path) -> Result<Self, OrbitError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|_| OrbitError::Other(anyhow::anyhow!("failed to read instances.toml")))?;
        let registry: Self = toml::from_str(&content).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to parse instances.toml: {e}"))
        })?;
        Ok(registry)
    }

    pub fn save(&self, path: &Path) -> Result<(), OrbitError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to serialize instances.toml: {e}"))
        })?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.name == name)
    }

    pub fn default_instance(&self) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.is_default)
    }

    pub fn upsert(&mut self, mut entry: InstanceEntry) {
        if let Some(existing) = self
            .instances
            .iter_mut()
            .find(|existing| existing.name == entry.name)
        {
            entry.is_default = existing.is_default || entry.is_default;
            *existing = entry;
        } else {
            self.instances.push(entry);
        }
        self.instances
            .sort_by(|left, right| left.name.cmp(&right.name));
    }

    pub fn set_default(&mut self, name: &str) -> Option<InstanceEntry> {
        let selected = self.find(name)?.clone();
        for instance in &mut self.instances {
            instance.is_default = instance.name == name;
        }
        Some(selected)
    }

    pub fn remove(&mut self, name: &str) -> Option<InstanceEntry> {
        let index = self
            .instances
            .iter()
            .position(|instance| instance.name == name)?;
        Some(self.instances.remove(index))
    }
}

pub fn register_instance(
    paths: &crate::runtime::RuntimePaths,
    entry: InstanceEntry,
) -> Result<(), OrbitError> {
    let mut registry = InstancesRegistry::load(paths.instances_file())?;
    registry.upsert(entry);
    registry.save(paths.instances_file())
}

pub fn set_default_instance(
    paths: &crate::runtime::RuntimePaths,
    name: &str,
) -> Result<InstanceEntry, OrbitError> {
    let mut registry = InstancesRegistry::load(paths.instances_file())?;
    let selected = registry.set_default(name).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "instance '{name}' not found; run 'orbit instances list' to see registered instances"
        ))
    })?;
    registry.save(paths.instances_file())?;

    let mut config = GlobalConfig::load(paths.config_file())?;
    config.core.default_instance = Some(name.to_string());
    config.save(paths.config_file())?;
    Ok(selected)
}

pub fn remove_instance(
    paths: &crate::runtime::RuntimePaths,
    name: &str,
) -> Result<InstanceEntry, OrbitError> {
    let mut registry = InstancesRegistry::load(paths.instances_file())?;
    let removed = registry
        .remove(name)
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("instance '{name}' not found")))?;
    registry.save(paths.instances_file())?;

    let mut config = GlobalConfig::load(paths.config_file())?;
    if config.core.default_instance.as_deref() == Some(name) {
        config.core.default_instance = None;
        config.save(paths.config_file())?;
    }
    Ok(removed)
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
        assert!(config.cache.dir.is_none());
        assert_eq!(config.cache.capacity_mib, 5 * 1024);
        assert_eq!(
            config.cache.capacity_bytes().unwrap(),
            5 * 1024 * 1024 * 1024
        );
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
        let deserialized: GlobalConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.core.max_concurrent_downloads, 8);
        assert_eq!(deserialized.cache.capacity_mib, 5 * 1024);
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

        registry.upsert(updated);

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
}
