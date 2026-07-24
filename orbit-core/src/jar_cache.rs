//! 基于哈希的 JAR 文件缓存。
//!
//! 索引文件 `index.toml` 记录三种哈希（sha1/sha256/sha512）→ JAR 文件名的映射。
//! JAR 以原始文件名存放在 `{cache_dir}/jars/` 下。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::OrbitError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CacheIndex {
    #[serde(default)]
    sha1: HashMap<String, String>,
    #[serde(default)]
    sha256: HashMap<String, String>,
    #[serde(default)]
    sha512: HashMap<String, String>,
}

pub struct JarCache {
    jar_dir: PathBuf,
    index: CacheIndex,
    index_path: PathBuf,
}

impl JarCache {
    pub fn load() -> Result<Self, OrbitError> {
        let dir = crate::config::GlobalConfig::load()?.cache.resolved_dir();
        Self::load_from(dir)
    }

    fn load_from(dir: PathBuf) -> Result<Self, OrbitError> {
        let jar_dir = dir.join("jars");
        let index_path = dir.join("index.toml");
        let index = if index_path.exists() {
            let content = std::fs::read_to_string(&index_path).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!("failed to read cache index: {e}"))
            })?;
            toml::from_str(&content).unwrap_or_default()
        } else {
            CacheIndex::default()
        };
        Ok(Self {
            jar_dir,
            index,
            index_path,
        })
    }

    fn save(&self) -> Result<(), OrbitError> {
        if let Some(parent) = self.index_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(&self.index).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("failed to serialize cache index: {e}"))
        })?;
        std::fs::write(&self.index_path, content)?;
        Ok(())
    }

    fn get_by_sha512(&self, sha512: &str) -> Option<PathBuf> {
        self.index.sha512.get(sha512).map(|f| self.jar_dir.join(f))
    }

    /// 从缓存取字节，未命中返回 None
    pub fn get_bytes(&self, sha512: &str) -> Option<Vec<u8>> {
        if sha512.is_empty() {
            return None;
        }
        let path = self.get_by_sha512(sha512)?;
        std::fs::read(&path).ok()
    }

    /// 存入 JAR 并更新三种哈希索引（哈希全部从 bytes 自算）
    pub fn store_bytes(
        &mut self,
        sha512: &str,
        filename: &str,
        bytes: &[u8],
    ) -> Result<(), OrbitError> {
        if sha512.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.jar_dir)?;
        let dest = self.jar_dir.join(filename);
        std::fs::write(&dest, bytes)?;

        let sha1 = crate::jar::sha1_digest(bytes);
        let sha256 = crate::jar::sha256_digest(bytes);
        if !sha1.is_empty() {
            self.index.sha1.insert(sha1, filename.to_string());
        }
        if !sha256.is_empty() {
            self.index.sha256.insert(sha256, filename.to_string());
        }
        self.index
            .sha512
            .insert(sha512.to_string(), filename.to_string());
        self.save()
    }

    /// 复制缓存文件到目标路径，未命中返回 false
    pub fn copy_to(&self, sha512: &str, dest: &Path) -> bool {
        if let Some(ref src) = self.get_by_sha512(sha512)
            && src.exists()
        {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            return std::fs::copy(src, dest).is_ok();
        }
        false
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheSummary {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

pub fn inspect_cache() -> Result<CacheSummary, OrbitError> {
    let path = crate::config::GlobalConfig::load()?.cache.resolved_dir();
    inspect_cache_dir(&path)
}

pub fn clean_cache() -> Result<CacheSummary, OrbitError> {
    let summary = inspect_cache()?;
    if !summary.path.exists() {
        return Ok(summary);
    }
    validate_cache_dir(&summary.path)?;
    std::fs::remove_dir_all(&summary.path)?;
    Ok(summary)
}

fn inspect_cache_dir(path: &Path) -> Result<CacheSummary, OrbitError> {
    let mut summary = CacheSummary {
        path: path.to_path_buf(),
        ..CacheSummary::default()
    };
    if !path.exists() {
        return Ok(summary);
    }
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                summary.files += 1;
                summary.bytes += metadata.len();
            }
        }
    }
    Ok(summary)
}

fn validate_cache_dir(path: &Path) -> Result<(), OrbitError> {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if resolved.parent().is_none()
        || resolved == std::env::current_dir().unwrap_or_default()
        || resolved == crate::config::orbit_data_dir()
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "refusing to clear unsafe cache directory '{}'",
            resolved.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("orbit-cache-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn cache_inspection_counts_nested_files() {
        let directory = test_dir("inspect");
        let nested = directory.join("jars");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(directory.join("index.toml"), b"1234").unwrap();
        std::fs::write(nested.join("a.jar"), b"123456").unwrap();

        let summary = inspect_cache_dir(&directory).unwrap();

        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, 10);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unsafe_cache_roots_are_rejected() {
        let root = std::path::Path::new(std::path::MAIN_SEPARATOR_STR);
        assert!(validate_cache_dir(root).is_err());
    }
}
