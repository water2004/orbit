use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use uuid::Uuid;

use crate::artifact::{ArtifactCache, ArtifactRequest, ArtifactTransferEvent, ExpectedHash};
use crate::atomic_io::write_atomic;
use crate::error::LauncherError;
use crate::lockfile::LockFile;
use crate::lockfile::LockedJavaRuntime;
use crate::mojang::MojangJavaRequirement;
use crate::platform::{Architecture, HostPlatform, OperatingSystem};
use crate::runtime::RuntimePaths;

pub const MOJANG_RUNTIME_MANIFEST_URL: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const RUNTIME_INVENTORY_FILE: &str = "orbit-launcher-runtime.toml";
const RUNTIME_STAGING_DIRECTORY: &str = ".staging";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaTarget {
    WindowsX64,
    WindowsX86,
    WindowsArm64,
    LinuxX64,
    LinuxX86,
    MacOsX64,
    MacOsArm64,
}

impl JavaTarget {
    pub fn native() -> Result<Self, LauncherError> {
        Self::for_platform(&HostPlatform::native()?)
    }

    pub fn for_platform(platform: &HostPlatform) -> Result<Self, LauncherError> {
        match (platform.os, platform.architecture) {
            (OperatingSystem::Windows, Architecture::X86_64) => Ok(Self::WindowsX64),
            (OperatingSystem::Windows, Architecture::X86) => Ok(Self::WindowsX86),
            (OperatingSystem::Windows, Architecture::Arm64) => Ok(Self::WindowsArm64),
            (OperatingSystem::Linux, Architecture::X86_64) => Ok(Self::LinuxX64),
            (OperatingSystem::Linux, Architecture::X86) => Ok(Self::LinuxX86),
            (OperatingSystem::MacOs, Architecture::X86_64) => Ok(Self::MacOsX64),
            (OperatingSystem::MacOs, Architecture::Arm64) => Ok(Self::MacOsArm64),
            (os, arch) => Err(LauncherError::UnsupportedRequirement(format!(
                "Mojang does not publish a Java runtime mapping for {os:?}/{arch:?}"
            ))),
        }
    }

    pub const fn mojang_name(self) -> &'static str {
        match self {
            Self::WindowsX64 => "windows-x64",
            Self::WindowsX86 => "windows-x86",
            Self::WindowsArm64 => "windows-arm64",
            Self::LinuxX64 => "linux",
            Self::LinuxX86 => "linux-i386",
            Self::MacOsX64 => "mac-os",
            Self::MacOsArm64 => "mac-os-arm64",
        }
    }

    const fn executable(self) -> &'static str {
        match self {
            Self::WindowsX64 | Self::WindowsX86 | Self::WindowsArm64 => "bin/java.exe",
            _ => "bin/java",
        }
    }

    const fn expected_os_arch(self) -> &'static [&'static str] {
        match self {
            Self::WindowsX64 | Self::LinuxX64 | Self::MacOsX64 => &["amd64", "x86_64"],
            Self::WindowsX86 | Self::LinuxX86 => &["x86", "i386"],
            Self::WindowsArm64 | Self::MacOsArm64 => &["aarch64", "arm64"],
        }
    }
}

#[derive(Debug, Clone)]
pub struct MojangJavaPlan {
    target: JavaTarget,
    component: String,
    major: u32,
    version: String,
    runtime_id: String,
    manifest_url: String,
    manifest_sha1: String,
    entries: Vec<PlannedEntry>,
}

impl MojangJavaPlan {
    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub const fn major(&self) -> u32 {
        self.major
    }

    pub fn artifact_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.kind, PlannedEntryKind::File { .. }))
            .count()
    }

    pub fn total_download_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                PlannedEntryKind::File { request, .. } => request.expected_size,
                _ => None,
            })
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JavaProgressEvent {
    ManifestStarted,
    RuntimeResolved {
        runtime_id: String,
        artifacts: usize,
        total_bytes: u64,
    },
    Artifact(ArtifactTransferEvent),
    Materialized {
        completed: usize,
        total: usize,
    },
    RuntimeVerified {
        runtime_id: String,
    },
    RuntimeCached {
        runtime_id: String,
    },
}

#[derive(Debug, Clone)]
pub struct ManagedJavaRuntime {
    pub locked: LockedJavaRuntime,
    pub root: PathBuf,
    pub downloaded_artifacts: usize,
    pub cached_artifacts: usize,
}

#[derive(Debug, Clone)]
pub struct InstalledJavaRuntime {
    pub runtime_id: String,
    pub provider: String,
    pub component: String,
    pub platform: String,
    pub version: String,
    pub major: u32,
    pub root: PathBuf,
    pub executable: PathBuf,
    pub files: usize,
    pub bytes: u64,
    /// `None` means the caller requested inventory-only listing.
    pub verified: Option<bool>,
}

