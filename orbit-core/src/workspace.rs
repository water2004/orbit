//! manifest / lockfile 的读写封装。
//!
//! 其他模块通过 `ManifestFile` / `Lockfile` 读写 orbit.toml / orbit.lock，
//! 不应直接使用 `std::fs::write` 操作这两个文件。

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{LockMeta, OrbitLockfile};
use crate::manifest::OrbitManifest;

// ── ManifestFile ──────────────────────────────────────────────────

/// orbit.toml 的内存表示 + 文件路径，提供加载/保存。
#[derive(Debug, Clone)]
pub struct ManifestFile {
    path: PathBuf,
    pub inner: OrbitManifest,
}

impl ManifestFile {
    /// 从实例目录加载 orbit.toml。
    pub fn open(dir: &Path) -> Result<Self, OrbitError> {
        let path = dir.join("orbit.toml");
        let inner = OrbitManifest::from_path(&path)?;
        Ok(Self { path, inner })
    }

    /// 用预先构建的 manifest 创建（用于 init）。
    pub fn new(dir: &Path, inner: OrbitManifest) -> Self {
        Self {
            path: dir.join("orbit.toml"),
            inner,
        }
    }

    /// 写入 orbit.toml。
    pub fn save(&self) -> Result<(), OrbitError> {
        std::fs::write(&self.path, self.inner.to_toml_string()?)?;
        Ok(())
    }

    /// 文件所在目录。
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }
}

// ── Lockfile ──────────────────────────────────────────────────────

/// orbit.lock 的内存表示 + 文件路径，提供加载/保存。
#[derive(Debug, Clone)]
pub struct Lockfile {
    path: PathBuf,
    pub inner: OrbitLockfile,
}

impl Lockfile {
    /// 从实例目录加载 orbit.lock（必须存在）。
    pub fn open(dir: &Path) -> Result<Self, OrbitError> {
        let path = dir.join("orbit.lock");
        let inner = OrbitLockfile::from_path(&path)?;
        Ok(Self { path, inner })
    }

    /// 加载 orbit.lock；只有文件确实不存在时才用给定 meta 创建空锁。
    ///
    /// 格式错误或违反模型约束的 lock 必须显式失败，不能被静默当成空状态。
    pub fn open_or_default(dir: &Path, meta: LockMeta) -> Result<Self, OrbitError> {
        let path = dir.join("orbit.lock");
        let inner = match OrbitLockfile::from_path(&path) {
            Ok(lockfile) => lockfile,
            Err(OrbitError::LockfileNotFound) => OrbitLockfile {
                meta,
                packages: vec![],
            },
            Err(error) => return Err(error),
        };
        Ok(Self { path, inner })
    }

    /// 用预先构建的 lockfile 创建（用于 init）。
    pub fn new(dir: &Path, inner: OrbitLockfile) -> Self {
        Self {
            path: dir.join("orbit.lock"),
            inner,
        }
    }

    /// 写入 orbit.lock。
    pub fn save(&self) -> Result<(), OrbitError> {
        std::fs::write(&self.path, self.inner.to_toml_string()?)?;
        Ok(())
    }

    /// 按 mod_id 查找条目。
    pub fn find(&self, mod_id: &str) -> Option<&crate::lockfile::PackageEntry> {
        self.inner.find(mod_id)
    }

    /// 按 JAR 声明的 mod_id 查找包条目。
    pub fn find_entry(&self, package: &str) -> Option<&crate::lockfile::PackageEntry> {
        crate::resolver::find_entry(package, &self.inner.packages)
    }

    /// 通过 mod_id 找到 JAR 文件路径（同时校验 SHA-256）。
    pub fn find_jar_path(&self, mod_id: &str, mods_dir: &Path) -> Result<PathBuf, OrbitError> {
        let entry = self
            .inner
            .find(mod_id)
            .ok_or_else(|| OrbitError::ModNotFound(mod_id.to_string()))?;
        if entry.filename.is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "no filename recorded for '{mod_id}'"
            )));
        }
        let path = mods_dir.join(&entry.filename);
        if !path.exists() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "JAR not found: {}",
                path.display()
            )));
        }
        if !entry.sha256.is_empty() {
            let actual = crate::jar::compute_sha256(&path)?;
            if actual != entry.sha256 {
                return Err(OrbitError::ChecksumMismatch {
                    name: entry.filename.clone(),
                    expected: entry.sha256.clone(),
                    actual,
                });
            }
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> LockMeta {
        LockMeta {
            mc_version: "1.21.1".to_string(),
            modloader: "fabric".to_string(),
            modloader_version: "0.16.10".to_string(),
        }
    }

    #[test]
    fn open_or_default_only_defaults_a_missing_lock() {
        let directory = tempfile::tempdir().unwrap();
        let empty = Lockfile::open_or_default(directory.path(), meta()).unwrap();
        assert!(empty.inner.packages.is_empty());

        std::fs::write(directory.path().join("orbit.lock"), "not valid toml").unwrap();
        let error = Lockfile::open_or_default(directory.path(), meta()).unwrap_err();
        assert!(error.to_string().contains("failed to parse orbit.lock"));
    }
}
