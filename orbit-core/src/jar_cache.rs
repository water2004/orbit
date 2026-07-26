//! Content-addressed JAR cache.
//!
//! Provider filenames are not trusted as cache keys. Artifacts are stored by
//! their locally computed SHA-512; a SHA-1 alias is only an index into that
//! content-addressed store. Access order is recorded explicitly because
//! filesystem access times are neither portable nor reliably enabled.

mod lru;

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use crate::error::OrbitError;

#[derive(Debug, Clone)]
pub struct JarCache {
    root: PathBuf,
    accesses: lru::AccessTracker,
}

impl JarCache {
    pub fn open(root: PathBuf) -> Result<Self, OrbitError> {
        if root.as_os_str().is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "JAR cache path must not be empty"
            )));
        }
        Ok(Self {
            root,
            accesses: lru::AccessTracker::default(),
        })
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

    fn shared_lock(&self) -> Result<File, OrbitError> {
        self.open_lock(true)
    }

    fn exclusive_lock(&self) -> Result<File, OrbitError> {
        self.open_lock(false)
    }

    fn open_lock(&self, shared: bool) -> Result<File, OrbitError> {
        std::fs::create_dir_all(&self.root)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(lru::LOCK_FILE))?;
        if shared {
            lock.lock_shared()?;
        } else {
            lock.lock()?;
        }
        Ok(lock)
    }

    fn resolve_artifact(&self, sha512: &str, sha1: &str) -> Option<(String, PathBuf)> {
        if let Some(sha512) = normalized_hash(sha512, 128) {
            let path = self.sha512_path(&sha512);
            if path.is_file() {
                return Some((sha512, path));
            }
        }

        let sha1 = normalized_hash(sha1, 40)?;
        let target_hash = std::fs::read_to_string(self.sha1_alias_path(&sha1)).ok()?;
        let target_hash = normalized_hash(target_hash.trim(), 128)?;
        let path = self.sha512_path(&target_hash);
        path.is_file().then_some((target_hash, path))
    }

    /// Read an artifact from cache. A missing or stale entry is a cache miss.
    pub fn get_bytes(&self, sha512: &str, sha1: &str) -> Option<Vec<u8>> {
        if !self.root.is_dir() {
            return None;
        }
        let _lock = self.shared_lock().ok()?;
        let (sha512, path) = self.resolve_artifact(sha512, sha1)?;
        let bytes = std::fs::read(path).ok()?;
        self.accesses.record(sha512);
        Some(bytes)
    }

    /// Store bytes under locally computed hashes.
    pub fn store_bytes(&self, bytes: &[u8]) -> Result<(), OrbitError> {
        let _lock = self.exclusive_lock()?;
        let sha1 = crate::jar::sha1_digest(bytes);
        let sha512 = crate::jar::sha512_digest(bytes);
        let artifact_path = self.sha512_path(&sha512);
        if !artifact_path.is_file() {
            write_atomic(&artifact_path, bytes)?;
        }
        self.accesses.record(sha512.clone());

        let alias_path = self.sha1_alias_path(&sha1);
        write_atomic(&alias_path, sha512.as_bytes())?;
        Ok(())
    }

    /// Copy a cached artifact to an installation path.
    pub fn copy_to(&self, sha512: &str, sha1: &str, destination: &Path) -> bool {
        if !self.root.is_dir() {
            return false;
        }
        let Ok(_lock) = self.shared_lock() else {
            return false;
        };
        let Some((sha512, source)) = self.resolve_artifact(sha512, sha1) else {
            return false;
        };
        if let Some(parent) = destination.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return false;
        }
        if std::fs::copy(source, destination).is_err() {
            return false;
        }
        self.accesses.record(sha512);
        true
    }

    /// Merge this process's cache hits into the persistent LRU index and evict
    /// least-recently-used JARs until the hard byte capacity is satisfied.
    ///
    /// Artifacts that have no access record are treated as the oldest entries;
    /// there is no alternate lookup or legacy cache path.
    pub fn prune_to_capacity(&self, capacity_bytes: u64) -> Result<CachePruneSummary, OrbitError> {
        if !self.root.is_dir() {
            return Ok(CachePruneSummary {
                path: self.root.clone(),
                capacity_bytes,
                ..CachePruneSummary::default()
            });
        }

        let _lock = self.exclusive_lock()?;
        lru::prune(&self.root, &self.accesses, capacity_bytes)
    }
}