pub fn list_managed_java_runtimes(
    runtime_paths: &RuntimePaths,
    verify: bool,
) -> Result<Vec<InstalledJavaRuntime>, LauncherError> {
    let runtimes_root = runtime_paths.data_dir().join("runtimes");
    if !runtimes_root.exists() {
        return Ok(Vec::new());
    }
    let mut directories = std::fs::read_dir(&runtimes_root)?.collect::<Result<Vec<_>, _>>()?;
    directories.sort_by_key(std::fs::DirEntry::file_name);
    let mut runtimes = Vec::new();
    for directory in directories {
        if !directory.file_type()?.is_dir() {
            continue;
        }
        let root = directory.path();
        if root.file_name().and_then(|name| name.to_str()) == Some(RUNTIME_STAGING_DIRECTORY) {
            continue;
        }
        let inventory = JavaRuntimeInventory::load(&root.join(RUNTIME_INVENTORY_FILE))?;
        if root.file_name().and_then(|name| name.to_str()) != Some(&inventory.runtime_id) {
            return Err(LauncherError::InvalidLock(format!(
                "Java runtime directory '{}' does not match inventory ID '{}'",
                root.display(),
                inventory.runtime_id
            )));
        }
        if verify {
            inventory.verify_files(&root)?;
        }
        let expected_executable = if inventory.platform.starts_with("windows") {
            "bin/java.exe"
        } else {
            "bin/java"
        };
        let executable = inventory
            .files
            .iter()
            .find(|file| file.executable && file.path == expected_executable)
            .ok_or_else(|| {
                LauncherError::InvalidLock(format!(
                    "managed Java runtime '{}' has no executable",
                    inventory.runtime_id
                ))
            })?;
        let bytes = inventory.files.iter().try_fold(0_u64, |total, file| {
            total.checked_add(file.size).ok_or_else(|| {
                LauncherError::InvalidLock(format!(
                    "managed Java runtime '{}' size overflows",
                    inventory.runtime_id
                ))
            })
        })?;
        runtimes.push(InstalledJavaRuntime {
            runtime_id: inventory.runtime_id,
            provider: inventory.provider,
            component: inventory.component,
            platform: inventory.platform,
            version: inventory.version,
            major: inventory.major,
            root: root.clone(),
            executable: root.join(path_from_portable(&executable.path)),
            files: inventory.files.len(),
            bytes,
            verified: verify.then_some(true),
        });
    }
    Ok(runtimes)
}

pub fn verify_managed_java_runtime(
    runtime_paths: &RuntimePaths,
    runtime_id: &str,
) -> Result<InstalledJavaRuntime, LauncherError> {
    list_managed_java_runtimes(runtime_paths, true)?
        .into_iter()
        .find(|runtime| runtime.runtime_id == runtime_id)
        .ok_or_else(|| LauncherError::JavaRuntimeNotFound(runtime_id.to_string()))
}

pub fn remove_managed_java_runtime(
    runtime_paths: &RuntimePaths,
    runtime_id: &str,
) -> Result<InstalledJavaRuntime, LauncherError> {
    validate_runtime_id(runtime_id)?;
    let runtime = list_managed_java_runtimes(runtime_paths, false)?
        .into_iter()
        .find(|runtime| runtime.runtime_id == runtime_id)
        .ok_or_else(|| LauncherError::JavaRuntimeNotFound(runtime_id.to_string()))?;
    let registry = crate::registry::InstanceRegistry::load(&runtime_paths.instances_file())?;
    let mut used_by = Vec::new();
    for instance in &registry.instances {
        let Some(lock) = LockFile::open_optional(&instance.root)? else {
            continue;
        };
        if lock
            .inner
            .java
            .as_ref()
            .is_some_and(|java| java.runtime_id == runtime_id)
        {
            used_by.push(instance.name.clone());
        }
    }
    if !used_by.is_empty() {
        return Err(LauncherError::JavaRuntimeInUse {
            runtime_id: runtime_id.to_string(),
            instances: used_by.join(", "),
        });
    }

    let runtimes_root = dunce::canonicalize(runtime_paths.data_dir().join("runtimes"))?;
    let target = dunce::canonicalize(&runtime.root)?;
    if target.parent() != Some(runtimes_root.as_path()) {
        return Err(LauncherError::InvalidConfig(format!(
            "managed Java runtime '{}' resolves outside the runtime directory",
            runtime_id
        )));
    }
    std::fs::remove_dir_all(&target)?;
    Ok(runtime)
}

