use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use uuid::Uuid;

use crate::artifact::{ArtifactCache, ArtifactRequest, ArtifactTransferEvent, ExpectedHash};
use crate::atomic_io::write_atomic;
use crate::error::LauncherError;
use crate::lockfile::LockedJavaRuntime;
use crate::mojang::MojangJavaRequirement;
use crate::platform::{Architecture, HostPlatform, OperatingSystem};
use crate::runtime::RuntimePaths;

pub const MOJANG_RUNTIME_MANIFEST_URL: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const RUNTIME_INVENTORY_FILE: &str = "orbit-launcher-runtime.toml";

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
        .join(".staging")
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
            if std::fs::metadata(&path).is_err()
                || std::fs::metadata(&path)?.len() != file.size
                || crate::artifact::hash_file_sha256(&path)? != file.sha256
            {
                return Err(LauncherError::ArtifactIntegrity(format!(
                    "managed Java runtime file '{}' failed verification",
                    file.path
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
    }
}
