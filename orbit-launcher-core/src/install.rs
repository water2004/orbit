use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::artifact::{
    ArtifactCache, ArtifactTransferEvent, CachedArtifact, ExpectedHash, hash_file_sha256,
};
use crate::atomic_io::write_atomic;
use crate::authlib_injector::{
    ResolvedAuthlibInjector, resolve_authlib_injector, verify_authlib_injector,
};
use crate::client::{ClientDownload, ResolvedVanillaClient, resolve_vanilla_client};
use crate::error::LauncherError;
use crate::eula::{EulaAcceptance, EulaDocument, require_current_acceptance, show_current_eula};
use crate::installer::{
    INSTALLER_STAGING_NAME, InstallerSide, LoaderInstallerEvent, ResolvedLoaderInstaller,
    inspect_loader_installer, installed_server_argument_file, read_installed_client_profile,
    resolve_loader_installer, run_loader_installer,
};
use crate::instance::{
    InstanceKind, InstanceManifest, LoaderKind, ManifestFile, ServerAuthenticationProvider,
};
use crate::java::{
    JavaProgressEvent, JavaTarget, MojangJavaPlan, install_mojang_java, plan_mojang_java,
};
use crate::layout::{INSTANCE_MINECRAFT_JAR, InstanceLocation};
use crate::loader::{LoaderSide, ResolvedLoaderProfile, resolve_loader_profile};
use crate::lockfile::{
    ArtifactOwner, INSTANCE_LOCK_FILE, LOCK_SCHEMA, LauncherLock, LockFile, LockedArguments,
    LockedArtifact, LockedArtifactSource, LockedAuthlibInjector, LockedEntrypoint, LockedLoader,
    LockedMinecraft, portable_relative_path,
};
use crate::mojang::{MojangClient, ResolvedVanillaServer, VERSION_MANIFEST_V2_URL};
use crate::platform::HostPlatform;
use crate::runtime::RuntimePaths;

const STATE_DIRECTORY: &str = ".orbit-launcher";
const TRANSACTION_LOCK: &str = "transaction.lock";
const TRANSACTION_JOURNAL: &str = "transaction.json";

#[derive(Debug, Clone)]
pub struct ServerInstallPlan {
    instance_id: Uuid,
    minecraft_requirement: String,
    loader_requirement: Option<String>,
    resolved: ResolvedVanillaServer,
    java: MojangJavaPlan,
    eula: EulaDocument,
    acceptance: Option<EulaAcceptance>,
    loader: PlannedLoader,
    authlib_injector: Option<ResolvedAuthlibInjector>,
}

#[derive(Debug, Clone)]
pub struct ClientInstallPlan {
    instance_id: Uuid,
    minecraft_requirement: String,
    loader_requirement: Option<String>,
    resolved: ResolvedVanillaClient,
    java: MojangJavaPlan,
    loader: PlannedLoader,
    authlib_injector: Option<ResolvedAuthlibInjector>,
}

#[derive(Debug, Clone)]
enum PlannedLoader {
    Vanilla,
    Profile(ResolvedLoaderProfile),
    Installer(ResolvedLoaderInstaller),
}

#[derive(Debug, Clone)]
pub enum InstallPlan {
    Client(ClientInstallPlan),
    Server(ServerInstallPlan),
}

impl InstallPlan {
    pub const fn server(&self) -> Option<&ServerInstallPlan> {
        match self {
            Self::Client(_) => None,
            Self::Server(plan) => Some(plan),
        }
    }
}

impl ClientInstallPlan {
    pub fn minecraft_version(&self) -> &str {
        &self.resolved.minecraft_version
    }

    pub fn java_major(&self) -> Option<u32> {
        self.resolved.java.as_ref().map(|java| java.major)
    }

    pub fn artifact_count(&self) -> usize {
        self.resolved.downloads.len() + 1 + usize::from(self.authlib_injector.is_some())
    }

    pub fn download_size(&self) -> Option<u64> {
        if self.authlib_injector.is_some() {
            return None;
        }
        self.resolved
            .downloads
            .iter()
            .try_fold(0_u64, |total, download| {
                download
                    .request
                    .expected_size
                    .and_then(|size| total.checked_add(size))
            })
    }
}

impl ServerInstallPlan {
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
        self.authlib_injector
            .is_none()
            .then_some(self.resolved.server.expected_size)
            .flatten()
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
    LoaderInstaller(LoaderInstallerEvent),
    StagingVerified,
    Committed,
}

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub lock: LauncherLock,
    pub downloaded_artifacts: usize,
    pub cached_artifacts: usize,
}

async fn plan_authlib_injector(
    manifest: &InstanceManifest,
    client: &reqwest::Client,
) -> Result<Option<ResolvedAuthlibInjector>, LauncherError> {
    if manifest_requires_authlib_injector(manifest) {
        resolve_authlib_injector(client).await.map(Some)
    } else {
        Ok(None)
    }
}

fn manifest_requires_authlib_injector(manifest: &InstanceManifest) -> bool {
    match manifest.kind {
        InstanceKind::Client => true,
        InstanceKind::Server => manifest.server.as_ref().is_some_and(|server| {
            server.authentication.provider == ServerAuthenticationProvider::ExternalYggdrasil
        }),
    }
}

fn append_authlib_download(
    downloads: &mut Vec<ClientDownload>,
    authlib_injector: Option<&ResolvedAuthlibInjector>,
) {
    if let Some(authlib_injector) = authlib_injector {
        downloads.push(ClientDownload {
            request: authlib_injector.request.clone(),
            target: authlib_injector.target.clone(),
            owner: ArtifactOwner::AuthlibInjector,
            native_extract: None,
        });
    }
}

fn lock_authlib_injector(
    authlib_injector: Option<&ResolvedAuthlibInjector>,
    artifacts: &BTreeMap<String, (ClientDownload, CachedArtifact)>,
) -> Result<Option<LockedAuthlibInjector>, LauncherError> {
    let Some(resolved) = authlib_injector else {
        return Ok(None);
    };
    let (_, artifact) = artifacts.get(&resolved.target).ok_or_else(|| {
        LauncherError::Transaction(
            "resolved authlib-injector is missing from the download transaction".to_string(),
        )
    })?;
    verify_authlib_injector(&artifact.object_path, resolved, artifact)?;
    Ok(Some(LockedAuthlibInjector {
        version: resolved.version.clone(),
        build_number: resolved.build_number,
        path: resolved.target.clone(),
    }))
}

async fn prepare_vanilla_client_install<F>(
    instance_root: &Path,
    client: &reqwest::Client,
    mut progress: F,
) -> Result<ClientInstallPlan, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    let manifest = ManifestFile::open(instance_root)?.inner;
    require_vanilla_client(&manifest)?;
    progress(InstallProgressEvent::MetadataStarted);
    let resolved = resolve_vanilla_client(
        &MojangClient::new(client.clone()),
        &manifest.minecraft.requirement,
        &HostPlatform::native()?,
    )
    .await?;
    let authlib_injector = plan_authlib_injector(&manifest, client).await?;
    progress(InstallProgressEvent::MinecraftResolved {
        version: resolved.minecraft_version.clone(),
        total_artifacts: resolved.downloads.len() + 1 + usize::from(authlib_injector.is_some()),
    });
    let java_requirement = resolved.java.as_ref().ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Minecraft '{}' does not publish an authoritative Java runtime requirement",
            resolved.minecraft_version
        ))
    })?;
    let java = plan_mojang_java(client, java_requirement, JavaTarget::native()?, |event| {
        progress(InstallProgressEvent::Java(event));
    })
    .await?;
    Ok(ClientInstallPlan {
        instance_id: manifest.id,
        minecraft_requirement: manifest.minecraft.requirement,
        loader_requirement: manifest.loader.requirement,
        resolved,
        java,
        loader: PlannedLoader::Vanilla,
        authlib_injector,
    })
}