fn validate_runtime_id(runtime_id: &str) -> Result<(), LauncherError> {
    let mut components = Path::new(runtime_id).components();
    let valid = matches!(components.next(), Some(Component::Normal(value)) if value == runtime_id)
        && components.next().is_none()
        && !runtime_id.is_empty()
        && !runtime_id.chars().any(char::is_control);
    if valid {
        Ok(())
    } else {
        Err(LauncherError::InvalidConfig(format!(
            "managed Java runtime ID '{runtime_id}' is invalid"
        )))
    }
}

/// Verifies that a lock still refers to the exact managed runtime installed in
/// the launcher data directory, then returns its executable path.
pub fn verify_locked_java_runtime(
    runtime_paths: &RuntimePaths,
    locked: &LockedJavaRuntime,
) -> Result<PathBuf, LauncherError> {
    let root = runtime_paths
        .data_dir()
        .join("runtimes")
        .join(&locked.runtime_id);
    let inventory = JavaRuntimeInventory::load(&root.join(RUNTIME_INVENTORY_FILE))?;
    if inventory.runtime_id != locked.runtime_id
        || inventory.provider != locked.provider
        || inventory.platform != locked.platform
        || inventory.version != locked.version
        || inventory.major != locked.major
    {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "managed Java runtime '{}' does not match orbit-launcher.lock",
            locked.runtime_id
        )));
    }
    inventory.verify_files(&root)?;
    let executable = inventory
        .files
        .iter()
        .find(|file| file.path == locked.executable && file.executable)
        .ok_or_else(|| {
            LauncherError::ArtifactIntegrity(format!(
                "managed Java runtime '{}' does not inventory '{}' as an executable",
                locked.runtime_id, locked.executable
            ))
        })?;
    let path = root.join(path_from_portable(&executable.path));
    if !path.is_file() {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "managed Java runtime '{}' is missing its executable",
            locked.runtime_id
        )));
    }
    Ok(path)
}

pub async fn plan_mojang_java<F>(
    client: &reqwest::Client,
    requirement: &MojangJavaRequirement,
    target: JavaTarget,
    mut progress: F,
) -> Result<MojangJavaPlan, LauncherError>
where
    F: FnMut(JavaProgressEvent),
{
    progress(JavaProgressEvent::ManifestStarted);
    let all_bytes = fetch_metadata(client, MOJANG_RUNTIME_MANIFEST_URL).await?;
    let all: RuntimeCatalog = serde_json::from_slice(&all_bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse Mojang Java runtime catalog: {error}"
        ))
    })?;
    let runtime = all
        .get(target.mojang_name())
        .and_then(|components| components.get(&requirement.component))
        .and_then(|versions| versions.first())
        .ok_or_else(|| {
            LauncherError::UnsupportedRequirement(format!(
                "Mojang Java component '{}' is unavailable for {}",
                requirement.component,
                target.mojang_name()
            ))
        })?;
    runtime.manifest.validate("Java runtime manifest")?;
    validate_mojang_url(&runtime.manifest.url, "Java runtime manifest")?;
    if runtime.version.name.trim().is_empty() {
        return Err(LauncherError::InvalidRemoteData(
            "Mojang Java runtime version is empty".to_string(),
        ));
    }
    let manifest_bytes = fetch_metadata(client, &runtime.manifest.url).await?;
    if hex::encode(Sha1::digest(&manifest_bytes)) != runtime.manifest.sha1 {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "Mojang Java manifest for '{}' did not match its SHA-1",
            requirement.component
        )));
    }
    let manifest: RuntimeFileManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            LauncherError::InvalidRemoteData(format!(
                "failed to parse Mojang Java component manifest: {error}"
            ))
        })?;
    let entries = plan_entries(manifest)?;
    let runtime_id = format!(
        "mojang-{}-{}-{}-{}",
        target.mojang_name(),
        requirement.component,
        sanitize_id(&runtime.version.name),
        &runtime.manifest.sha1[..12]
    );
    let plan = MojangJavaPlan {
        target,
        component: requirement.component.clone(),
        major: requirement.major,
        version: runtime.version.name.clone(),
        runtime_id,
        manifest_url: runtime.manifest.url.clone(),
        manifest_sha1: runtime.manifest.sha1.clone(),
        entries,
    };
    progress(JavaProgressEvent::RuntimeResolved {
        runtime_id: plan.runtime_id.clone(),
        artifacts: plan.artifact_count(),
        total_bytes: plan.total_download_bytes(),
    });
    Ok(plan)
}

