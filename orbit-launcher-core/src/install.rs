use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::artifact::{
    ArtifactCache, ArtifactTransferEvent, CachedArtifact, ExpectedHash, hash_file_sha256,
};
use crate::atomic_io::write_atomic;
use crate::config::JavaProvider;
use crate::error::LauncherError;
use crate::eula::{EulaAcceptance, EulaDocument, require_current_acceptance, show_current_eula};
use crate::instance::{InstanceKind, InstanceManifest, JavaPolicy, LoaderKind, ManifestFile};
use crate::java::{
    JavaProgressEvent, JavaTarget, MojangJavaPlan, install_mojang_java, plan_mojang_java,
};
use crate::lockfile::{
    ArtifactOwner, INSTANCE_LOCK_FILE, LOCK_SCHEMA, LauncherLock, LockFile, LockedArguments,
    LockedArtifact, LockedEntrypoint, LockedLoader, LockedMinecraft,
};
use crate::mojang::{MojangClient, ResolvedVanillaServer, VERSION_MANIFEST_V2_URL};
use crate::runtime::RuntimePaths;

const STATE_DIRECTORY: &str = ".orbit-launcher";
const TRANSACTION_LOCK: &str = "transaction.lock";
const TRANSACTION_JOURNAL: &str = "transaction.json";

#[derive(Debug, Clone)]
pub struct VanillaServerInstallPlan {
    instance_id: Uuid,
    minecraft_requirement: String,
    resolved: ResolvedVanillaServer,
    java: MojangJavaPlan,
    eula: EulaDocument,
    acceptance: Option<EulaAcceptance>,
}

impl VanillaServerInstallPlan {
    pub fn minecraft_version(&self) -> &str {
        &self.resolved.minecraft_version
    }

    pub fn java_major(&self) -> Option<u32> {
        self.resolved.java.as_ref().map(|java| java.major)
    }

    pub fn eula(&self) -> &EulaDocument {
        &self.eula
    }

    pub const fn eula_is_accepted(&self) -> bool {
        self.acceptance.is_some()
    }

    pub fn download_size(&self) -> Option<u64> {
        self.resolved.server.expected_size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallProgressEvent {
    MetadataStarted,
    MinecraftResolved {
        version: String,
        total_artifacts: usize,
    },
    EulaChecked {
        digest_sha256: String,
        accepted: bool,
    },
    Artifact(ArtifactTransferEvent),
    Java(JavaProgressEvent),
    StagingVerified,
    Committed,
}

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub lock: LauncherLock,
    pub downloaded_artifacts: usize,
    pub cached_artifacts: usize,
}

pub async fn prepare_vanilla_server_install<F>(
    instance_root: &Path,
    client: &reqwest::Client,
    default_java_provider: JavaProvider,
    mut progress: F,
) -> Result<VanillaServerInstallPlan, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    let manifest = ManifestFile::open(instance_root)?.inner;
    require_vanilla_server(&manifest)?;
    progress(InstallProgressEvent::MetadataStarted);
    let resolved = MojangClient::new(client.clone())
        .resolve_vanilla_server(&manifest.minecraft.requirement)
        .await?;
    progress(InstallProgressEvent::MinecraftResolved {
        version: resolved.minecraft_version.clone(),
        total_artifacts: 1,
    });
    let java_requirement = resolved.java.as_ref().ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Minecraft '{}' does not publish an authoritative Java runtime requirement",
            resolved.minecraft_version
        ))
    })?;
    let provider = match manifest.java.policy {
        JavaPolicy::Auto => default_java_provider,
        JavaPolicy::Managed => manifest.java.provider.unwrap_or(default_java_provider),
        JavaPolicy::System => {
            return Err(LauncherError::UnsupportedRequirement(
                "system Java selection is not yet supported by the managed install transaction"
                    .to_string(),
            ));
        }
    };
    if provider != JavaProvider::Mojang {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Java provider '{}' is not yet implemented",
            provider.as_str()
        )));
    }
    let java = plan_mojang_java(client, java_requirement, JavaTarget::native()?, |event| {
        progress(InstallProgressEvent::Java(event));
    })
    .await?;
    let eula = show_current_eula(instance_root, client).await?;
    let acceptance = match require_current_acceptance(instance_root, &eula) {
        Ok(acceptance) => Some(acceptance),
        Err(LauncherError::EulaRequired(_)) => None,
        Err(error) => return Err(error),
    };
    progress(InstallProgressEvent::EulaChecked {
        digest_sha256: eula.digest_sha256.clone(),
        accepted: acceptance.is_some(),
    });
    Ok(VanillaServerInstallPlan {
        instance_id: manifest.id,
        minecraft_requirement: manifest.minecraft.requirement,
        resolved,
        java,
        eula,
        acceptance,
    })
}