async fn prepare_vanilla_server_install<F>(
    instance_root: &Path,
    client: &reqwest::Client,
    mut progress: F,
) -> Result<ServerInstallPlan, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    let manifest = ManifestFile::open(instance_root)?.inner;
    require_vanilla_server(&manifest)?;
    progress(InstallProgressEvent::MetadataStarted);
    let resolved = MojangClient::new(client.clone())
        .resolve_vanilla_server(&manifest.minecraft.requirement)
        .await?;
    let authlib_injector = plan_authlib_injector(&manifest, client).await?;
    progress(InstallProgressEvent::MinecraftResolved {
        version: resolved.minecraft_version.clone(),
        total_artifacts: 1 + usize::from(authlib_injector.is_some()),
    });
    let java_requirement = resolved.java.as_ref().ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Minecraft '{}' does not publish an authoritative Java runtime requirement",
            resolved.minecraft_version
        ))
    })?;
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
    Ok(ServerInstallPlan {
        instance_id: manifest.id,
        minecraft_requirement: manifest.minecraft.requirement,
        loader_requirement: manifest.loader.requirement,
        resolved,
        java,
        eula,
        acceptance,
        loader: PlannedLoader::Vanilla,
        authlib_injector,
    })
}

async fn prepare_profile_loader_install<F>(
    instance_root: &Path,
    client: &reqwest::Client,
    mut progress: F,
) -> Result<InstallPlan, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    let manifest = ManifestFile::open(instance_root)?.inner;
    if !matches!(manifest.loader.kind, LoaderKind::Fabric | LoaderKind::Quilt) {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Loader '{}' does not use the profile install pipeline",
            manifest.loader.kind.as_str()
        )));
    }
    let loader_requirement = manifest.loader.requirement.as_deref().ok_or_else(|| {
        LauncherError::InvalidManifest("profile Loader requires a version requirement".to_string())
    })?;
    progress(InstallProgressEvent::MetadataStarted);
    let mojang = MojangClient::new(client.clone());
    match manifest.kind {
        InstanceKind::Client => {
            let resolved = resolve_vanilla_client(
                &mojang,
                &manifest.minecraft.requirement,
                &HostPlatform::native()?,
            )
            .await?;
            let profile = resolve_loader_profile(
                client,
                manifest.loader.kind,
                &resolved.minecraft_version,
                loader_requirement,
                LoaderSide::Client,
            )
            .await?;
            let authlib_injector = plan_authlib_injector(&manifest, client).await?;
            progress(InstallProgressEvent::MinecraftResolved {
                version: resolved.minecraft_version.clone(),
                total_artifacts: resolved.downloads.len()
                    + profile.downloads.len()
                    + 1
                    + usize::from(authlib_injector.is_some()),
            });
            ensure_loader_java_compatible(
                resolved.java.as_ref(),
                profile.minimum_java_major,
                profile.kind,
                &profile.version,
            )?;
            let java_requirement = resolved.java.as_ref().ok_or_else(|| {
                LauncherError::UnsupportedRequirement(format!(
                    "Minecraft '{}' does not publish an authoritative Java runtime requirement",
                    resolved.minecraft_version
                ))
            })?;
            let java = plan_selected_java(client, java_requirement, &mut progress).await?;
            Ok(InstallPlan::Client(ClientInstallPlan {
                instance_id: manifest.id,
                minecraft_requirement: manifest.minecraft.requirement,
                loader_requirement: manifest.loader.requirement,
                resolved,
                java,
                loader: PlannedLoader::Profile(profile),
                authlib_injector,
            }))
        }
        InstanceKind::Server => {
            let resolved = mojang
                .resolve_vanilla_server(&manifest.minecraft.requirement)
                .await?;
            let profile = resolve_loader_profile(
                client,
                manifest.loader.kind,
                &resolved.minecraft_version,
                loader_requirement,
                LoaderSide::Server,
            )
            .await?;
            let authlib_injector = plan_authlib_injector(&manifest, client).await?;
            progress(InstallProgressEvent::MinecraftResolved {
                version: resolved.minecraft_version.clone(),
                total_artifacts: profile.downloads.len()
                    + 1
                    + usize::from(authlib_injector.is_some()),
            });
            ensure_loader_java_compatible(
                resolved.java.as_ref(),
                profile.minimum_java_major,
                profile.kind,
                &profile.version,
            )?;
            let java_requirement = resolved.java.as_ref().ok_or_else(|| {
                LauncherError::UnsupportedRequirement(format!(
                    "Minecraft '{}' does not publish an authoritative Java runtime requirement",
                    resolved.minecraft_version
                ))
            })?;
            let java = plan_selected_java(client, java_requirement, &mut progress).await?;
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
            Ok(InstallPlan::Server(ServerInstallPlan {
                instance_id: manifest.id,
                minecraft_requirement: manifest.minecraft.requirement,
                loader_requirement: manifest.loader.requirement,
                resolved,
                java,
                eula,
                acceptance,
                loader: PlannedLoader::Profile(profile),
                authlib_injector,
            }))
        }
    }
}

async fn prepare_installer_loader_install<F>(
    instance_root: &Path,
    client: &reqwest::Client,
    mut progress: F,
) -> Result<InstallPlan, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    let manifest = ManifestFile::open(instance_root)?.inner;
    if !matches!(
        manifest.loader.kind,
        LoaderKind::Forge | LoaderKind::Neoforge
    ) {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Loader '{}' does not use the installer pipeline",
            manifest.loader.kind.as_str()
        )));
    }
    let loader_requirement = manifest.loader.requirement.as_deref().ok_or_else(|| {
        LauncherError::InvalidManifest(
            "installer Loader requires a version requirement".to_string(),
        )
    })?;
    progress(InstallProgressEvent::MetadataStarted);
    let mojang = MojangClient::new(client.clone());
    match manifest.kind {
        InstanceKind::Client => {
            let resolved = resolve_vanilla_client(
                &mojang,
                &manifest.minecraft.requirement,
                &HostPlatform::native()?,
            )
            .await?;
            let installer = resolve_loader_installer(
                client,
                manifest.loader.kind,
                &resolved.minecraft_version,
                loader_requirement,
            )
            .await?;
            let authlib_injector = plan_authlib_injector(&manifest, client).await?;
            progress(InstallProgressEvent::MinecraftResolved {
                version: resolved.minecraft_version.clone(),
                total_artifacts: resolved.downloads.len()
                    + 2
                    + usize::from(authlib_injector.is_some()),
            });
            let java_requirement = resolved.java.as_ref().ok_or_else(|| {
                LauncherError::UnsupportedRequirement(format!(
                    "Minecraft '{}' does not publish an authoritative Java runtime requirement",
                    resolved.minecraft_version
                ))
            })?;
            let java = plan_selected_java(client, java_requirement, &mut progress).await?;
            Ok(InstallPlan::Client(ClientInstallPlan {
                instance_id: manifest.id,
                minecraft_requirement: manifest.minecraft.requirement,
                loader_requirement: manifest.loader.requirement,
                resolved,
                java,
                loader: PlannedLoader::Installer(installer),
                authlib_injector,
            }))
        }
        InstanceKind::Server => {
            let resolved = mojang
                .resolve_vanilla_server(&manifest.minecraft.requirement)
                .await?;
            let installer = resolve_loader_installer(
                client,
                manifest.loader.kind,
                &resolved.minecraft_version,
                loader_requirement,
            )
            .await?;
            let authlib_injector = plan_authlib_injector(&manifest, client).await?;
            progress(InstallProgressEvent::MinecraftResolved {
                version: resolved.minecraft_version.clone(),
                total_artifacts: 2 + usize::from(authlib_injector.is_some()),
            });
            let java_requirement = resolved.java.as_ref().ok_or_else(|| {
                LauncherError::UnsupportedRequirement(format!(
                    "Minecraft '{}' does not publish an authoritative Java runtime requirement",
                    resolved.minecraft_version
                ))
            })?;
            let java = plan_selected_java(client, java_requirement, &mut progress).await?;
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
            Ok(InstallPlan::Server(ServerInstallPlan {
                instance_id: manifest.id,
                minecraft_requirement: manifest.minecraft.requirement,
                loader_requirement: manifest.loader.requirement,
                resolved,
                java,
                eula,
                acceptance,
                loader: PlannedLoader::Installer(installer),
                authlib_injector,
            }))
        }
    }
}

