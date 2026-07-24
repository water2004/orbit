//! 基于哈希的 JAR 文件缓存。
//!
//! 索引文件 `index.toml` 记录三种哈希（sha1/sha256/sha512）→ JAR 文件名的映射。
//! JAR 以原始文件名存放在 `{cache_dir}/jars/` 下。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::default_cache_dir;
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
        let dir = default_cache_dir();
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
        if let Some(ref src) = self.get_by_sha512(sha512) {
            if src.exists() {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                return std::fs::copy(src, dest).is_ok();
            }
        }
        false
    }
}