pub async fn install_mojang_java<F>(
    runtime_paths: &RuntimePaths,
    client: &reqwest::Client,
    plan: MojangJavaPlan,
    concurrency: usize,
    progress: F,
) -> Result<ManagedJavaRuntime, LauncherError>
where
    F: FnMut(JavaProgressEvent) + Send,
{
    if concurrency == 0 {
        return Err(LauncherError::InvalidConfig(
            "Java download concurrency must be greater than zero".to_string(),
        ));
    }
    let runtimes_root = runtime_paths.data_dir().join("runtimes");
    let final_root = runtimes_root.join(&plan.runtime_id);
    if final_root.is_dir() {
        let artifact_count = plan.artifact_count();
        let locked = verify_existing_runtime(&final_root, &plan)?;
        let mut progress = progress;
        progress(JavaProgressEvent::RuntimeCached {
            runtime_id: plan.runtime_id,
        });
        return Ok(ManagedJavaRuntime {
            locked,
            root: final_root,
            downloaded_artifacts: 0,
            cached_artifacts: artifact_count,
        });
    }

    let staging = runtimes_root
        .join(RUNTIME_STAGING_DIRECTORY)
        .join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&staging)?;
    let _staging_guard = StagingDirectoryGuard(staging.clone());
    let progress = Arc::new(Mutex::new(progress));
    let cache = ArtifactCache::new(runtime_paths.cache_dir());
    let files: Vec<_> = plan
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            PlannedEntryKind::File {
                request,
                executable,
            } => Some((entry.path.clone(), request.clone(), *executable)),
            _ => None,
        })
        .collect();
    let mut downloads =
        futures_util::stream::iter(files.into_iter().map(|(path, request, executable)| {
            let cache = cache.clone();
            let client = client.clone();
            let progress = Arc::clone(&progress);
            async move {
                let artifact = cache
                    .fetch(&client, &request, |event| {
                        if let Ok(mut progress) = progress.lock() {
                            progress(JavaProgressEvent::Artifact(event));
                        }
                    })
                    .await?;
                Ok::<_, LauncherError>((path, request, executable, artifact))
            }
        }))
        .buffer_unordered(concurrency);

    let mut downloaded = 0;
    let mut cached = 0;
    let mut artifacts = BTreeMap::new();
    while let Some(result) = downloads.next().await {
        let (path, request, executable, artifact) = result?;
        downloaded += usize::from(!artifact.cache_hit);
        cached += usize::from(artifact.cache_hit);
        artifacts.insert(path, (request, executable, artifact));
    }
    cache.flush()?;

    materialize_runtime(&staging, &plan, &cache, &artifacts, &progress)?;
    let inventory = JavaRuntimeInventory::from_plan(&plan, &artifacts);
    inventory.save(&staging.join(RUNTIME_INVENTORY_FILE))?;
    let locked = verify_runtime(&staging, &plan)?;
    emit(
        &progress,
        JavaProgressEvent::RuntimeVerified {
            runtime_id: plan.runtime_id.clone(),
        },
    )?;
    std::fs::create_dir_all(&runtimes_root)?;
    match std::fs::rename(&staging, &final_root) {
        Ok(()) => {}
        Err(_error) if final_root.is_dir() => {
            std::fs::remove_dir_all(&staging)?;
            verify_existing_runtime(&final_root, &plan)?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(ManagedJavaRuntime {
        locked,
        root: final_root,
        downloaded_artifacts: downloaded,
        cached_artifacts: cached,
    })
}

fn materialize_runtime<F>(
    staging: &Path,
    plan: &MojangJavaPlan,
    cache: &ArtifactCache,
    artifacts: &BTreeMap<String, (ArtifactRequest, bool, crate::artifact::CachedArtifact)>,
    progress: &Arc<Mutex<F>>,
) -> Result<(), LauncherError>
where
    F: FnMut(JavaProgressEvent),
{
    for entry in &plan.entries {
        if matches!(entry.kind, PlannedEntryKind::Directory) {
            std::fs::create_dir_all(staging.join(path_from_portable(&entry.path)))?;
        }
    }
    let total = artifacts.len();
    for (index, (path, (_, executable, artifact))) in artifacts.iter().enumerate() {
        let destination = staging.join(path_from_portable(path));
        cache.materialize(artifact, &destination)?;
        set_executable(&destination, *executable)?;
        emit(
            progress,
            JavaProgressEvent::Materialized {
                completed: index + 1,
                total,
            },
        )?;
    }
    for entry in &plan.entries {
        if let PlannedEntryKind::Link { target } = &entry.kind {
            create_runtime_link(staging, &entry.path, target)?;
        }
    }
    Ok(())
}

fn verify_existing_runtime(
    root: &Path,
    plan: &MojangJavaPlan,
) -> Result<LockedJavaRuntime, LauncherError> {
    let inventory = JavaRuntimeInventory::load(&root.join(RUNTIME_INVENTORY_FILE))?;
    if inventory.runtime_id != plan.runtime_id
        || inventory.manifest_sha1 != plan.manifest_sha1
        || inventory.component != plan.component
    {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "managed Java runtime '{}' has mismatched inventory",
            plan.runtime_id
        )));
    }
    inventory.verify_files(root)?;
    for entry in &plan.entries {
        let path = root.join(path_from_portable(&entry.path));
        let valid = match &entry.kind {
            PlannedEntryKind::Directory => path.is_dir(),
            PlannedEntryKind::File { .. } => path.is_file(),
            PlannedEntryKind::Link { .. } => std::fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.file_type().is_symlink()),
        };
        if !valid {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "managed Java runtime '{}' is missing or changed at '{}'",
                plan.runtime_id, entry.path
            )));
        }
    }
    verify_runtime(root, plan)
}