pub async fn prepare_install<F>(
    location: &InstanceLocation,
    client: &reqwest::Client,
    progress: F,
) -> Result<InstallPlan, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    location
        .validate()
        .map_err(LauncherError::InvalidRegistry)?;
    let instance_root = location.instance_directory();
    let manifest = ManifestFile::open(instance_root)?.inner;
    if manifest.kind != location.kind() {
        return Err(LauncherError::InstanceRegistryMismatch(
            "registered instance layout kind disagrees with orbit-launcher.toml".to_string(),
        ));
    }
    let mut plan = match manifest.loader.kind {
        LoaderKind::Vanilla => match manifest.kind {
            InstanceKind::Client => prepare_vanilla_client_install(instance_root, client, progress)
                .await
                .map(InstallPlan::Client),
            InstanceKind::Server => prepare_vanilla_server_install(instance_root, client, progress)
                .await
                .map(InstallPlan::Server),
        },
        LoaderKind::Fabric | LoaderKind::Quilt => {
            prepare_profile_loader_install(instance_root, client, progress).await
        }
        LoaderKind::Forge | LoaderKind::Neoforge => {
            prepare_installer_loader_install(instance_root, client, progress).await
        }
    }?;
    if let InstallPlan::Client(client) = &mut plan {
        client
            .resolved
            .relocate_minecraft_jar(location.instance_relative_path(INSTANCE_MINECRAFT_JAR)?)?;
    }
    Ok(plan)
}

async fn plan_selected_java<F>(
    client: &reqwest::Client,
    requirement: &crate::mojang::MojangJavaRequirement,
    progress: &mut F,
) -> Result<MojangJavaPlan, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    plan_mojang_java(client, requirement, JavaTarget::native()?, |event| {
        progress(InstallProgressEvent::Java(event));
    })
    .await
}

fn ensure_loader_java_compatible(
    minecraft: Option<&crate::mojang::MojangJavaRequirement>,
    loader_minimum: Option<u32>,
    kind: LoaderKind,
    version: &str,
) -> Result<(), LauncherError> {
    if let (Some(minecraft), Some(loader_minimum)) = (minecraft, loader_minimum)
        && minecraft.major < loader_minimum
    {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "{} {version} requires Java {loader_minimum} or newer, but Minecraft metadata selects Java {}",
            kind.as_str(),
            minecraft.major
        )));
    }
    Ok(())
}

async fn execute_client_install<F>(
    location: &InstanceLocation,
    runtime_paths: &RuntimePaths,
    client: &reqwest::Client,
    plan: ClientInstallPlan,
    concurrency: usize,
    installer_timeout_seconds: u64,
    mut progress: F,
) -> Result<InstallResult, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    if concurrency == 0 {
        return Err(LauncherError::InvalidConfig(
            "artifact download concurrency must be greater than zero".to_string(),
        ));
    }
    let instance_root = location.instance_directory();
    let manifest = ManifestFile::open(instance_root)?.inner;
    require_supported_client(&manifest)?;
    if manifest.id != plan.instance_id
        || manifest.minecraft.requirement != plan.minecraft_requirement
        || manifest.loader.requirement != plan.loader_requirement
        || manifest_requires_authlib_injector(&manifest) != plan.authlib_injector.is_some()
    {
        return Err(LauncherError::Transaction(
            "instance manifest changed after the install plan was generated".to_string(),
        ));
    }
    let authlib_injector = plan.authlib_injector.clone();
    let transaction = InstallTransaction::begin(location, "install")?;
    let java = install_mojang_java(runtime_paths, client, plan.java, concurrency, |event| {
        progress(InstallProgressEvent::Java(event));
    })
    .await?;
    let cache = ArtifactCache::new(runtime_paths.cache_dir());
    let mut downloaded = java.downloaded_artifacts;
    let mut cached = java.cached_artifacts;
    let mut installer_producer = None;
    let (mut resolved, locked_loader) = match plan.loader {
        PlannedLoader::Vanilla => (plan.resolved, LockedLoader::vanilla()),
        PlannedLoader::Profile(profile) => merge_client_profile(plan.resolved, profile)?,
        PlannedLoader::Installer(installer) => {
            let installer_artifact = cache
                .fetch(client, &installer.artifact, |event| {
                    progress(InstallProgressEvent::Artifact(event));
                })
                .await?;
            downloaded += usize::from(!installer_artifact.cache_hit);
            cached += usize::from(installer_artifact.cache_hit);
            let inspected = inspect_loader_installer(&installer_artifact.object_path, &installer)?;
            let staged_installer = transaction.staging.join(INSTALLER_STAGING_NAME);
            cache.materialize(&installer_artifact, &staged_installer)?;
            run_loader_installer(
                &java.root.join(&java.locked.executable),
                &staged_installer,
                &installer,
                InstallerSide::Client,
                &transaction.staging,
                std::time::Duration::from_secs(installer_timeout_seconds),
                |event| progress(InstallProgressEvent::LoaderInstaller(event)),
            )
            .await?;
            let installed_profile = read_installed_client_profile(
                &transaction.staging,
                &installer,
                &HostPlatform::native()?,
            )?;
            remove_installed_client_profile(&transaction.staging, &installed_profile.profile_path)?;
            cleanup_installer_scaffolding(&transaction.staging)?;
            let locked = LockedLoader::installer(
                installer.kind,
                installer.version.clone(),
                installer.artifact.url.clone(),
                installer_artifact.sha256.clone(),
                inspected.install_profile_sha256,
            );
            installer_producer = Some((
                installer_artifact.sha256,
                format!("{} {}", installer.kind.as_str(), installer.version),
            ));
            (
                merge_client_installer_profile(plan.resolved, installed_profile)?,
                locked,
            )
        }
    };
    append_authlib_download(&mut resolved.downloads, authlib_injector.as_ref());
    cache.flush()?;
    let progress = Arc::new(Mutex::new(progress));
    let mut transfers = stream::iter(resolved.downloads.iter().cloned().map(|download| {
        let client = client.clone();
        let cache = cache.clone();
        let progress = Arc::clone(&progress);
        async move {
            let artifact = cache
                .fetch(&client, &download.request, |event| {
                    if let Ok(mut progress) = progress.lock() {
                        progress(InstallProgressEvent::Artifact(event));
                    }
                })
                .await?;
            Ok::<_, LauncherError>((download, artifact))
        }
    }))
    .buffer_unordered(concurrency);
    let mut artifacts = BTreeMap::new();
    while let Some(result) = transfers.next().await {
        let (download, artifact) = result?;
        downloaded += usize::from(!artifact.cache_hit);
        cached += usize::from(artifact.cache_hit);
        if artifacts
            .insert(download.target.clone(), (download, artifact))
            .is_some()
        {
            return Err(LauncherError::InvalidRemoteData(
                "Minecraft metadata resolves multiple artifacts to the same instance path"
                    .to_string(),
            ));
        }
    }
    cache.flush()?;
    let locked_authlib_injector = lock_authlib_injector(authlib_injector.as_ref(), &artifacts)?;

    let mut locked_artifacts = Vec::new();
    for (target, (download, artifact)) in &artifacts {
        cache.materialize(artifact, &transaction.staging.join(target))?;
        locked_artifacts.push(locked_artifact(download, artifact));
    }

    if let Some((installer_sha256, logical_name)) = installer_producer {
        let excluded: HashSet<_> = locked_artifacts
            .iter()
            .map(|artifact| artifact.path.clone())
            .collect();
        locked_artifacts.extend(collect_installer_outputs(
            &transaction.staging,
            &excluded,
            &installer_sha256,
            &logical_name,
        )?);
    }

    let mut generated_files = materialize_legacy_assets(&transaction.staging, location, &resolved)?;
    generated_files.sort();
    generated_files.dedup();
    locked_artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    let lock = LauncherLock {
        schema: LOCK_SCHEMA,
        instance_id: manifest.id,
        kind: InstanceKind::Client,
        minecraft: LockedMinecraft {
            version: resolved.minecraft_version.clone(),
            version_type: resolved.version_type,
            asset_index: Some(resolved.asset_index_id),
            version_manifest_url: VERSION_MANIFEST_V2_URL.to_string(),
            version_manifest_sha256: resolved.version_manifest_sha256,
            version_json_url: resolved.version_json_url,
            version_json_sha1: resolved.version_json_sha1,
        },
        loader: locked_loader,
        java: Some(java.locked),
        authlib_injector: locked_authlib_injector,
        entrypoint: LockedEntrypoint::Classpath {
            main_class: resolved.main_class,
            classpath: resolved.classpath,
        },
        arguments: LockedArguments {
            jvm: resolved.jvm_arguments,
            game: resolved.game_arguments,
        },
        artifacts: locked_artifacts,
        generated_files,
        eula: None,
    };
    lock.validate()?;
    LockFile::new(&transaction.staged_instance_directory(), lock.clone()).save()?;
    verify_staged_client(
        &transaction.staging,
        &transaction.staged_instance_directory(),
        &lock,
    )?;
    emit_install(&progress, InstallProgressEvent::StagingVerified)?;
    let mut owned_files: Vec<_> = lock
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .chain(lock.generated_files.iter().cloned())
        .collect();
    owned_files.push(transaction.lock_relative.clone());
    owned_files.sort();
    transaction.commit(&owned_files)?;
    emit_install(&progress, InstallProgressEvent::Committed)?;
    Ok(InstallResult {
        lock,
        downloaded_artifacts: downloaded,
        cached_artifacts: cached,
    })
}