pub async fn execute_vanilla_server_install<F>(
    instance_root: &Path,
    runtime_paths: &RuntimePaths,
    client: &reqwest::Client,
    plan: VanillaServerInstallPlan,
    concurrency: usize,
    mut progress: F,
) -> Result<InstallResult, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    let manifest = ManifestFile::open(instance_root)?.inner;
    require_vanilla_server(&manifest)?;
    if manifest.id != plan.instance_id
        || manifest.minecraft.requirement != plan.minecraft_requirement
    {
        return Err(LauncherError::Transaction(
            "instance manifest changed after the install plan was generated".to_string(),
        ));
    }
    let acceptance = require_current_acceptance(instance_root, &plan.eula)?;
    let transaction = InstallTransaction::begin(instance_root, "install")?;
    let java = install_mojang_java(runtime_paths, client, plan.java, concurrency, |event| {
        progress(InstallProgressEvent::Java(event))
    })
    .await?;
    let cache = ArtifactCache::new(runtime_paths.cache_dir());
    let cached = cache
        .fetch(client, &plan.resolved.server, |event| {
            progress(InstallProgressEvent::Artifact(event));
        })
        .await?;
    cache.materialize(&cached, &transaction.staging.join("server.jar"))?;
    std::fs::write(transaction.staging.join("eula.txt"), b"eula=true\n")?;

    let upstream_sha1 = match &plan.resolved.server.expected_hash {
        ExpectedHash::Sha1(value) => Some(value.clone()),
        _ => None,
    };
    let lock = LauncherLock {
        schema: LOCK_SCHEMA,
        instance_id: manifest.id,
        kind: InstanceKind::Server,
        minecraft: LockedMinecraft {
            version: plan.resolved.minecraft_version,
            version_type: plan.resolved.version_type,
            version_manifest_url: VERSION_MANIFEST_V2_URL.to_string(),
            version_manifest_sha256: plan.resolved.version_manifest_sha256,
            version_json_url: plan.resolved.version_json_url,
            version_json_sha1: plan.resolved.version_json_sha1,
        },
        loader: LockedLoader {
            kind: LoaderKind::Vanilla,
            version: None,
            profile_url: None,
            profile_sha256: None,
        },
        java: Some(java.locked),
        entrypoint: LockedEntrypoint::Jar {
            path: "server.jar".to_string(),
        },
        arguments: LockedArguments {
            jvm: Vec::new(),
            game: vec!["nogui".to_string()],
        },
        artifacts: vec![LockedArtifact {
            logical_name: plan.resolved.server.logical_name,
            owner: ArtifactOwner::Minecraft,
            source_url: plan.resolved.server.url,
            upstream_sha1,
            sha256: cached.sha256.clone(),
            size: cached.size,
            path: "server.jar".to_string(),
        }],
        generated_files: vec!["eula.txt".to_string()],
        eula: Some(acceptance),
    };
    lock.validate()?;
    LockFile::new(&transaction.staging, lock.clone()).save()?;
    verify_staged_server(&transaction.staging, &cached)?;
    progress(InstallProgressEvent::StagingVerified);
    transaction.commit(&["server.jar", "eula.txt", INSTANCE_LOCK_FILE])?;
    progress(InstallProgressEvent::Committed);
    Ok(InstallResult {
        lock,
        downloaded_artifacts: usize::from(!cached.cache_hit) + java.downloaded_artifacts,
        cached_artifacts: usize::from(cached.cache_hit) + java.cached_artifacts,
    })
}