fn verify_runtime(root: &Path, plan: &MojangJavaPlan) -> Result<LockedJavaRuntime, LauncherError> {
    let executable_relative = plan.target.executable();
    let executable = root.join(path_from_portable(executable_relative));
    if !executable.is_file() {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "managed Java runtime '{}' is missing {}",
            plan.runtime_id, executable_relative
        )));
    }
    let output = Command::new(&executable)
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .map_err(|error| {
            LauncherError::Transaction(format!(
                "failed to execute managed Java '{}': {error}",
                executable.display()
            ))
        })?;
    if !output.status.success() {
        return Err(LauncherError::Transaction(format!(
            "managed Java '{}' exited with {} during verification",
            executable.display(),
            output.status
        )));
    }
    let properties = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let java_version = property(&properties, "java.version").ok_or_else(|| {
        LauncherError::InvalidRemoteData(
            "managed Java did not report the java.version property".to_string(),
        )
    })?;
    let os_arch = property(&properties, "os.arch").ok_or_else(|| {
        LauncherError::InvalidRemoteData(
            "managed Java did not report the os.arch property".to_string(),
        )
    })?;
    let actual_major = parse_java_major(java_version)?;
    if actual_major != plan.major || !java_version.starts_with(&plan.version) {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "managed Java reported version '{java_version}', expected {} (major {})",
            plan.version, plan.major
        )));
    }
    if !plan.target.expected_os_arch().contains(&os_arch) {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "managed Java reported architecture '{os_arch}', expected {}",
            plan.target.mojang_name()
        )));
    }
    Ok(LockedJavaRuntime {
        runtime_id: plan.runtime_id.clone(),
        provider: "mojang".to_string(),
        version: plan.version.clone(),
        major: plan.major,
        platform: plan.target.mojang_name().to_string(),
        executable: executable_relative.to_string(),
    })
}

fn property<'a>(output: &'a str, name: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        (key.trim() == name).then(|| value.trim())
    })
}

fn parse_java_major(version: &str) -> Result<u32, LauncherError> {
    let first = version.split(['.', '-']).next().unwrap_or_default();
    let first = first.parse::<u32>().map_err(|_| {
        LauncherError::InvalidRemoteData(format!("Java version '{version}' is invalid"))
    })?;
    if first == 1 {
        version
            .split('.')
            .nth(1)
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| {
                LauncherError::InvalidRemoteData(format!("Java version '{version}' is invalid"))
            })
    } else {
        Ok(first)
    }
}