async fn execute_server_install<F>(
    location: &InstanceLocation,
    runtime_paths: &RuntimePaths,
    client: &reqwest::Client,
    plan: ServerInstallPlan,
    concurrency: usize,
    installer_timeout_seconds: u64,
    mut progress: F,
) -> Result<InstallResult, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    if concurrency == 0 {
        return Err(LauncherError::InvalidConfig(
            "artifact download concurrency must be greater than zero".to_string(),
        ));
    }
    let instance_root = location.instance_directory();
    let manifest = ManifestFile::open(instance_root)?.inner;
    require_supported_server(&manifest)?;
    if manifest.id != plan.instance_id
        || manifest.minecraft.requirement != plan.minecraft_requirement
        || manifest.loader.requirement != plan.loader_requirement
        || manifest_requires_authlib_injector(&manifest) != plan.authlib_injector.is_some()
    {
        return Err(LauncherError::Transaction(
            "instance manifest changed after the install plan was generated".to_string(),
        ));
    }
    let authlib_injector = plan.authlib_injector.clone();
    let acceptance = require_current_acceptance(instance_root, &plan.eula)?;
    let transaction = InstallTransaction::begin(location, "install")?;
    let java = install_mojang_java(runtime_paths, client, plan.java, concurrency, |event| {
        progress(InstallProgressEvent::Java(event))
    })
    .await?;
    let cache = ArtifactCache::new(runtime_paths.cache_dir());
    let mut downloaded = java.downloaded_artifacts;
    let mut cached = java.cached_artifacts;
    let mut installer_producer = None;
    let installer_loader = matches!(&plan.loader, PlannedLoader::Installer(_));
    let (loader, entrypoint, arguments, mut downloads) = match plan.loader {
        PlannedLoader::Vanilla => server_profile_parts(&plan.resolved, None)?,
        PlannedLoader::Profile(profile) => server_profile_parts(&plan.resolved, Some(profile))?,
        PlannedLoader::Installer(installer) => {
            let installer_artifact = cache
                .fetch(client, &installer.artifact, |event| {
                    progress(InstallProgressEvent::Artifact(event));
                })
                .await?;
            downloaded += usize::from(!installer_artifact.cache_hit);
            cached += usize::from(installer_artifact.cache_hit);
            let inspected = inspect_loader_installer(&installer_artifact.object_path, &installer)?;
            let staged_installer = transaction.staging.join(INSTALLER_STAGING_NAME);
            cache.materialize(&installer_artifact, &staged_installer)?;
            run_loader_installer(
                &java.root.join(&java.locked.executable),
                &staged_installer,
                &installer,
                InstallerSide::Server,
                &transaction.staging,
                std::time::Duration::from_secs(installer_timeout_seconds),
                |event| progress(InstallProgressEvent::LoaderInstaller(event)),
            )
            .await?;
            let argument_file = installed_server_argument_file(
                &transaction.staging,
                &installer,
                &HostPlatform::native()?,
            )?;
            cleanup_installer_scaffolding(&transaction.staging)?;
            let locked = LockedLoader::installer(
                installer.kind,
                installer.version.clone(),
                installer.artifact.url.clone(),
                installer_artifact.sha256.clone(),
                inspected.install_profile_sha256,
            );
            installer_producer = Some((
                installer_artifact.sha256,
                format!("{} {}", installer.kind.as_str(), installer.version),
            ));
            (
                locked,
                LockedEntrypoint::ArgumentFile {
                    path: argument_file,
                },
                LockedArguments {
                    jvm: Vec::new(),
                    game: vec!["nogui".to_string()],
                },
                Vec::new(),
            )
        }
    };
    if !installer_loader {
        downloads.push(ClientDownload {
            request: plan.resolved.server.clone(),
            target: "server.jar".to_string(),
            owner: ArtifactOwner::Minecraft,
            native_extract: None,
        });
    }
    append_authlib_download(&mut downloads, authlib_injector.as_ref());
    cache.flush()?;
    let progress = Arc::new(Mutex::new(progress));
    let mut transfers = stream::iter(downloads.into_iter().map(|download| {
        let client = client.clone();
        let cache = cache.clone();
        let progress = Arc::clone(&progress);
        async move {
            let artifact = cache
                .fetch(&client, &download.request, |event| {
                    if let Ok(mut progress) = progress.lock() {
                        progress(InstallProgressEvent::Artifact(event));
                    }
                })
                .await?;
            Ok::<_, LauncherError>((download, artifact))
        }
    }))
    .buffer_unordered(concurrency);
    let mut artifacts = BTreeMap::new();
    while let Some(result) = transfers.next().await {
        let (download, artifact) = result?;
        downloaded += usize::from(!artifact.cache_hit);
        cached += usize::from(artifact.cache_hit);
        if artifacts
            .insert(download.target.clone(), (download, artifact))
            .is_some()
        {
            return Err(LauncherError::InvalidRemoteData(
                "server metadata resolves multiple artifacts to the same instance path".to_string(),
            ));
        }
    }
    cache.flush()?;
    let locked_authlib_injector = lock_authlib_injector(authlib_injector.as_ref(), &artifacts)?;
    let mut locked_artifacts = Vec::new();
    for (target, (download, artifact)) in &artifacts {
        cache.materialize(artifact, &transaction.staging.join(target))?;
        locked_artifacts.push(locked_artifact(download, artifact));
    }
    if let Some((installer_sha256, logical_name)) = installer_producer {
        let excluded: HashSet<_> = locked_artifacts
            .iter()
            .map(|artifact| artifact.path.clone())
            .collect();
        locked_artifacts.extend(collect_installer_outputs(
            &transaction.staging,
            &excluded,
            &installer_sha256,
            &logical_name,
        )?);
    }
    locked_artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    std::fs::write(transaction.staging.join("eula.txt"), b"eula=true\n")?;

    let lock = LauncherLock {
        schema: LOCK_SCHEMA,
        instance_id: manifest.id,
        kind: InstanceKind::Server,
        minecraft: LockedMinecraft {
            version: plan.resolved.minecraft_version,
            version_type: plan.resolved.version_type,
            asset_index: None,
            version_manifest_url: VERSION_MANIFEST_V2_URL.to_string(),
            version_manifest_sha256: plan.resolved.version_manifest_sha256,
            version_json_url: plan.resolved.version_json_url,
            version_json_sha1: plan.resolved.version_json_sha1,
        },
        loader,
        java: Some(java.locked),
        authlib_injector: locked_authlib_injector,
        entrypoint,
        arguments,
        artifacts: locked_artifacts,
        generated_files: vec!["eula.txt".to_string()],
        eula: Some(acceptance),
    };
    lock.validate()?;
    LockFile::new(&transaction.staged_instance_directory(), lock.clone()).save()?;
    verify_staged_server(
        &transaction.staging,
        &transaction.staged_instance_directory(),
        &lock,
    )?;
    emit_install(&progress, InstallProgressEvent::StagingVerified)?;
    let mut owned_files: Vec<_> = lock
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .chain(lock.generated_files.iter().cloned())
        .collect();
    owned_files.push(transaction.lock_relative.clone());
    owned_files.sort();
    transaction.commit(&owned_files)?;
    emit_install(&progress, InstallProgressEvent::Committed)?;
    Ok(InstallResult {
        lock,
        downloaded_artifacts: downloaded,
        cached_artifacts: cached,
    })
}