fn require_vanilla_server(manifest: &InstanceManifest) -> Result<(), LauncherError> {
    if manifest.kind != InstanceKind::Server {
        return Err(LauncherError::UnsupportedRequirement(
            "Vanilla server installation requires a server instance".to_string(),
        ));
    }
    if manifest.loader.kind != LoaderKind::Vanilla {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Loader '{}' must use its own install adapter",
            manifest.loader.kind.as_str()
        )));
    }
    Ok(())
}

fn verify_staged_server(staging: &Path, artifact: &CachedArtifact) -> Result<(), LauncherError> {
    let server = staging.join("server.jar");
    if std::fs::metadata(&server)?.len() != artifact.size
        || hash_file_sha256(&server)? != artifact.sha256
    {
        return Err(LauncherError::ArtifactIntegrity(
            "staged Minecraft server JAR failed final verification".to_string(),
        ));
    }
    let eula = std::fs::read_to_string(staging.join("eula.txt"))?;
    if eula != "eula=true\n" {
        return Err(LauncherError::Transaction(
            "staged eula.txt is invalid".to_string(),
        ));
    }
    LockFile::open(staging)?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionIdentity {
    schema: u32,
    id: Uuid,
    pid: u32,
    started_at_unix_seconds: u64,
    executable: PathBuf,
    command: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionJournal {
    schema: u32,
    id: Uuid,
    phase: String,
    files: Vec<String>,
}

struct InstallTransaction {
    root: PathBuf,
    state: PathBuf,
    staging: PathBuf,
    id: Uuid,
}

impl InstallTransaction {
    fn begin(root: &Path, command: &str) -> Result<Self, LauncherError> {
        let state = root.join(STATE_DIRECTORY);
        std::fs::create_dir_all(&state)?;
        let id = Uuid::new_v4();
        let identity = TransactionIdentity {
            schema: 1,
            id,
            pid: std::process::id(),
            started_at_unix_seconds: unix_seconds()?,
            executable: std::env::current_exe()?,
            command: command.to_string(),
        };
        let lock_path = state.join(TRANSACTION_LOCK);
        let mut lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| {
                LauncherError::Transaction(format!(
                    "cannot acquire instance transaction lock '{}': {error}; run repair instead of deleting it blindly",
                    lock_path.display()
                ))
            })?;
        let identity_bytes = serde_json::to_vec_pretty(&identity).map_err(|error| {
            LauncherError::Transaction(format!("cannot serialize transaction identity: {error}"))
        })?;
        lock.write_all(&identity_bytes)?;
        lock.flush()?;
        lock.sync_all()?;
        let staging = state.join("staging").join(id.to_string());
        if let Err(error) = std::fs::create_dir_all(&staging) {
            let _ = std::fs::remove_file(&lock_path);
            return Err(error.into());
        }
        Ok(Self {
            root: root.to_path_buf(),
            state,
            staging,
            id,
        })
    }

    fn commit(self, relative_files: &[&str]) -> Result<(), LauncherError> {
        let previous = load_previous_owned_paths(&self.root)?;
        for relative in relative_files {
            let target = self.root.join(relative);
            if target.exists() && *relative != INSTANCE_LOCK_FILE && !previous.contains(*relative) {
                return Err(LauncherError::Transaction(format!(
                    "refusing to overwrite unowned instance file '{relative}'"
                )));
            }
            if !self.staging.join(relative).is_file() {
                return Err(LauncherError::Transaction(format!(
                    "staging file '{relative}' is missing"
                )));
            }
        }
        let journal = TransactionJournal {
            schema: 1,
            id: self.id,
            phase: "committing".to_string(),
            files: relative_files
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        };
        write_json_atomic(&self.state.join(TRANSACTION_JOURNAL), &journal)?;

        let backup_root = self.staging.join("backup");
        let mut committed = Vec::new();
        for relative in relative_files {
            let target = self.root.join(relative);
            let backup = backup_root.join(relative);
            let had_backup = target.exists();
            if had_backup {
                if let Some(parent) = backup.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&target, &backup)?;
            }
            let source = self.staging.join(relative);
            if let Err(error) = std::fs::rename(&source, &target) {
                if had_backup {
                    let _ = std::fs::rename(&backup, &target);
                }
                rollback_committed(&self.root, &backup_root, &committed)?;
                let _ = std::fs::remove_file(self.state.join(TRANSACTION_JOURNAL));
                let _ = std::fs::remove_file(self.state.join(TRANSACTION_LOCK));
                let _ = std::fs::remove_dir_all(&self.staging);
                return Err(error.into());
            }
            committed.push(((*relative).to_string(), had_backup));
        }

        std::fs::remove_file(self.state.join(TRANSACTION_JOURNAL))?;
        std::fs::remove_file(self.state.join(TRANSACTION_LOCK))?;
        std::fs::remove_dir_all(&self.staging)?;
        Ok(())
    }
}