fn plan_entries(manifest: RuntimeFileManifest) -> Result<Vec<PlannedEntry>, LauncherError> {
    let mut entries = Vec::with_capacity(manifest.files.len());
    let mut paths = HashSet::new();
    for (path, entry) in manifest.files {
        validate_portable_path(&path)?;
        if !paths.insert(path.clone()) {
            return Err(LauncherError::InvalidRemoteData(format!(
                "duplicate Java runtime path '{path}'"
            )));
        }
        let kind = match entry {
            RuntimeFile::Directory => PlannedEntryKind::Directory,
            RuntimeFile::File {
                downloads,
                executable,
            } => {
                downloads.raw.validate("Java runtime file")?;
                validate_mojang_url(&downloads.raw.url, "Java runtime file")?;
                PlannedEntryKind::File {
                    request: ArtifactRequest {
                        logical_name: format!("Java runtime {path}"),
                        url: downloads.raw.url,
                        expected_hash: ExpectedHash::Sha1(downloads.raw.sha1),
                        expected_size: Some(downloads.raw.size),
                    },
                    executable,
                }
            }
            RuntimeFile::Link { target } => {
                validate_link_target(&path, &target)?;
                PlannedEntryKind::Link { target }
            }
        };
        entries.push(PlannedEntry { path, kind });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

#[derive(Debug, Clone)]
struct PlannedEntry {
    path: String,
    kind: PlannedEntryKind,
}

#[derive(Debug, Clone)]
enum PlannedEntryKind {
    Directory,
    File {
        request: ArtifactRequest,
        executable: bool,
    },
    Link {
        target: String,
    },
}

type RuntimeCatalog = BTreeMap<String, BTreeMap<String, Vec<RuntimeVersion>>>;

#[derive(Debug, Deserialize)]
struct RuntimeVersion {
    manifest: RuntimeDownload,
    version: RuntimeVersionName,
}

#[derive(Debug, Deserialize)]
struct RuntimeVersionName {
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeDownload {
    sha1: String,
    size: u64,
    url: String,
}

impl RuntimeDownload {
    fn validate(&self, subject: &str) -> Result<(), LauncherError> {
        if self.sha1.len() != 40
            || !self
                .sha1
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.size == 0
        {
            return Err(LauncherError::InvalidRemoteData(format!(
                "Mojang {subject} integrity metadata is invalid"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeFileManifest {
    files: BTreeMap<String, RuntimeFile>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RuntimeFile {
    Directory,
    File {
        downloads: RuntimeDownloads,
        #[serde(default)]
        executable: bool,
    },
    Link {
        target: String,
    },
}

#[derive(Debug, Deserialize)]
struct RuntimeDownloads {
    raw: RuntimeDownload,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JavaRuntimeInventory {
    schema: u32,
    runtime_id: String,
    provider: String,
    component: String,
    platform: String,
    version: String,
    major: u32,
    manifest_url: String,
    manifest_sha1: String,
    files: Vec<RuntimeInventoryFile>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeInventoryFile {
    path: String,
    source_url: String,
    upstream_sha1: String,
    sha256: String,
    size: u64,
    executable: bool,
}

impl JavaRuntimeInventory {
    fn from_plan(
        plan: &MojangJavaPlan,
        artifacts: &BTreeMap<String, (ArtifactRequest, bool, crate::artifact::CachedArtifact)>,
    ) -> Self {
        Self {
            schema: 1,
            runtime_id: plan.runtime_id.clone(),
            provider: "mojang".to_string(),
            component: plan.component.clone(),
            platform: plan.target.mojang_name().to_string(),
            version: plan.version.clone(),
            major: plan.major,
            manifest_url: plan.manifest_url.clone(),
            manifest_sha1: plan.manifest_sha1.clone(),
            files: artifacts
                .iter()
                .map(
                    |(path, (request, executable, artifact))| RuntimeInventoryFile {
                        path: path.clone(),
                        source_url: request.url.clone(),
                        upstream_sha1: match &request.expected_hash {
                            ExpectedHash::Sha1(value) => value.clone(),
                            _ => unreachable!("Mojang Java files always use SHA-1"),
                        },
                        sha256: artifact.sha256.clone(),
                        size: artifact.size,
                        executable: *executable,
                    },
                )
                .collect(),
        }
    }

    fn load(path: &Path) -> Result<Self, LauncherError> {
        let content = std::fs::read_to_string(path)?;
        let inventory: Self = toml::from_str(&content).map_err(LauncherError::LockParse)?;
        if inventory.schema != 1 {
            return Err(LauncherError::InvalidLock(format!(
                "unsupported Java runtime inventory schema {}",
                inventory.schema
            )));
        }
        Ok(inventory)
    }

    fn save(&self, path: &Path) -> Result<(), LauncherError> {
        let content = toml::to_string_pretty(self)?;
        write_atomic(path, content.as_bytes())
    }

    fn verify_files(&self, root: &Path) -> Result<(), LauncherError> {
        for file in &self.files {
            validate_portable_path(&file.path)?;
            let path = root.join(path_from_portable(&file.path));
            let metadata = std::fs::metadata(&path).map_err(|error| {
                LauncherError::ArtifactIntegrity(format!(
                    "managed Java runtime '{}' cannot read '{}': {error}",
                    self.runtime_id, file.path
                ))
            })?;
            let actual_hash = crate::artifact::hash_file_sha256(&path).map_err(|error| {
                LauncherError::ArtifactIntegrity(format!(
                    "managed Java runtime '{}' cannot hash '{}': {error}",
                    self.runtime_id, file.path
                ))
            })?;
            if metadata.len() != file.size || actual_hash != file.sha256 {
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "managed Java runtime '{}' file '{}' failed verification",
                    self.runtime_id, file.path
                )));
            }
        }
        Ok(())
    }
}

struct StagingDirectoryGuard(PathBuf);

impl Drop for StagingDirectoryGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn fetch_metadata(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, LauncherError> {
    let response = client.get(url).send().await?.error_for_status()?;
    if response.url().scheme() != "https"
        || response
            .content_length()
            .is_some_and(|size| size > MAX_METADATA_BYTES)
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Mojang Java metadata URL '{}' or size is invalid",
            response.url()
        )));
    }
    let bytes = response.bytes().await?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Mojang Java metadata '{url}' exceeds {MAX_METADATA_BYTES} bytes"
        )));
    }
    Ok(bytes.to_vec())
}

fn validate_mojang_url(value: &str, subject: &str) -> Result<(), LauncherError> {
    let url = url::Url::parse(value).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("Mojang {subject} URL is invalid: {error}"))
    })?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("piston-meta.mojang.com" | "piston-data.mojang.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Mojang {subject} URL '{value}' is not an allowed official HTTPS URL"
        )));
    }
    Ok(())
}