pub async fn apply_install_plan<F>(
    location: &InstanceLocation,
    runtime_paths: &RuntimePaths,
    client: &reqwest::Client,
    plan: InstallPlan,
    concurrency: usize,
    installer_timeout_seconds: u64,
    progress: F,
) -> Result<InstallResult, LauncherError>
where
    F: FnMut(InstallProgressEvent) + Send,
{
    match plan {
        InstallPlan::Client(plan) => {
            execute_client_install(
                location,
                runtime_paths,
                client,
                plan,
                concurrency,
                installer_timeout_seconds,
                progress,
            )
            .await
        }
        InstallPlan::Server(plan) => {
            execute_server_install(
                location,
                runtime_paths,
                client,
                plan,
                concurrency,
                installer_timeout_seconds,
                progress,
            )
            .await
        }
    }
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

fn require_supported_server(manifest: &InstanceManifest) -> Result<(), LauncherError> {
    if manifest.kind != InstanceKind::Server {
        return Err(LauncherError::UnsupportedRequirement(
            "server installation requires a server instance".to_string(),
        ));
    }
    if !matches!(
        manifest.loader.kind,
        LoaderKind::Vanilla
            | LoaderKind::Fabric
            | LoaderKind::Quilt
            | LoaderKind::Forge
            | LoaderKind::Neoforge
    ) {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Loader '{}' must use its own installer adapter",
            manifest.loader.kind.as_str()
        )));
    }
    Ok(())
}

fn server_profile_parts(
    resolved: &ResolvedVanillaServer,
    profile: Option<ResolvedLoaderProfile>,
) -> Result<
    (
        LockedLoader,
        LockedEntrypoint,
        LockedArguments,
        Vec<ClientDownload>,
    ),
    LauncherError,
> {
    let Some(profile) = profile else {
        return Ok((
            LockedLoader::vanilla(),
            LockedEntrypoint::Jar {
                path: "server.jar".to_string(),
            },
            LockedArguments {
                jvm: Vec::new(),
                game: vec!["nogui".to_string()],
            },
            Vec::new(),
        ));
    };
    let mut classpath = profile.classpath.clone();
    classpath.push("server.jar".to_string());
    let mut game_arguments = profile.game_arguments.clone();
    if !game_arguments.iter().any(|argument| argument == "nogui") {
        game_arguments.push("nogui".to_string());
    }
    if resolved.server.url.is_empty() {
        return Err(LauncherError::InvalidRemoteData(
            "resolved Minecraft server artifact is empty".to_string(),
        ));
    }
    Ok((
        LockedLoader::profile(
            profile.kind,
            profile.version,
            profile.profile_url,
            profile.profile_sha256,
        ),
        LockedEntrypoint::Classpath {
            main_class: profile.main_class,
            classpath,
        },
        LockedArguments {
            jvm: profile.jvm_arguments,
            game: game_arguments,
        },
        profile.downloads,
    ))
}

fn require_vanilla_client(manifest: &InstanceManifest) -> Result<(), LauncherError> {
    if manifest.kind != InstanceKind::Client {
        return Err(LauncherError::UnsupportedRequirement(
            "Vanilla client installation requires a client instance".to_string(),
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

fn require_supported_client(manifest: &InstanceManifest) -> Result<(), LauncherError> {
    if manifest.kind != InstanceKind::Client {
        return Err(LauncherError::UnsupportedRequirement(
            "client installation requires a client instance".to_string(),
        ));
    }
    if !matches!(
        manifest.loader.kind,
        LoaderKind::Vanilla
            | LoaderKind::Fabric
            | LoaderKind::Quilt
            | LoaderKind::Forge
            | LoaderKind::Neoforge
    ) {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Loader '{}' must use its own installer adapter",
            manifest.loader.kind.as_str()
        )));
    }
    Ok(())
}

fn merge_client_profile(
    mut resolved: ResolvedVanillaClient,
    profile: ResolvedLoaderProfile,
) -> Result<(ResolvedVanillaClient, LockedLoader), LauncherError> {
    let mut occupied: HashSet<_> = resolved
        .downloads
        .iter()
        .map(|download| download.target.clone())
        .collect();
    for download in profile.downloads {
        if !occupied.insert(download.target.clone()) {
            return Err(LauncherError::InvalidRemoteData(format!(
                "{} profile library conflicts with inherited Minecraft path '{}'",
                profile.kind.as_str(),
                download.target
            )));
        }
        resolved.downloads.push(download);
    }
    let mut classpath = profile.classpath;
    classpath.extend(resolved.classpath);
    resolved.classpath = classpath;
    resolved.main_class = profile.main_class;
    resolved.jvm_arguments.extend(profile.jvm_arguments);
    resolved.game_arguments.extend(profile.game_arguments);
    Ok((
        resolved,
        LockedLoader::profile(
            profile.kind,
            profile.version,
            profile.profile_url,
            profile.profile_sha256,
        ),
    ))
}

fn merge_client_installer_profile(
    mut resolved: ResolvedVanillaClient,
    profile: crate::installer::InstalledClientProfile,
) -> Result<ResolvedVanillaClient, LauncherError> {
    let mut occupied: HashSet<_> = resolved.classpath.iter().cloned().collect();
    let mut classpath = Vec::new();
    for entry in profile.classpath {
        if occupied.insert(entry.clone()) {
            classpath.push(entry);
        }
    }
    classpath.extend(resolved.classpath);
    resolved.classpath = classpath;
    resolved.main_class = profile.main_class;
    resolved.jvm_arguments.extend(profile.jvm_arguments);
    resolved.game_arguments.extend(profile.game_arguments);
    Ok(resolved)
}

fn locked_artifact(download: &ClientDownload, artifact: &CachedArtifact) -> LockedArtifact {
    let upstream_sha1 = match &download.request.expected_hash {
        ExpectedHash::Sha1(value) => Some(value.clone()),
        _ => None,
    };
    LockedArtifact {
        logical_name: download.request.logical_name.clone(),
        owner: download.owner,
        source: LockedArtifactSource::Download {
            url: download.request.url.clone(),
            upstream_sha1,
        },
        sha256: artifact.sha256.clone(),
        size: artifact.size,
        path: download.target.clone(),
        native_extraction: download.native_extract.as_ref().map(|native| {
            crate::lockfile::LockedNativeExtraction {
                excludes: native.excludes.clone(),
            }
        }),
    }
}

fn remove_installed_client_profile(staging: &Path, relative: &str) -> Result<(), LauncherError> {
    let profile = staging.join(relative);
    if !profile.is_file() {
        return Err(LauncherError::Transaction(format!(
            "official Loader installer profile '{relative}' disappeared before cleanup"
        )));
    }
    std::fs::remove_file(&profile)?;
    if let Some(directory) = profile.parent()
        && directory != staging
        && std::fs::read_dir(directory)?.next().is_none()
    {
        std::fs::remove_dir(directory)?;
    }
    let versions = staging.join("versions");
    if versions.is_dir() && std::fs::read_dir(&versions)?.next().is_none() {
        std::fs::remove_dir(versions)?;
    }
    Ok(())
}

#[cfg(test)]
fn write_staged_file(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), LauncherError> {
    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, bytes)?;
    Ok(())
}

