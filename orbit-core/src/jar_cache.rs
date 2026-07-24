//! Content-addressed JAR cache.
//!
//! Provider filenames are not trusted as cache keys. Artifacts are stored by
//! their locally computed SHA-512; a SHA-1 alias is only an index into that
//! content-addressed store.

use std::path::{Path, PathBuf};

use crate::error::OrbitError;

#[derive(Debug, Clone)]
pub struct JarCache {
    root: PathBuf,
}

impl JarCache {
    pub fn open(root: PathBuf) -> Result<Self, OrbitError> {
        if root.as_os_str().is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "JAR cache path must not be empty"
            )));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn sha512_path(&self, sha512: &str) -> PathBuf {
        self.root.join("jars").join("sha512").join(sha512)
    }

    fn sha1_alias_path(&self, sha1: &str) -> PathBuf {
        self.root.join("aliases").join("sha1").join(sha1)
    }

    fn get_by_sha512(&self, sha512: &str) -> Option<PathBuf> {
        (!sha512.is_empty()).then(|| self.sha512_path(sha512))
    }

    fn get_by_sha1(&self, sha1: &str) -> Option<PathBuf> {
        if sha1.is_empty() {
            return None;
        }
        let target_hash = std::fs::read_to_string(self.sha1_alias_path(sha1)).ok()?;
        let target_hash = target_hash.trim();
        if target_hash.is_empty() {
            return None;
        }
        Some(self.sha512_path(target_hash))
    }

    /// Read an artifact from cache. A missing or stale entry is a cache miss.
    pub fn get_bytes(&self, sha512: &str, sha1: &str) -> Option<Vec<u8>> {
        let path = self
            .get_by_sha512(sha512)
            .filter(|path| path.is_file())
            .or_else(|| self.get_by_sha1(sha1).filter(|path| path.is_file()))?;
        std::fs::read(path).ok()
    }

    /// Store bytes under locally computed hashes.
    pub fn store_bytes(&self, bytes: &[u8]) -> Result<(), OrbitError> {
        let sha1 = crate::jar::sha1_digest(bytes);
        let sha512 = crate::jar::sha512_digest(bytes);
        let artifact_path = self.sha512_path(&sha512);
        if let Some(parent) = artifact_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !artifact_path.is_file() {
            std::fs::write(&artifact_path, bytes)?;
        }

        let alias_path = self.sha1_alias_path(&sha1);
        if let Some(parent) = alias_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(alias_path, sha512)?;
        Ok(())
    }

    /// Copy a cached artifact to an installation path.
    pub fn copy_to(&self, sha512: &str, sha1: &str, destination: &Path) -> bool {
        let source = self
            .get_by_sha512(sha512)
            .filter(|path| path.is_file())
            .or_else(|| self.get_by_sha1(sha1).filter(|path| path.is_file()));
        let Some(source) = source else {
            return false;
        };
        if let Some(parent) = destination.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return false;
        }
        std::fs::copy(source, destination).is_ok()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheSummary {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

pub fn inspect_cache(path: &Path, protected_paths: &[&Path]) -> Result<CacheSummary, OrbitError> {
    if path.exists() {
        validate_cache_dir(path, protected_paths)?;
    }
    inspect_cache_dir(path)
}

pub fn clean_cache(path: &Path, protected_paths: &[&Path]) -> Result<CacheSummary, OrbitError> {
    let summary = inspect_cache(path, protected_paths)?;
    if !summary.path.exists() {
        return Ok(summary);
    }
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

fn validate_cache_dir(path: &Path, protected_paths: &[&Path]) -> Result<(), OrbitError> {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let current = std::env::current_dir().unwrap_or_default();
    let contains_current = current.starts_with(&resolved);
    let contains_protected_path = protected_paths.iter().any(|protected| {
        protected
            .canonicalize()
            .unwrap_or_else(|_| protected.to_path_buf())
            .starts_with(&resolved)
    });
    if resolved.parent().is_none() || contains_current || contains_protected_path {
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
        std::fs::write(directory.join("alias"), b"1234").unwrap();
        std::fs::write(nested.join("artifact"), b"123456").unwrap();

        let summary = inspect_cache_dir(&directory).unwrap();

        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, 10);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unsafe_cache_roots_are_rejected() {
        let root = std::path::Path::new(std::path::MAIN_SEPARATOR_STR);
        assert!(validate_cache_dir(root, &[]).is_err());
    }

    #[test]
    fn cache_cleanup_rejects_a_directory_containing_protected_data() {
        let directory = test_dir("protected");
        let config = directory.join("config.toml");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&config, b"").unwrap();

        assert!(validate_cache_dir(&directory, &[&config]).is_err());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn artifacts_are_addressed_by_content_not_provider_filename() {
        let directory = test_dir("content");
        let cache = JarCache::open(directory.clone()).unwrap();
        let bytes = b"artifact";
        let sha1 = crate::jar::sha1_digest(bytes);
        let sha512 = crate::jar::sha512_digest(bytes);

        cache.store_bytes(bytes).unwrap();

        assert_eq!(cache.get_bytes(&sha512, "").unwrap(), bytes);
        assert_eq!(cache.get_bytes("", &sha1).unwrap(), bytes);
        assert_eq!(
            cache.sha512_path(&sha512),
            directory.join("jars").join("sha512").join(sha512)
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