fn validate_portable_path(path: &str) -> Result<(), LauncherError> {
    if path.is_empty()
        || path.contains('\\')
        || path.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Java runtime path '{path}' is not a normalized relative path"
        )));
    }
    Ok(())
}

fn validate_link_target(path: &str, target: &str) -> Result<(), LauncherError> {
    if target.is_empty() || target.contains('\\') || target.starts_with('/') {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Java runtime link '{path}' has unsafe target '{target}'"
        )));
    }
    let mut depth = path.split('/').count().saturating_sub(1);
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." if depth > 0 => depth -= 1,
            ".." => {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "Java runtime link '{path}' escapes the runtime root"
                )));
            }
            value if value.chars().any(char::is_control) => {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "Java runtime link '{path}' has invalid target"
                )));
            }
            _ => depth += 1,
        }
    }
    Ok(())
}

fn path_from_portable(value: &str) -> PathBuf {
    value.split('/').collect()
}

#[cfg(unix)]
fn create_runtime_link(root: &Path, path: &str, target: &str) -> Result<(), LauncherError> {
    use std::os::unix::fs::symlink;
    let link = root.join(path_from_portable(path));
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    symlink(target, link)?;
    Ok(())
}

#[cfg(windows)]
fn create_runtime_link(_root: &Path, path: &str, _target: &str) -> Result<(), LauncherError> {
    Err(LauncherError::UnsupportedRequirement(format!(
        "Mojang unexpectedly published symbolic link '{path}' for a Windows Java runtime"
    )))
}