fn cleanup_installer_scaffolding(staging: &Path) -> Result<(), LauncherError> {
    for relative in [
        INSTALLER_STAGING_NAME,
        ".orbit-loader-installer.jar.log",
        "launcher_profiles.json",
        "run.bat",
        "run.sh",
        "user_jvm_args.txt",
    ] {
        let path = staging.join(relative);
        if path.exists() {
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(LauncherError::Transaction(format!(
                    "official Loader installer created unsafe scaffolding path '{relative}'"
                )));
            }
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn collect_installer_outputs(
    staging: &Path,
    excluded: &HashSet<String>,
    installer_sha256: &str,
    logical_name: &str,
) -> Result<Vec<LockedArtifact>, LauncherError> {
    const MAX_OUTPUT_FILES: usize = 100_000;
    const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024 * 1024;

    let mut directories = vec![staging.to_path_buf()];
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "official Loader installer produced a symbolic link at '{}'",
                    entry.path().display()
                )));
            }
            if metadata.is_dir() {
                directories.push(entry.path());
                continue;
            }
            if !metadata.is_file() {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "official Loader installer produced an unsupported filesystem object at '{}'",
                    entry.path().display()
                )));
            }
            let path = entry.path();
            let relative = path.strip_prefix(staging).map_err(|_| {
                LauncherError::Transaction("Loader installer output escaped staging".to_string())
            })?;
            let relative = crate::lockfile::portable_relative_path(relative)?;
            if excluded.contains(&relative) {
                continue;
            }
            total_bytes = total_bytes.checked_add(metadata.len()).ok_or_else(|| {
                LauncherError::InvalidRemoteData(
                    "Loader installer output size overflowed".to_string(),
                )
            })?;
            if files.len() >= MAX_OUTPUT_FILES || total_bytes > MAX_OUTPUT_BYTES {
                return Err(LauncherError::InvalidRemoteData(
                    "Loader installer output exceeds the supported inventory limits".to_string(),
                ));
            }
            files.push((relative, path, metadata.len()));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
        .into_iter()
        .map(|(relative, path, size)| {
            Ok(LockedArtifact {
                logical_name: format!("{logical_name} installer output"),
                owner: ArtifactOwner::Loader,
                source: LockedArtifactSource::InstallerOutput {
                    installer_sha256: installer_sha256.to_string(),
                },
                sha256: hash_file_sha256(&path)?,
                size,
                path: relative,
                native_extraction: None,
            })
        })
        .collect()
}

fn materialize_legacy_assets(
    staging: &Path,
    location: &InstanceLocation,
    resolved: &ResolvedVanillaClient,
) -> Result<Vec<String>, LauncherError> {
    if !resolved.legacy_virtual_assets && !resolved.map_assets_to_resources {
        return Ok(Vec::new());
    }
    let mut generated = Vec::new();
    for mapping in &resolved.asset_mappings {
        let source = staging.join(&mapping.object_path);
        let mut targets = Vec::with_capacity(2);
        if resolved.legacy_virtual_assets {
            targets.push(format!(
                "assets/virtual/{}/{}",
                resolved.asset_index_id, mapping.logical_path
            ));
        }
        if resolved.map_assets_to_resources {
            targets.push(
                location.instance_relative_path(&format!("resources/{}", mapping.logical_path))?,
            );
        }
        for relative in targets {
            let target = staging.join(&relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&source, target)?;
            generated.push(relative);
        }
    }
    Ok(generated)
}

fn verify_staged_client(
    staging: &Path,
    staged_instance: &Path,
    lock: &LauncherLock,
) -> Result<(), LauncherError> {
    for artifact in &lock.artifacts {
        let path = staging.join(&artifact.path);
        if std::fs::metadata(&path)?.len() != artifact.size
            || hash_file_sha256(&path)? != artifact.sha256
        {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "staged artifact '{}' failed final verification",
                artifact.logical_name
            )));
        }
    }
    for generated in &lock.generated_files {
        if !staging.join(generated).is_file() {
            return Err(LauncherError::Transaction(format!(
                "staged generated file '{generated}' is missing"
            )));
        }
    }
    LockFile::open(staged_instance)?;
    Ok(())
}

fn emit_install<F>(
    progress: &Arc<Mutex<F>>,
    event: InstallProgressEvent,
) -> Result<(), LauncherError>
where
    F: FnMut(InstallProgressEvent),
{
    let mut progress = progress.lock().map_err(|_| {
        LauncherError::Transaction("install progress callback lock was poisoned".to_string())
    })?;
    progress(event);
    Ok(())
}

fn verify_staged_server(
    staging: &Path,
    staged_instance: &Path,
    lock: &LauncherLock,
) -> Result<(), LauncherError> {
    for artifact in &lock.artifacts {
        let path = staging.join(&artifact.path);
        if std::fs::metadata(&path)?.len() != artifact.size
            || hash_file_sha256(&path)? != artifact.sha256
        {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "staged server artifact '{}' failed final verification",
                artifact.logical_name
            )));
        }
    }
    let eula = std::fs::read_to_string(staging.join("eula.txt"))?;
    if eula != "eula=true\n" {
        return Err(LauncherError::Transaction(
            "staged eula.txt is invalid".to_string(),
        ));
    }
    LockFile::open(staged_instance)?;
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
    write_files: Vec<String>,
    remove_files: Vec<String>,
    reused_files: Vec<String>,
}

struct InstallTransaction {
    root: PathBuf,
    instance_root: PathBuf,
    state: PathBuf,
    staging: PathBuf,
    lock_relative: String,
    owned_prefix: Option<String>,
    id: Uuid,
    _lock: File,
}

impl InstallTransaction {
    fn begin(location: &InstanceLocation, command: &str) -> Result<Self, LauncherError> {
        let root = location.artifact_directory();
        let instance_root = location.instance_directory();
        let state = instance_root.join(STATE_DIRECTORY);
        std::fs::create_dir_all(&state)?;
        let lock_path = state.join(TRANSACTION_LOCK);
        let mut lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| {
                LauncherError::Transaction(format!(
                    "cannot open instance transaction lock '{}': {error}",
                    lock_path.display()
                ))
            })?;
        FileExt::try_lock_exclusive(&lock).map_err(|error| {
            LauncherError::Transaction(format!(
                "another install transaction holds '{}': {error}",
                lock_path.display()
            ))
        })?;
        recover_abandoned_transaction(root, &state)?;