pub(super) fn normalized_hash(value: &str, expected_len: usize) -> Option<String> {
    (value.len() == expected_len && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "cache file '{}' has no parent directory",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary
        .persist(path)
        .map_err(|error| OrbitError::Io(error.error))?;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CachePruneSummary {
    pub path: PathBuf,
    pub capacity_bytes: u64,
    pub files_before: usize,
    pub bytes_before: u64,
    pub files_removed: usize,
    pub bytes_freed: u64,
    pub files_after: usize,
    pub bytes_after: u64,
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

    #[test]
    fn lru_pruning_removes_the_oldest_artifact_and_its_alias() {
        let directory = test_dir("lru-oldest");
        let cache = JarCache::open(directory.clone()).unwrap();
        let oldest = b"a";
        let newest = b"b";
        let oldest_sha1 = crate::jar::sha1_digest(oldest);
        let oldest_sha512 = crate::jar::sha512_digest(oldest);
        let newest_sha512 = crate::jar::sha512_digest(newest);

        cache.store_bytes(oldest).unwrap();
        cache.store_bytes(newest).unwrap();
        let summary = cache.prune_to_capacity(1).unwrap();

        assert_eq!(summary.files_before, 2);
        assert_eq!(summary.files_removed, 1);
        assert_eq!(summary.bytes_before, 2);
        assert_eq!(summary.bytes_after, 1);
        assert!(!cache.sha512_path(&oldest_sha512).exists());
        assert!(!cache.sha1_alias_path(&oldest_sha1).exists());
        assert!(cache.sha512_path(&newest_sha512).is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lru_order_persists_across_cache_sessions() {
        let directory = test_dir("lru-persistent");
        let first = b"a";
        let second = b"b";
        let third = b"c";
        let first_sha512 = crate::jar::sha512_digest(first);
        let second_sha512 = crate::jar::sha512_digest(second);
        let third_sha512 = crate::jar::sha512_digest(third);

        {
            let cache = JarCache::open(directory.clone()).unwrap();
            cache.store_bytes(first).unwrap();
            cache.store_bytes(second).unwrap();
            cache.prune_to_capacity(2).unwrap();
        }
        {
            let cache = JarCache::open(directory.clone()).unwrap();
            assert_eq!(cache.get_bytes(&first_sha512, "").unwrap(), first);
            cache.store_bytes(third).unwrap();
            cache.prune_to_capacity(2).unwrap();
        }

        assert!(
            directory
                .join("jars")
                .join("sha512")
                .join(first_sha512)
                .is_file()
        );
        assert!(
            !directory
                .join("jars")
                .join("sha512")
                .join(second_sha512)
                .exists()
        );
        assert!(
            directory
                .join("jars")
                .join("sha512")
                .join(third_sha512)
                .is_file()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn zero_capacity_evicts_every_artifact() {
        let directory = test_dir("lru-zero");
        let cache = JarCache::open(directory.clone()).unwrap();
        cache.store_bytes(b"artifact").unwrap();

        let summary = cache.prune_to_capacity(0).unwrap();

        assert_eq!(summary.files_removed, 1);
        assert_eq!(summary.files_after, 0);
        assert_eq!(summary.bytes_after, 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn copying_an_artifact_marks_it_as_recently_used() {
        let directory = test_dir("lru-copy");
        let destination = directory.join("installed").join("selected.jar");
        let first = b"a";
        let second = b"b";
        let first_sha512 = crate::jar::sha512_digest(first);
        let second_sha512 = crate::jar::sha512_digest(second);

        {
            let cache = JarCache::open(directory.clone()).unwrap();
            cache.store_bytes(first).unwrap();
            cache.store_bytes(second).unwrap();
            cache.prune_to_capacity(2).unwrap();
        }
        {
            let cache = JarCache::open(directory.clone()).unwrap();
            assert!(cache.copy_to(&first_sha512, "", &destination));
            cache.prune_to_capacity(1).unwrap();
        }

        assert_eq!(std::fs::read(destination).unwrap(), first);
        assert!(
            directory
                .join("jars")
                .join("sha512")
                .join(first_sha512)
                .is_file()
        );
        assert!(
            !directory
                .join("jars")
                .join("sha512")
                .join(second_sha512)
                .exists()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn corrupt_lru_index_is_reported_instead_of_guessed() {
        let directory = test_dir("lru-corrupt-index");
        let cache = JarCache::open(directory.clone()).unwrap();
        cache.store_bytes(b"artifact").unwrap();
        std::fs::write(directory.join("lru-index.json"), b"not json").unwrap();

        let error = cache.prune_to_capacity(u64::MAX).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to parse JAR cache LRU index")
        );
        assert!(error.to_string().contains("orbit cache clean"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
