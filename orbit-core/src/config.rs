//! 全局 Orbit 配置管理。
//!
//! 管理 `~/.orbit/instances.toml`（实例注册表）和 `~/.orbit/cache/`（下载缓存）。

use serde::{Deserialize, Serialize};
use crate::error::OrbitError;

/// 注册的实例条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceEntry {
    pub name: String,
    pub path: String,
    pub mc_version: String,
    pub modloader: String,
    #[serde(default)]
    pub is_default: bool,
}

/// 全局实例注册表 (~/.orbit/instances.toml)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstancesRegistry {
    pub instances: Vec<InstanceEntry>,
}

impl InstancesRegistry {
    /// 加载全局实例注册表
    pub fn load() -> Result<Self, OrbitError> {
        let path = instances_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|_| OrbitError::Other(anyhow::anyhow!("failed to read instances.toml")))?;
        let registry: Self = toml::from_str(&content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to parse instances.toml: {e}")))?;
        Ok(registry)
    }

    /// 保存全局实例注册表
    pub fn save(&self) -> Result<(), OrbitError> {
        let path = instances_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to serialize instances.toml: {e}")))?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// 按名称查找实例
    pub fn find(&self, name: &str) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.name == name)
    }

    /// 获取默认实例
    pub fn default_instance(&self) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.is_default)
    }
}

fn instances_path() -> std::path::PathBuf {
    dirs_next().join(".orbit").join("instances.toml")
}

fn cache_dir() -> std::path::PathBuf {
    dirs_next().join(".orbit").join("cache")
}

fn dirs_next() -> std::path::PathBuf {
    // 跨平台用户目录
    #[cfg(target_os = "windows")]
    { std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into())) }

    #[cfg(not(target_os = "windows"))]
    { std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())) }
}

/// 获取缓存目录路径
pub fn get_cache_dir() -> std::path::PathBuf {
    cache_dir()
}
