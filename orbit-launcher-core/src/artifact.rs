use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha256;

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;

const ALIAS_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedHash {
    Sha1(String),
    Sha256(String),
    Unverified,
}

#[derive(Debug, Clone)]
pub struct ArtifactRequest {
    pub logical_name: String,
    pub url: String,
    pub expected_hash: ExpectedHash,
    pub expected_size: Option<u64>,
}

impl ArtifactRequest {
    pub fn validate(&self) -> Result<(), LauncherError> {
        if self.logical_name.trim().is_empty() || self.logical_name.chars().any(char::is_control) {
            return Err(LauncherError::InvalidRemoteData(
                "artifact logical name is invalid".to_string(),
            ));
        }
        let url = url::Url::parse(&self.url).map_err(|error| {
            LauncherError::InvalidRemoteData(format!(
                "artifact '{}' has invalid URL: {error}",
                self.logical_name
            ))
        })?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err(LauncherError::InvalidRemoteData(format!(
                "artifact '{}' must use an absolute HTTPS URL",
                self.logical_name
            )));
        }
        match &self.expected_hash {
            ExpectedHash::Sha1(value) => validate_digest(value, 40, "SHA-1")?,
            ExpectedHash::Sha256(value) => validate_digest(value, 64, "SHA-256")?,
            ExpectedHash::Unverified => {}
        }
        if self.expected_size == Some(0) {
            return Err(LauncherError::InvalidRemoteData(format!(
                "artifact '{}' declares a zero byte size",
                self.logical_name
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedArtifact {
    pub sha256: String,
    pub size: u64,
    pub object_path: PathBuf,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactTransferEvent {
    Started {
        logical_name: String,
        total_bytes: Option<u64>,
    },
    Bytes {
        logical_name: String,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    Cached {
        logical_name: String,
        size: u64,
    },
    Finished {
        logical_name: String,
        size: u64,
    },
}

#[derive(Debug, Clone)]
pub struct ArtifactCache {
    root: PathBuf,
    aliases: Arc<Mutex<Option<AliasIndex>>>,
    aliases_dirty: Arc<Mutex<bool>>,
}

impl ArtifactCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            aliases: Arc::new(Mutex::new(None)),
            aliases_dirty: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn fetch<F>(
        &self,
        client: &reqwest::Client,
        request: &ArtifactRequest,
        mut progress: F,
    ) -> Result<CachedArtifact, LauncherError>
    where
        F: FnMut(ArtifactTransferEvent),
    {
        request.validate()?;
        let aliases = self.aliases_snapshot()?;
        if let Some(cached) = self.find_cached(request, &aliases)? {
            progress(ArtifactTransferEvent::Cached {
                logical_name: request.logical_name.clone(),
                size: cached.size,
            });
            return Ok(cached);
        }

        std::fs::create_dir_all(self.root.join("staging"))?;
        let temporary = tempfile::Builder::new()
            .prefix("artifact-")
            .tempfile_in(self.root.join("staging"))?;
        let (temporary_file, temporary_path) = temporary.keep().map_err(|error| error.error)?;
        drop(temporary_file);
        let _temporary_guard = TemporaryFileGuard(temporary_path.clone());
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&temporary_path)?;

        let mut response = client.get(&request.url).send().await?.error_for_status()?;
        if response.url().scheme() != "https" {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(LauncherError::InvalidRemoteData(format!(
                "artifact '{}' redirected to a non-HTTPS URL",
                request.logical_name
            )));
        }
        let response_size = response.content_length();
        if let (Some(expected), Some(actual)) = (request.expected_size, response_size)
            && expected != actual
        {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(LauncherError::ArtifactIntegrity(format!(
                "artifact '{}' expected {expected} bytes but server declared {actual}",
                request.logical_name
            )));
        }
        let total = request.expected_size.or(response_size);
        progress(ArtifactTransferEvent::Started {
            logical_name: request.logical_name.clone(),
            total_bytes: total,
        });

        let mut sha1 = Sha1::new();
        let mut sha256 = Sha256::new();
        let mut size = 0_u64;
        while let Some(chunk) = response.chunk().await? {
            size = size.checked_add(chunk.len() as u64).ok_or_else(|| {
                LauncherError::ArtifactIntegrity(format!(
                    "artifact '{}' exceeds the supported size",
                    request.logical_name
                ))
            })?;
            if let Some(expected) = request.expected_size
                && size > expected
            {
                let _ = std::fs::remove_file(&temporary_path);
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "artifact '{}' exceeded its declared size of {expected} bytes",
                    request.logical_name
                )));
            }
            sha1.update(&chunk);
            sha256.update(&chunk);
            file.write_all(&chunk)?;
            progress(ArtifactTransferEvent::Bytes {
                logical_name: request.logical_name.clone(),
                downloaded_bytes: size,
                total_bytes: total,
            });
        }
        file.flush()?;
        file.sync_all()?;
        drop(file);

        if let Some(expected) = request.expected_size
            && expected != size
        {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(LauncherError::ArtifactIntegrity(format!(
                "artifact '{}' expected {expected} bytes but downloaded {size}",
                request.logical_name
            )));
        }
        let actual_sha1 = hex::encode(sha1.finalize());
        let actual_sha256 = hex::encode(sha256.finalize());
        let hash_matches = match &request.expected_hash {
            ExpectedHash::Sha1(expected) => expected == &actual_sha1,
            ExpectedHash::Sha256(expected) => expected == &actual_sha256,
            ExpectedHash::Unverified => true,
        };
        if !hash_matches {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(LauncherError::ArtifactIntegrity(format!(
                "artifact '{}' did not match its declared content hash",
                request.logical_name
            )));
        }

        let object_path = self.object_path(&actual_sha256);
        if let Some(parent) = object_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if object_path.exists() {
            std::fs::remove_file(&temporary_path)?;
        } else if let Err(error) = std::fs::rename(&temporary_path, &object_path) {
            if object_path.exists() {
                std::fs::remove_file(&temporary_path)?;
            } else {
                return Err(error.into());
            }
        }

        self.record_alias(actual_sha1, actual_sha256.clone())?;
        progress(ArtifactTransferEvent::Finished {
            logical_name: request.logical_name.clone(),
            size,
        });
        Ok(CachedArtifact {
            sha256: actual_sha256,
            size,
            object_path,
            cache_hit: false,
        })
    }

    pub fn materialize(
        &self,
        artifact: &CachedArtifact,
        destination: &Path,
    ) -> Result<(), LauncherError> {
        if destination.exists() {
            let actual = hash_file_sha256(destination)?;
            if actual == artifact.sha256 {
                return Ok(());
            }
            return Err(LauncherError::Transaction(format!(
                "refusing to overwrite unexpected staging file '{}'",
                destination.display()
            )));
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::hard_link(&artifact.object_path, destination).is_err() {
            std::fs::copy(&artifact.object_path, destination)?;
        }
        if hash_file_sha256(destination)? != artifact.sha256 {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "materialized artifact '{}' changed content",
                destination.display()
            )));
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<(), LauncherError> {
        let aliases = self.aliases.lock().map_err(|_| {
            LauncherError::Transaction("artifact alias lock was poisoned".to_string())
        })?;
        let mut dirty = self.aliases_dirty.lock().map_err(|_| {
            LauncherError::Transaction("artifact alias dirty flag was poisoned".to_string())
        })?;
        if !*dirty {
            return Ok(());
        }
        let aliases = aliases.as_ref().ok_or_else(|| {
            LauncherError::Transaction(
                "artifact alias index was marked dirty before initialization".to_string(),
            )
        })?;
        aliases.save(&self.alias_path())?;
        *dirty = false;
        Ok(())
    }

    fn find_cached(
        &self,
        request: &ArtifactRequest,
        aliases: &AliasIndex,
    ) -> Result<Option<CachedArtifact>, LauncherError> {
        let sha256 = match &request.expected_hash {
            ExpectedHash::Sha256(value) => Some(value.clone()),
            ExpectedHash::Sha1(value) => aliases.sha1.get(value).cloned(),
            ExpectedHash::Unverified => None,
        };
        let Some(sha256) = sha256 else {
            return Ok(None);
        };
        let path = self.object_path(&sha256);
        let Ok(metadata) = std::fs::metadata(&path) else {
            return Ok(None);
        };
        if request
            .expected_size
            .is_some_and(|expected| metadata.len() != expected)
        {
            return Ok(None);
        }
        Ok(Some(CachedArtifact {
            sha256,
            size: metadata.len(),
            object_path: path,
            cache_hit: true,
        }))
    }

    fn object_path(&self, sha256: &str) -> PathBuf {
        self.root.join("objects").join("sha256").join(sha256)
    }

    fn alias_path(&self) -> PathBuf {
        self.root.join("metadata").join("artifact-aliases.toml")
    }

    fn aliases_snapshot(&self) -> Result<AliasIndex, LauncherError> {
        let mut aliases = self.aliases.lock().map_err(|_| {
            LauncherError::Transaction("artifact alias lock was poisoned".to_string())
        })?;
        if aliases.is_none() {
            *aliases = Some(AliasIndex::load(&self.alias_path())?);
        }
        aliases.as_ref().cloned().ok_or_else(|| {
            LauncherError::Transaction("artifact alias index was not initialized".to_string())
        })
    }

    fn record_alias(&self, sha1: String, sha256: String) -> Result<(), LauncherError> {
        let mut aliases = self.aliases.lock().map_err(|_| {
            LauncherError::Transaction("artifact alias lock was poisoned".to_string())
        })?;
        if aliases.is_none() {
            *aliases = Some(AliasIndex::load(&self.alias_path())?);
        }
        let aliases = aliases.as_mut().ok_or_else(|| {
            LauncherError::Transaction("artifact alias index was not initialized".to_string())
        })?;
        aliases.sha1.insert(sha1, sha256);
        *self.aliases_dirty.lock().map_err(|_| {
            LauncherError::Transaction("artifact alias dirty flag was poisoned".to_string())
        })? = true;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AliasIndex {
    schema: u32,
    #[serde(default)]
    sha1: BTreeMap<String, String>,
}

impl Default for AliasIndex {
    fn default() -> Self {
        Self {
            schema: ALIAS_SCHEMA,
            sha1: BTreeMap::new(),
        }
    }
}

impl AliasIndex {
    fn load(path: &Path) -> Result<Self, LauncherError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let index: Self = toml::from_str(&content).map_err(LauncherError::ConfigParse)?;
        if index.schema != ALIAS_SCHEMA
            || index.sha1.iter().any(|(sha1, sha256)| {
                validate_digest(sha1, 40, "SHA-1").is_err()
                    || validate_digest(sha256, 64, "SHA-256").is_err()
            })
        {
            return Err(LauncherError::InvalidConfig(
                "artifact alias index is invalid".to_string(),
            ));
        }
        Ok(index)
    }

    fn save(&self, path: &Path) -> Result<(), LauncherError> {
        let content = toml::to_string_pretty(self)?;
        write_atomic(path, content.as_bytes())
    }
}

fn validate_digest(value: &str, length: usize, name: &str) -> Result<(), LauncherError> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "'{value}' is not a lowercase {name} digest"
        )));
    }
    Ok(())
}