        let id = Uuid::new_v4();
        let identity = TransactionIdentity {
            schema: 1,
            id,
            pid: std::process::id(),
            started_at_unix_seconds: unix_seconds()?,
            executable: std::env::current_exe()?,
            command: command.to_string(),
        };
        let identity_bytes = serde_json::to_vec_pretty(&identity).map_err(|error| {
            LauncherError::Transaction(format!("cannot serialize transaction identity: {error}"))
        })?;
        lock.set_len(0)?;
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
            instance_root: instance_root.to_path_buf(),
            state,
            staging,
            lock_relative: location.instance_relative_path(INSTANCE_LOCK_FILE)?,
            owned_prefix: match location {
                InstanceLocation::Client { .. } => {
                    let lock = location.instance_relative_path(INSTANCE_LOCK_FILE)?;
                    Some(
                        lock.strip_suffix(INSTANCE_LOCK_FILE)
                            .expect("lock path ends in lock filename")
                            .to_string(),
                    )
                }
                InstanceLocation::Server { .. } => None,
            },
            id,
            _lock: lock,
        })
    }

    fn staged_instance_directory(&self) -> PathBuf {
        self.staging.join(
            Path::new(&self.lock_relative)
                .parent()
                .expect("lock path always has an instance parent"),
        )
    }

    fn commit(self, relative_files: &[String]) -> Result<(), LauncherError> {
        let previous = load_previous_owned_paths(&self.instance_root)?;
        let next: HashSet<_> = relative_files.iter().cloned().collect();
        if next.len() != relative_files.len() {
            return Err(LauncherError::Transaction(
                "install transaction contains duplicate target paths".to_string(),
            ));
        }
        let mut stale: Vec<_> = previous
            .difference(&next)
            .filter(|relative| {
                self.owned_prefix
                    .as_ref()
                    .is_none_or(|prefix| relative.starts_with(prefix))
            })
            .cloned()
            .collect();
        stale.sort();
        let mut reused = HashSet::new();
        for relative in relative_files {
            let target = self.root.join(relative);
            if !self.staging.join(relative).is_file() {
                return Err(LauncherError::Transaction(format!(
                    "staging file '{relative}' is missing"
                )));
            }
            if target.exists() && relative != &self.lock_relative && !previous.contains(relative) {
                let staged = self.staging.join(relative);
                let same = std::fs::metadata(&target)?.len() == std::fs::metadata(&staged)?.len()
                    && hash_file_sha256(&target)? == hash_file_sha256(&staged)?;
                if !same {
                    return Err(LauncherError::Transaction(format!(
                        "refusing to overwrite conflicting shared Minecraft file '{relative}'"
                    )));
                }
                reused.insert(relative.clone());
            }
        }
        let mut reused_files: Vec<_> = reused.iter().cloned().collect();
        reused_files.sort();
        let journal = TransactionJournal {
            schema: 1,
            id: self.id,
            phase: "committing".to_string(),
            write_files: relative_files.to_vec(),
            remove_files: stale.clone(),
            reused_files,
        };
        write_json_atomic(&self.state.join(TRANSACTION_JOURNAL), &journal)?;

        let backup_root = self.staging.join("backup");
        let mut backup_paths: Vec<String> = relative_files
            .iter()
            .filter(|relative| !reused.contains(*relative) && self.root.join(relative).exists())
            .cloned()
            .collect();
        backup_paths.extend(
            stale
                .iter()
                .filter(|relative| self.root.join(relative).exists())
                .cloned(),
        );
        backup_paths.sort();
        backup_paths.dedup();
        let mut backed_up = Vec::new();
        for relative in &backup_paths {
            let target = self.root.join(relative);
            let backup = backup_root.join(relative);
            if let Some(parent) = backup.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if let Err(error) = std::fs::rename(&target, &backup) {
                restore_backups(&self.root, &backup_root, &backed_up)?;
                self.abort_after_rollback()?;
                return Err(error.into());
            }
            backed_up.push(relative.clone());
        }

        let mut committed = Vec::new();
        for relative in relative_files {
            if reused.contains(relative) {
                std::fs::remove_file(self.staging.join(relative))?;
                continue;
            }
            let target = self.root.join(relative);
            let source = self.staging.join(relative);
            if let Some(parent) = target.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                rollback_transaction(&self.root, &backup_root, &committed, &backed_up)?;
                self.abort_after_rollback()?;
                return Err(error.into());
            }
            if let Err(error) = std::fs::rename(&source, &target) {
                rollback_transaction(&self.root, &backup_root, &committed, &backed_up)?;
                self.abort_after_rollback()?;
                return Err(error.into());
            }
            committed.push(relative.clone());
        }

        std::fs::remove_file(self.state.join(TRANSACTION_JOURNAL))?;
        std::fs::remove_dir_all(&self.staging)?;
        Ok(())
    }

    fn abort_after_rollback(&self) -> Result<(), LauncherError> {
        let journal = self.state.join(TRANSACTION_JOURNAL);
        if journal.exists() {
            std::fs::remove_file(journal)?;
        }
        if self.staging.exists() {
            std::fs::remove_dir_all(&self.staging)?;
        }
        Ok(())
    }
}

impl Drop for InstallTransaction {
    fn drop(&mut self) {
        if !self.state.join(TRANSACTION_JOURNAL).exists() {
            let _ = std::fs::remove_dir_all(&self.staging);
        }
    }
}

fn recover_abandoned_transaction(root: &Path, state: &Path) -> Result<(), LauncherError> {
    let staging_root = state.join("staging");
    let journal_path = state.join(TRANSACTION_JOURNAL);
    if !journal_path.exists() {
        if staging_root.exists() {
            std::fs::remove_dir_all(staging_root)?;
        }
        return Ok(());
    }

    let journal: TransactionJournal = serde_json::from_slice(&std::fs::read(&journal_path)?)
        .map_err(|error| {
            LauncherError::Transaction(format!(
                "cannot parse abandoned transaction journal '{}': {error}",
                journal_path.display()
            ))
        })?;
    if journal.schema != 1 || journal.phase != "committing" {
        return Err(LauncherError::Transaction(format!(
            "abandoned transaction journal '{}' has unsupported schema {} or phase '{}'",
            journal_path.display(),
            journal.schema,
            journal.phase
        )));
    }
    validate_transaction_journal(&journal)?;

    let staging = staging_root.join(journal.id.to_string());
    if !staging.is_dir() {
        return Err(LauncherError::Transaction(format!(
            "abandoned transaction staging directory '{}' is missing",
            staging.display()
        )));
    }
    let reused: HashSet<_> = journal.reused_files.iter().map(String::as_str).collect();
    let backup_root = staging.join("backup");
    for relative in &journal.write_files {
        let target = transaction_path(root, relative)?;
        let source = transaction_path(&staging, relative)?;
        let backup = transaction_path(&backup_root, relative)?;
        if backup.is_file() {
            replace_with_backup(&target, &backup)?;
        } else if !source.exists() && !reused.contains(relative.as_str()) && target.exists() {
            std::fs::remove_file(&target)?;
        }
    }
    for relative in &journal.remove_files {
        let target = transaction_path(root, relative)?;
        let backup = transaction_path(&backup_root, relative)?;
        if backup.is_file() {
            replace_with_backup(&target, &backup)?;
        }
    }

    std::fs::remove_file(journal_path)?;
    std::fs::remove_dir_all(staging_root)?;
    Ok(())
}

fn validate_transaction_journal(journal: &TransactionJournal) -> Result<(), LauncherError> {
    let writes: HashSet<_> = journal.write_files.iter().collect();
    let removals: HashSet<_> = journal.remove_files.iter().collect();
    let reused: HashSet<_> = journal.reused_files.iter().collect();
    if writes.len() != journal.write_files.len()
        || removals.len() != journal.remove_files.len()
        || reused.len() != journal.reused_files.len()
        || !writes.is_disjoint(&removals)
        || !reused.is_subset(&writes)
    {
        return Err(LauncherError::Transaction(
            "abandoned transaction journal contains inconsistent file sets".to_string(),
        ));
    }
    for relative in journal
        .write_files
        .iter()
        .chain(&journal.remove_files)
        .chain(&journal.reused_files)
    {
        transaction_path(Path::new("."), relative)?;
    }
    Ok(())
}

fn transaction_path(root: &Path, relative: &str) -> Result<PathBuf, LauncherError> {
    let normalized = portable_relative_path(Path::new(relative))?;
    if normalized != relative {
        return Err(LauncherError::Transaction(format!(
            "transaction path '{relative}' is not normalized"
        )));
    }
    Ok(root.join(relative))
}