#[cfg(not(any(unix, windows)))]
fn create_runtime_link(_root: &Path, path: &str, _target: &str) -> Result<(), LauncherError> {
    Err(LauncherError::UnsupportedRequirement(format!(
        "symbolic link '{path}' is unsupported on this platform"
    )))
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), LauncherError> {
    use std::os::unix::fs::PermissionsExt;
    if executable {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), LauncherError> {
    Ok(())
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn emit<F>(progress: &Arc<Mutex<F>>, event: JavaProgressEvent) -> Result<(), LauncherError>
where
    F: FnMut(JavaProgressEvent),
{
    let mut progress = progress.lock().map_err(|_| {
        LauncherError::Transaction("Java progress callback lock was poisoned".to_string())
    })?;
    progress(event);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eula::{EulaAcceptance, EulaAcceptanceMethod};
    use crate::instance::{InstanceKind, LoaderKind};
    use crate::lockfile::{
        ArtifactOwner, LOCK_SCHEMA, LauncherLock, LockedArguments, LockedArtifact,
        LockedArtifactSource, LockedEntrypoint, LockedJavaRuntime, LockedLoader, LockedMinecraft,
    };
    use crate::operations::{CreateInstanceRequest, create_instance};

    fn test_paths(root: &Path) -> RuntimePaths {
        RuntimePaths::resolve(&crate::runtime::RuntimePathOptions {
            config_dir: Some(root.join("config")),
            data_dir: Some(root.join("data")),
            cache_dir: Some(root.join("cache")),
        })
        .unwrap()
    }

    fn write_runtime_fixture(paths: &RuntimePaths) -> PathBuf {
        let root = paths.data_dir().join("runtimes/runtime-21");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let executable = root.join(if cfg!(windows) {
            "bin/java.exe"
        } else {
            "bin/java"
        });
        std::fs::write(&executable, b"managed-java").unwrap();
        let relative = if cfg!(windows) {
            "bin/java.exe"
        } else {
            "bin/java"
        };
        JavaRuntimeInventory {
            schema: 1,
            runtime_id: "runtime-21".to_string(),
            provider: "mojang".to_string(),
            component: "java-runtime-delta".to_string(),
            platform: if cfg!(windows) {
                "windows-x64".to_string()
            } else {
                "linux".to_string()
            },
            version: "21.0.7".to_string(),
            major: 21,
            manifest_url: "https://example.invalid/manifest.json".to_string(),
            manifest_sha1: "0".repeat(40),
            files: vec![RuntimeInventoryFile {
                path: relative.to_string(),
                source_url: "https://example.invalid/java".to_string(),
                upstream_sha1: "0".repeat(40),
                sha256: crate::artifact::hash_file_sha256(&executable).unwrap(),
                size: 12,
                executable: true,
            }],
        }
        .save(&root.join(RUNTIME_INVENTORY_FILE))
        .unwrap();
        root
    }

    fn write_server_lock(root: &Path, instance_id: Uuid) {
        LockFile::new(
            root,
            LauncherLock {
                schema: LOCK_SCHEMA,
                instance_id,
                kind: InstanceKind::Server,
                minecraft: LockedMinecraft {
                    version: "1.21.1".to_string(),
                    version_type: "release".to_string(),
                    asset_index: None,
                    version_manifest_url:
                        "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
                            .to_string(),
                    version_manifest_sha256: "a".repeat(64),
                    version_json_url: "https://piston-meta.mojang.com/version.json".to_string(),
                    version_json_sha1: "b".repeat(40),
                },
                loader: LockedLoader::vanilla(),
                java: Some(LockedJavaRuntime {
                    runtime_id: "runtime-21".to_string(),
                    provider: "mojang".to_string(),
                    version: "21.0.7".to_string(),
                    major: 21,
                    platform: if cfg!(windows) {
                        "windows-x64".to_string()
                    } else {
                        "linux".to_string()
                    },
                    executable: if cfg!(windows) {
                        "bin/java.exe".to_string()
                    } else {
                        "bin/java".to_string()
                    },
                }),
                authlib_injector: None,
                entrypoint: LockedEntrypoint::Jar {
                    path: "server.jar".to_string(),
                },
                arguments: LockedArguments::default(),
                artifacts: vec![LockedArtifact {
                    logical_name: "Minecraft server".to_string(),
                    owner: ArtifactOwner::Minecraft,
                    source: LockedArtifactSource::Download {
                        url: "https://piston-data.mojang.com/server.jar".to_string(),
                        upstream_sha1: Some("c".repeat(40)),
                    },
                    sha256: "d".repeat(64),
                    size: 100,
                    path: "server.jar".to_string(),
                }],
                generated_files: vec!["eula.txt".to_string()],
                eula: Some(EulaAcceptance {
                    url: crate::eula::MINECRAFT_EULA_URL.to_string(),
                    digest_sha256: "e".repeat(64),
                    accepted_at_unix_seconds: 1,
                    method: EulaAcceptanceMethod::DigestCommand,
                }),
            },
        )
        .save()
        .unwrap();
    }

    #[test]
    fn link_validation_keeps_every_target_inside_the_runtime() {
        assert!(validate_link_target("legal/a/LICENSE", "../base/LICENSE").is_ok());
        assert!(validate_link_target("legal/LICENSE", "../../outside").is_err());
        assert!(validate_link_target("bin/java", "/absolute").is_err());
    }

    #[test]
    fn java_major_parser_handles_legacy_and_current_versions() {
        assert_eq!(parse_java_major("1.8.0_51").unwrap(), 8);
        assert_eq!(parse_java_major("21.0.7").unwrap(), 21);
        assert_eq!(parse_java_major("25-ea").unwrap(), 25);
    }

    #[test]
    fn runtime_id_sanitization_cannot_create_directories() {
        assert_eq!(sanitize_id("21/../../evil"), "21_.._.._evil");
        assert!(validate_runtime_id("../evil").is_err());
        assert!(validate_runtime_id("runtime-21").is_ok());
    }

    #[test]
    fn managed_runtime_list_verifies_and_removes_unreferenced_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        let root = write_runtime_fixture(&paths);
        std::fs::create_dir_all(
            paths
                .data_dir()
                .join("runtimes")
                .join(RUNTIME_STAGING_DIRECTORY),
        )
        .unwrap();

        let listed = list_managed_java_runtimes(&paths, true).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].major, 21);
        assert_eq!(listed[0].verified, Some(true));
        remove_managed_java_runtime(&paths, "runtime-21").unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn managed_runtime_removal_refuses_a_locked_instance_reference() {
        let directory = tempfile::tempdir().unwrap();
        let paths = test_paths(directory.path());
        let runtime_root = write_runtime_fixture(&paths);
        let instance_root = directory.path().join("server");
        let created = create_instance(
            &paths,
            CreateInstanceRequest {
                root: instance_root.clone(),
                name: "server".to_string(),
                kind: InstanceKind::Server,
                minecraft_requirement: "1.21.1".to_string(),
                loader_kind: LoaderKind::Vanilla,
                loader_requirement: None,
            },
        )
        .unwrap();
        write_server_lock(&instance_root, created.entry.id);

        let error = remove_managed_java_runtime(&paths, "runtime-21").unwrap_err();
        assert!(matches!(
            error,
            LauncherError::JavaRuntimeInUse { runtime_id, instances }
                if runtime_id == "runtime-21" && instances == "server"
        ));
        assert!(runtime_root.exists());
    }
}