impl Drop for InstallTransaction {
    fn drop(&mut self) {
        if !self.state.join(TRANSACTION_JOURNAL).exists() {
            let _ = std::fs::remove_dir_all(&self.staging);
            let _ = std::fs::remove_file(self.state.join(TRANSACTION_LOCK));
        }
    }
}

fn load_previous_owned_paths(root: &Path) -> Result<HashSet<String>, LauncherError> {
    if !root.join(INSTANCE_LOCK_FILE).exists() {
        return Ok(HashSet::new());
    }
    let lock = LockFile::open(root)?.inner;
    Ok(lock
        .artifacts
        .into_iter()
        .map(|artifact| artifact.path)
        .chain(lock.generated_files)
        .collect())
}

fn rollback_committed(
    root: &Path,
    backup_root: &Path,
    committed: &[(String, bool)],
) -> Result<(), LauncherError> {
    for (relative, had_backup) in committed.iter().rev() {
        let target = root.join(relative);
        if target.exists() {
            std::fs::remove_file(&target)?;
        }
        if *had_backup {
            std::fs::rename(backup_root.join(relative), target)?;
        }
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), LauncherError> {
    let content = serde_json::to_vec_pretty(value).map_err(|error| {
        LauncherError::Transaction(format!("cannot serialize transaction journal: {error}"))
    })?;
    write_atomic(path, &content)
}

fn unix_seconds() -> Result<u64, LauncherError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            LauncherError::Transaction(format!("system clock is before the Unix epoch: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_refuses_to_replace_an_unowned_server_jar() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("server.jar"), b"user file").unwrap();
        let transaction = InstallTransaction::begin(directory.path(), "test").unwrap();
        std::fs::write(transaction.staging.join("server.jar"), b"new").unwrap();
        std::fs::write(transaction.staging.join("eula.txt"), b"eula=true\n").unwrap();
        std::fs::write(transaction.staging.join(INSTANCE_LOCK_FILE), b"not reached").unwrap();
        assert!(
            transaction
                .commit(&["server.jar", "eula.txt", INSTANCE_LOCK_FILE])
                .is_err()
        );
        assert_eq!(
            std::fs::read(directory.path().join("server.jar")).unwrap(),
            b"user file"
        );
    }

    #[test]
    fn active_transaction_lock_is_never_deleted_based_on_pid_guessing() {
        let directory = tempfile::tempdir().unwrap();
        let first = InstallTransaction::begin(directory.path(), "first").unwrap();
        assert!(InstallTransaction::begin(directory.path(), "second").is_err());
        drop(first);
        assert!(InstallTransaction::begin(directory.path(), "third").is_ok());
    }
}