pub fn hash_file_sha256(path: &Path) -> Result<String, LauncherError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher_writer(&mut hasher))?;
    Ok(hex::encode(hasher.finalize()))
}

fn hasher_writer<D: Digest>(digest: &mut D) -> impl Write + '_ {
    struct Writer<'a, D>(&'a mut D);
    impl<D: Digest> Write for Writer<'_, D> {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.update(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    Writer(digest)
}

struct TemporaryFileGuard(PathBuf);

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialization_never_overwrites_different_staging_content() {
        let directory = tempfile::tempdir().unwrap();
        let cache = ArtifactCache::new(directory.path().join("cache"));
        let object = directory.path().join("object");
        let destination = directory.path().join("staging/server.jar");
        std::fs::write(&object, b"expected").unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&destination, b"unexpected").unwrap();
        let artifact = CachedArtifact {
            sha256: hash_file_sha256(&object).unwrap(),
            size: 8,
            object_path: object,
            cache_hit: false,
        };
        assert!(cache.materialize(&artifact, &destination).is_err());
    }

    #[test]
    fn alias_index_rejects_non_digest_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("aliases.toml");
        std::fs::write(&path, "schema = 1\n[sha1]\nbad = 'also-bad'\n").unwrap();
        assert!(AliasIndex::load(&path).is_err());
    }
}
