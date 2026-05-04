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
        Self { path: dir.join("orbit.toml"), inner }
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

    /// 加载 orbit.lock，不存在时用给定 meta 创建空锁文件。
    pub fn open_or_default(dir: &Path, meta: LockMeta) -> Self {
        let path = dir.join("orbit.lock");
        let inner = OrbitLockfile::from_path(&path).unwrap_or_else(|_| OrbitLockfile {
            meta,
            packages: vec![],
        });
        Self { path, inner }
    }

    /// 用预先构建的 lockfile 创建（用于 init）。
    pub fn new(dir: &Path, inner: OrbitLockfile) -> Self {
        Self { path: dir.join("orbit.lock"), inner }
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

    /// 按 slug 或 mod_id 查找条目。
    pub fn find_entry(&self, slug: &str) -> Option<&crate::lockfile::PackageEntry> {
        crate::resolver::find_entry(slug, &self.inner.packages)
    }
}