fn replace_with_backup(target: &Path, backup: &Path) -> Result<(), LauncherError> {
    if target.exists() {
        std::fs::remove_file(target)?;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(backup, target)?;
    Ok(())
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

fn rollback_transaction(
    root: &Path,
    backup_root: &Path,
    committed: &[String],
    backed_up: &[String],
) -> Result<(), LauncherError> {
    for relative in committed.iter().rev() {
        let target = root.join(relative);
        if target.exists() {
            std::fs::remove_file(&target)?;
        }
    }
    restore_backups(root, backup_root, backed_up)
}

fn restore_backups(
    root: &Path,
    backup_root: &Path,
    backed_up: &[String],
) -> Result<(), LauncherError> {
    for relative in backed_up.iter().rev() {
        let target = root.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(backup_root.join(relative), target)?;
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

    fn server_location(root: &Path) -> InstanceLocation {
        InstanceLocation::server(dunce::canonicalize(root).unwrap()).unwrap()
    }

    #[test]
    fn installer_profile_cleanup_removes_only_installer_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let profile_directory = directory.path().join("versions/forge-demo");
        std::fs::create_dir_all(&profile_directory).unwrap();
        std::fs::write(profile_directory.join("forge-demo.json"), b"{}").unwrap();
        std::fs::create_dir_all(directory.path().join("libraries/example")).unwrap();
        std::fs::write(
            directory.path().join("libraries/example/runtime.jar"),
            b"jar",
        )
        .unwrap();

        remove_installed_client_profile(directory.path(), "versions/forge-demo/forge-demo.json")
            .unwrap();

        assert!(!directory.path().join("versions").exists());
        assert!(
            directory
                .path()
                .join("libraries/example/runtime.jar")
                .is_file()
        );
    }

    fn previous_server_lock(id: Uuid) -> LauncherLock {
        LauncherLock {
            schema: LOCK_SCHEMA,
            instance_id: id,
            kind: InstanceKind::Server,
            minecraft: LockedMinecraft {
                version: "1.21.1".to_string(),
                version_type: "release".to_string(),
                asset_index: None,
                version_manifest_url: VERSION_MANIFEST_V2_URL.to_string(),
                version_manifest_sha256: "a".repeat(64),
                version_json_url: "https://piston-meta.mojang.com/version.json".to_string(),
                version_json_sha1: "b".repeat(40),
            },
            loader: LockedLoader::vanilla(),
            java: None,
            authlib_injector: None,
            entrypoint: LockedEntrypoint::Jar {
                path: "old/server.jar".to_string(),
            },
            arguments: LockedArguments::default(),
            artifacts: vec![LockedArtifact {
                logical_name: "old server".to_string(),
                owner: ArtifactOwner::Minecraft,
                source: LockedArtifactSource::Download {
                    url: "https://piston-data.mojang.com/old-server.jar".to_string(),
                    upstream_sha1: Some("c".repeat(40)),
                },
                sha256: "d".repeat(64),
                size: 3,
                path: "old/server.jar".to_string(),
                native_extraction: None,
            }],
            generated_files: vec!["old/generated.txt".to_string()],
            eula: Some(EulaAcceptance {
                url: crate::eula::MINECRAFT_EULA_URL.to_string(),
                digest_sha256: "e".repeat(64),
                accepted_at_unix_seconds: 1,
                method: crate::eula::EulaAcceptanceMethod::DigestCommand,
            }),
        }
    }

    #[test]
    fn transaction_refuses_to_replace_an_unowned_server_jar() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("server.jar"), b"user file").unwrap();
        let transaction =
            InstallTransaction::begin(&server_location(directory.path()), "test").unwrap();
        std::fs::write(transaction.staging.join("server.jar"), b"new").unwrap();
        std::fs::write(transaction.staging.join("eula.txt"), b"eula=true\n").unwrap();
        std::fs::write(transaction.staging.join(INSTANCE_LOCK_FILE), b"not reached").unwrap();
        assert!(
            transaction
                .commit(&[
                    "server.jar".to_string(),
                    "eula.txt".to_string(),
                    INSTANCE_LOCK_FILE.to_string(),
                ])
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
        let location = server_location(directory.path());
        let first = InstallTransaction::begin(&location, "first").unwrap();
        assert!(InstallTransaction::begin(&location, "second").is_err());
        drop(first);
        assert!(InstallTransaction::begin(&location, "third").is_ok());
    }

    #[test]
    fn next_transaction_rolls_back_an_abandoned_commit_journal() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join(STATE_DIRECTORY);
        let id = Uuid::new_v4();
        let staging = state.join("staging").join(id.to_string());
        write_staged_file(&staging.join("backup"), "replaced.jar", b"old").unwrap();
        write_staged_file(&staging.join("backup"), "removed.jar", b"removed").unwrap();
        std::fs::write(directory.path().join("replaced.jar"), b"new").unwrap();
        std::fs::write(directory.path().join("created.jar"), b"created").unwrap();
        std::fs::write(directory.path().join("shared.jar"), b"shared").unwrap();
        write_json_atomic(
            &state.join(TRANSACTION_JOURNAL),
            &TransactionJournal {
                schema: 1,
                id,
                phase: "committing".to_string(),
                write_files: vec![
                    "replaced.jar".to_string(),
                    "created.jar".to_string(),
                    "shared.jar".to_string(),
                ],
                remove_files: vec!["removed.jar".to_string()],
                reused_files: vec!["shared.jar".to_string()],
            },
        )
        .unwrap();

        let transaction =
            InstallTransaction::begin(&server_location(directory.path()), "recover").unwrap();

        assert_eq!(
            std::fs::read(directory.path().join("replaced.jar")).unwrap(),
            b"old"
        );
        assert_eq!(
            std::fs::read(directory.path().join("removed.jar")).unwrap(),
            b"removed"
        );
        assert_eq!(
            std::fs::read(directory.path().join("shared.jar")).unwrap(),
            b"shared"
        );
        assert!(!directory.path().join("created.jar").exists());
        assert!(!state.join(TRANSACTION_JOURNAL).exists());
        drop(transaction);
    }

    #[test]
    fn abandoned_transaction_paths_are_validated_before_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join(STATE_DIRECTORY);
        let id = Uuid::new_v4();
        std::fs::create_dir_all(state.join("staging").join(id.to_string())).unwrap();
        write_json_atomic(
            &state.join(TRANSACTION_JOURNAL),
            &TransactionJournal {
                schema: 1,
                id,
                phase: "committing".to_string(),
                write_files: vec!["../outside.jar".to_string()],
                remove_files: Vec::new(),
                reused_files: Vec::new(),
            },
        )
        .unwrap();

        assert!(InstallTransaction::begin(&server_location(directory.path()), "recover").is_err());
        assert!(state.join(TRANSACTION_JOURNAL).exists());
    }

    #[test]
    fn commit_creates_nested_parents_and_removes_stale_lock_owned_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("old")).unwrap();
        std::fs::write(directory.path().join("old/server.jar"), b"old").unwrap();
        std::fs::write(directory.path().join("old/generated.txt"), b"old").unwrap();
        LockFile::new(directory.path(), previous_server_lock(Uuid::new_v4()))
            .save()
            .unwrap();

        let transaction =
            InstallTransaction::begin(&server_location(directory.path()), "update").unwrap();
        write_staged_file(&transaction.staging, "new/nested/server.jar", b"new").unwrap();
        write_staged_file(&transaction.staging, INSTANCE_LOCK_FILE, b"new lock").unwrap();
        transaction
            .commit(&[
                "new/nested/server.jar".to_string(),
                INSTANCE_LOCK_FILE.to_string(),
            ])
            .unwrap();

        assert_eq!(
            std::fs::read(directory.path().join("new/nested/server.jar")).unwrap(),
            b"new"
        );
        assert!(!directory.path().join("old/server.jar").exists());
        assert!(!directory.path().join("old/generated.txt").exists());
    }
}
