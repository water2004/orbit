use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::artifact::{ArtifactRequest, ExpectedHash};
use crate::error::LauncherError;
use crate::instance::LoaderKind;
use crate::maven::artifact_path;
use crate::platform::{HostPlatform, OperatingSystem};

const FORGE_METADATA_URL: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json";
const FORGE_PROMOTIONS_URL: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const FORGE_MAVEN_ROOT: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const NEOFORGE_VERSIONS_URL: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
const NEOFORGE_LEGACY_VERSIONS_URL: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/forge";
const NEOFORGE_MAVEN_ROOT: &str = "https://maven.neoforged.net/releases/net/neoforged";
const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;
const MAX_INSTALL_PROFILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EMITTED_INSTALLER_LINES: usize = 2_000;
const MAX_INSTALLER_LINE_BYTES: usize = 16 * 1024;
pub const INSTALLER_STAGING_NAME: &str = ".orbit-loader-installer.jar";

#[derive(Debug, Clone)]
pub struct ResolvedLoaderInstaller {
    pub kind: LoaderKind,
    pub minecraft_version: String,
    pub version: String,
    pub artifact: ArtifactRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedLoaderInstaller {
    pub install_profile_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerSide {
    Client,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderInstallerEvent {
    Started {
        kind: LoaderKind,
        version: String,
        side: InstallerSide,
    },
    Output {
        stream: InstallerOutputStream,
        line: String,
    },
    OutputSuppressed {
        maximum_lines: usize,
    },
    Finished {
        kind: LoaderKind,
        version: String,
    },
}

#[derive(Debug, Clone)]
pub struct InstalledClientProfile {
    pub profile_path: String,
    pub main_class: String,
    pub game_arguments: Vec<String>,
    pub jvm_arguments: Vec<String>,
    pub classpath: Vec<String>,
}

pub async fn resolve_loader_installer(
    client: &reqwest::Client,
    kind: LoaderKind,
    minecraft_version: &str,
    requirement: &str,
) -> Result<ResolvedLoaderInstaller, LauncherError> {
    validate_identifier(minecraft_version, "Minecraft version")?;
    validate_requirement(requirement)?;
    let (version, url) = match kind {
        LoaderKind::Forge => {
            let version = resolve_forge_version(client, minecraft_version, requirement).await?;
            let url = format!("{FORGE_MAVEN_ROOT}/{version}/forge-{version}-installer.jar");
            (version, url)
        }
        LoaderKind::Neoforge => {
            resolve_neoforge_version(client, minecraft_version, requirement).await?
        }
        _ => {
            return Err(LauncherError::UnsupportedRequirement(format!(
                "Loader '{}' does not publish an official installer artifact",
                kind.as_str()
            )));
        }
    };
    let sha256 = fetch_digest_sidecar(client, &format!("{url}.sha256"), 64).await?;
    let artifact = ArtifactRequest {
        logical_name: format!("{} {version} installer", kind.as_str()),
        url,
        expected_hash: ExpectedHash::Sha256(sha256),
        expected_size: None,
    };
    artifact.validate()?;
    Ok(ResolvedLoaderInstaller {
        kind,
        minecraft_version: minecraft_version.to_string(),
        version,
        artifact,
    })
}

pub fn inspect_loader_installer(
    path: &Path,
    resolved: &ResolvedLoaderInstaller,
) -> Result<InspectedLoaderInstaller, LauncherError> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "{} {} installer is not a readable JAR: {error}",
            resolved.kind.as_str(),
            resolved.version
        ))
    })?;
    let mut entry = archive.by_name("install_profile.json").map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "{} {} installer has no install_profile.json: {error}",
            resolved.kind.as_str(),
            resolved.version
        ))
    })?;
    if entry.size() == 0 || entry.size() > MAX_INSTALL_PROFILE_BYTES {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{} {} install_profile.json has an invalid size",
            resolved.kind.as_str(),
            resolved.version
        )));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes)?;
    if bytes.len() as u64 != entry.size() {
        return Err(LauncherError::InvalidRemoteData(
            "Loader installer profile was truncated while reading".to_string(),
        ));
    }
    let profile: InstallProfile = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse {} {} install_profile.json: {error}",
            resolved.kind.as_str(),
            resolved.version
        ))
    })?;
    if profile.spec != 1 {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "{} {} installer schema {} is unsupported",
            resolved.kind.as_str(),
            resolved.version,
            profile.spec
        )));
    }
    if profile.minecraft != resolved.minecraft_version {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{} {} installer targets Minecraft '{}' instead of resolved '{}'",
            resolved.kind.as_str(),
            resolved.version,
            profile.minecraft,
            resolved.minecraft_version
        )));
    }
    if profile.version.trim().is_empty() || profile.profile.trim().is_empty() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{} {} installer profile is incomplete",
            resolved.kind.as_str(),
            resolved.version
        )));
    }
    Ok(InspectedLoaderInstaller {
        install_profile_sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

pub async fn run_loader_installer<F>(
    java_executable: &Path,
    installer_path: &Path,
    resolved: &ResolvedLoaderInstaller,
    side: InstallerSide,
    staging: &Path,
    timeout: std::time::Duration,
    mut progress: F,
) -> Result<(), LauncherError>
where
    F: FnMut(LoaderInstallerEvent) + Send,
{
    if !java_executable.is_file() {
        return Err(LauncherError::Transaction(format!(
            "managed Java executable '{}' does not exist",
            java_executable.display()
        )));
    }
    if !installer_path.is_file() || !staging.is_dir() {
        return Err(LauncherError::Transaction(
            "Loader installer execution paths are incomplete".to_string(),
        ));
    }
    if timeout.is_zero() {
        return Err(LauncherError::InvalidConfig(
            "Loader installer timeout must be greater than zero".to_string(),
        ));
    }
    if side == InstallerSide::Client {
        std::fs::write(
            staging.join("launcher_profiles.json"),
            b"{\"profiles\":{},\"settings\":{},\"version\":3}\n",
        )?;
    }
    progress(LoaderInstallerEvent::Started {
        kind: resolved.kind,
        version: resolved.version.clone(),
        side,
    });
    let argument = match side {
        InstallerSide::Client => "--installClient",
        InstallerSide::Server => "--installServer",
    };
    let mut child = Command::new(java_executable)
        .arg("-jar")
        .arg(installer_path)
        .arg(argument)
        .arg(staging)
        .current_dir(staging)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| {
        LauncherError::Transaction("Loader installer stdout was not captured".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        LauncherError::Transaction("Loader installer stderr was not captured".to_string())
    })?;
    let (sender, mut receiver) = mpsc::channel(256);
    let stdout_task = tokio::spawn(read_installer_output(
        stdout,
        InstallerOutputStream::Stdout,
        sender.clone(),
    ));
    let stderr_task = tokio::spawn(read_installer_output(
        stderr,
        InstallerOutputStream::Stderr,
        sender,
    ));
    let execution = async move {
        let mut emitted = 0_usize;
        let mut suppression_emitted = false;
        while let Some(event) = receiver.recv().await {
            if emitted < MAX_EMITTED_INSTALLER_LINES {
                progress(event);
                emitted += 1;
            } else if !suppression_emitted {
                progress(LoaderInstallerEvent::OutputSuppressed {
                    maximum_lines: MAX_EMITTED_INSTALLER_LINES,
                });
                suppression_emitted = true;
            }
        }
        join_output_reader(stdout_task).await?;
        join_output_reader(stderr_task).await?;
        let status = child.wait().await?;
        Ok::<_, LauncherError>((status, progress))
    };
    let (status, mut progress) =
        tokio::time::timeout(timeout, execution)
            .await
            .map_err(|_| {
                LauncherError::Transaction(format!(
                    "{} {} official installer exceeded its {} second timeout",
                    resolved.kind.as_str(),
                    resolved.version,
                    timeout.as_secs()
                ))
            })??;
    if !status.success() {
        return Err(LauncherError::Transaction(format!(
            "{} {} official installer exited with status {status}",
            resolved.kind.as_str(),
            resolved.version
        )));
    }
    progress(LoaderInstallerEvent::Finished {
        kind: resolved.kind,
        version: resolved.version.clone(),
    });
    Ok(())
}

pub fn read_installed_client_profile(
    staging: &Path,
    resolved: &ResolvedLoaderInstaller,
    platform: &HostPlatform,
) -> Result<InstalledClientProfile, LauncherError> {
    let versions = staging.join("versions");
    let mut candidates = Vec::new();
    if versions.is_dir() {
        for directory in std::fs::read_dir(&versions)? {
            let directory = directory?;
            if !directory.file_type()?.is_dir() {
                continue;
            }
            for file in std::fs::read_dir(directory.path())? {
                let file = file?;
                if file.file_type()?.is_file()
                    && file.path().extension().and_then(|value| value.to_str()) == Some("json")
                {
                    let bytes = std::fs::read(file.path())?;
                    let Ok(profile) = serde_json::from_slice::<InstalledVersionProfile>(&bytes)
                    else {
                        continue;
                    };
                    if profile.inherits_from.as_deref() == Some(&resolved.minecraft_version) {
                        candidates.push((file.path(), profile));
                    }
                }
            }
        }
    }
    if candidates.len() != 1 {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{} {} installer produced {} inherited client profiles; expected exactly one",
            resolved.kind.as_str(),
            resolved.version,
            candidates.len()
        )));
    }
    let (path, profile) = candidates.pop().expect("one candidate was checked");
    validate_profile_text(&profile.main_class, "installer main class")?;
    let (game_arguments, jvm_arguments) = installed_profile_arguments(&profile)?;
    let mut classpath = Vec::new();
    for library in profile.libraries {
        if !library.rules_allow(platform)? {
            continue;
        }
        let path = library
            .downloads
            .and_then(|downloads| downloads.artifact)
            .map(|artifact| artifact.path)
            .filter(|path| !path.trim().is_empty())
            .unwrap_or(artifact_path(&library.name, None)?);
        let target = format!("libraries/{path}");
        if !staging.join(&target).is_file() {
            return Err(LauncherError::InvalidRemoteData(format!(
                "installer profile library '{}' is missing from '{}'",
                library.name, target
            )));
        }
        if !classpath.contains(&target) {
            classpath.push(target);
        }
    }
    let profile_path = path.strip_prefix(staging).map_err(|_| {
        LauncherError::Transaction("installed Loader profile escaped staging".to_string())
    })?;
    Ok(InstalledClientProfile {
        profile_path: crate::lockfile::portable_relative_path(profile_path)?,
        main_class: profile.main_class,
        game_arguments,
        jvm_arguments,
        classpath,
    })
}

pub fn installed_server_argument_file(
    staging: &Path,
    resolved: &ResolvedLoaderInstaller,
    platform: &HostPlatform,
) -> Result<String, LauncherError> {
    let filename = match platform.os {
        OperatingSystem::Windows => "win_args.txt",
        OperatingSystem::Linux | OperatingSystem::MacOs => "unix_args.txt",
    };
    let group = match resolved.kind {
        LoaderKind::Forge => "net/minecraftforge/forge",
        LoaderKind::Neoforge if resolved.minecraft_version == "1.20.1" => "net/neoforged/forge",
        LoaderKind::Neoforge => "net/neoforged/neoforge",
        _ => {
            return Err(LauncherError::UnsupportedRequirement(
                "server argument files only apply to installer Loaders".to_string(),
            ));
        }
    };
    let relative = format!("libraries/{group}/{}/{filename}", resolved.version);
    let path = staging.join(&relative);
    if !path.is_file() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{} {} installer did not produce expected server argument file '{relative}'",
            resolved.kind.as_str(),
            resolved.version
        )));
    }
    let metadata = path.metadata()?;
    if metadata.len() == 0 || metadata.len() > 8 * 1024 * 1024 {
        return Err(LauncherError::InvalidRemoteData(format!(
            "server argument file '{relative}' has an invalid size"
        )));
    }
    Ok(relative)
}

async fn read_installer_output<R>(
    stream: R,
    kind: InstallerOutputStream,
    sender: mpsc::Sender<LoaderInstallerEvent>,
) -> Result<(), std::io::Error>
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let count = reader.read_until(b'\n', &mut bytes).await?;
        if count == 0 {
            return Ok(());
        }
        if bytes.len() > MAX_INSTALLER_LINE_BYTES {
            bytes.truncate(MAX_INSTALLER_LINE_BYTES);
        }
        let line = String::from_utf8_lossy(&bytes)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if !line.is_empty()
            && sender
                .send(LoaderInstallerEvent::Output { stream: kind, line })
                .await
                .is_err()
        {
            return Ok(());
        }
    }
}

async fn join_output_reader(
    task: tokio::task::JoinHandle<Result<(), std::io::Error>>,
) -> Result<(), LauncherError> {
    task.await.map_err(|error| {
        LauncherError::Transaction(format!("Loader installer output reader failed: {error}"))
    })??;
    Ok(())
}

fn installed_profile_arguments(
    profile: &InstalledVersionProfile,
) -> Result<(Vec<String>, Vec<String>), LauncherError> {
    if let Some(arguments) = &profile.arguments {
        return Ok((
            string_values(&arguments.game, "installer game arguments")?,
            string_values(&arguments.jvm, "installer JVM arguments")?,
        ));
    }
    let game = profile.minecraft_arguments.as_ref().ok_or_else(|| {
        LauncherError::InvalidRemoteData(
            "installed Loader profile has no supported argument model".to_string(),
        )
    })?;
    Ok((
        shell_words::split(game).map_err(|error| {
            LauncherError::InvalidRemoteData(format!(
                "installed Loader legacy arguments are invalid: {error}"
            ))
        })?,
        Vec::new(),
    ))
}

fn string_values(
    values: &[serde_json::Value],
    subject: &str,
) -> Result<Vec<String>, LauncherError> {
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.chars().any(char::is_control))
                .map(str::to_string)
                .ok_or_else(|| {
                    LauncherError::InvalidRemoteData(format!(
                        "{subject} contain a non-string value"
                    ))
                })
        })
        .collect()
}

fn validate_profile_text(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} is invalid"
        )));
    }
    Ok(())
}

async fn resolve_forge_version(
    client: &reqwest::Client,
    minecraft: &str,
    requirement: &str,
) -> Result<String, LauncherError> {
    let bytes = fetch_bounded(
        client,
        FORGE_METADATA_URL,
        MAX_METADATA_BYTES,
        "Forge version index",
    )
    .await?;
    let index: BTreeMap<String, Vec<String>> = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("failed to parse Forge version index: {error}"))
    })?;
    let compatible = index.get(minecraft).ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Forge publishes no installer versions for Minecraft {minecraft}"
        ))
    })?;
    let selected = match requirement {
        "latest" | "stable" => {
            let bytes = fetch_bounded(
                client,
                FORGE_PROMOTIONS_URL,
                MAX_METADATA_BYTES,
                "Forge promotions index",
            )
            .await?;
            let promotions: ForgePromotions = serde_json::from_slice(&bytes).map_err(|error| {
                LauncherError::InvalidRemoteData(format!(
                    "failed to parse Forge promotions index: {error}"
                ))
            })?;
            let channel = if requirement == "latest" {
                "latest"
            } else {
                "recommended"
            };
            let promoted = promotions
                .promos
                .get(&format!("{minecraft}-{channel}"))
                .ok_or_else(|| {
                    LauncherError::UnsupportedRequirement(format!(
                        "Forge has no {channel} promotion for Minecraft {minecraft}; use an exact version"
                    ))
                })?;
            format!("{minecraft}-{promoted}")
        }
        exact if exact.starts_with(&format!("{minecraft}-")) => exact.to_string(),
        exact => format!("{minecraft}-{exact}"),
    };
    compatible
        .iter()
        .any(|candidate| candidate == &selected)
        .then_some(selected)
        .ok_or_else(|| {
            LauncherError::UnsupportedRequirement(format!(
                "Forge requirement '{requirement}' has no installer for Minecraft {minecraft}"
            ))
        })
}

async fn resolve_neoforge_version(
    client: &reqwest::Client,
    minecraft: &str,
    requirement: &str,
) -> Result<(String, String), LauncherError> {
    let legacy = minecraft == "1.20.1";
    let endpoint = if legacy {
        NEOFORGE_LEGACY_VERSIONS_URL
    } else {
        NEOFORGE_VERSIONS_URL
    };
    let bytes = fetch_bounded(
        client,
        endpoint,
        MAX_METADATA_BYTES,
        "NeoForge version index",
    )
    .await?;
    let index: NeoForgeVersionIndex = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("failed to parse NeoForge version index: {error}"))
    })?;
    let compatible: Vec<_> = index
        .versions
        .into_iter()
        .filter(|version| neoforge_minecraft_version(version, legacy).as_deref() == Some(minecraft))
        .collect();
    let selected = match requirement {
        "latest" => compatible.last().cloned(),
        "stable" => compatible
            .iter()
            .rev()
            .find(|version| !is_prerelease(version))
            .cloned(),
        exact => compatible
            .iter()
            .find(|version| version.as_str() == exact)
            .cloned(),
    }
    .ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "NeoForge requirement '{requirement}' has no installer for Minecraft {minecraft}"
        ))
    })?;
    let (artifact, root) = if legacy {
        ("forge", format!("{NEOFORGE_MAVEN_ROOT}/forge"))
    } else {
        ("neoforge", format!("{NEOFORGE_MAVEN_ROOT}/neoforge"))
    };
    let url = format!("{root}/{selected}/{artifact}-{selected}-installer.jar");
    Ok((selected, url))
}

fn neoforge_minecraft_version(version: &str, legacy: bool) -> Option<String> {
    if legacy {
        return version
            .strip_prefix("1.20.1-")
            .or(Some(version))
            .filter(|value| value.starts_with("47.1."))
            .map(|_| "1.20.1".to_string());
    }
    if let Some(rest) = version.strip_prefix("0.") {
        let snapshot = rest.split('.').next()?;
        return (!snapshot.is_empty()).then(|| snapshot.to_string());
    }
    let numeric = version.split(['-', '+']).next()?;
    let parts: Vec<_> = numeric.split('.').collect();
    let major: u32 = parts.first()?.parse().ok()?;
    if major >= 26 {
        return (parts.len() >= 4).then(|| parts[..parts.len() - 1].join("."));
    }
    let minor = parts.get(1)?.parse::<u32>().ok()?;
    Some(if minor == 0 {
        format!("1.{major}")
    } else {
        format!("1.{major}.{minor}")
    })
}

fn is_prerelease(version: &str) -> bool {
    version
        .split_once('-')
        .is_some_and(|(_, suffix)| !suffix.is_empty())
}

async fn fetch_digest_sidecar(
    client: &reqwest::Client,
    url: &str,
    length: usize,
) -> Result<String, LauncherError> {
    let bytes = fetch_bounded(client, url, 1024, "Maven digest sidecar").await?;
    let value = std::str::from_utf8(&bytes)
        .map_err(|error| {
            LauncherError::InvalidRemoteData(format!("Maven digest sidecar is not UTF-8: {error}"))
        })?
        .trim()
        .to_ascii_lowercase();
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidRemoteData(
            "Maven digest sidecar contains an invalid digest".to_string(),
        ));
    }
    Ok(value)
}

async fn fetch_bounded(
    client: &reqwest::Client,
    url: &str,
    maximum: u64,
    subject: &str,
) -> Result<Vec<u8>, LauncherError> {
    let parsed = url::Url::parse(url).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("{subject} URL is invalid: {error}"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} URL must use HTTPS"
        )));
    }
    let response = client.get(parsed).send().await?.error_for_status()?;
    if response.url().scheme() != "https"
        || response.content_length().is_some_and(|size| size > maximum)
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} response URL or size is invalid"
        )));
    }
    let bytes = response.bytes().await?;
    if bytes.len() as u64 > maximum {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} exceeds {maximum} bytes"
        )));
    }
    Ok(bytes.to_vec())
}

fn validate_requirement(value: &str) -> Result<(), LauncherError> {
    if matches!(value, "latest" | "stable") {
        Ok(())
    } else {
        validate_identifier(value, "Loader version")
    }
}

fn validate_identifier(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 192
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
        })
    {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "{subject} '{value}' is not a supported exact identifier"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ForgePromotions {
    promos: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct NeoForgeVersionIndex {
    versions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct InstallProfile {
    spec: u32,
    minecraft: String,
    profile: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct InstalledVersionProfile {
    #[serde(rename = "inheritsFrom")]
    inherits_from: Option<String>,
    #[serde(rename = "mainClass")]
    main_class: String,
    arguments: Option<InstalledArguments>,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    #[serde(default)]
    libraries: Vec<InstalledLibrary>,
}

#[derive(Debug, Deserialize)]
struct InstalledArguments {
    #[serde(default)]
    game: Vec<serde_json::Value>,
    #[serde(default)]
    jvm: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct InstalledLibrary {
    name: String,
    downloads: Option<InstalledLibraryDownloads>,
    #[serde(default)]
    rules: Vec<InstalledRule>,
}

impl InstalledLibrary {
    fn rules_allow(&self, platform: &HostPlatform) -> Result<bool, LauncherError> {
        if self.rules.is_empty() {
            return Ok(true);
        }
        let mut allowed = false;
        for rule in &self.rules {
            if rule.matches(platform)? {
                allowed = rule.action == InstalledRuleAction::Allow;
            }
        }
        Ok(allowed)
    }
}

#[derive(Debug, Deserialize)]
struct InstalledLibraryDownloads {
    artifact: Option<InstalledLibraryArtifact>,
}

#[derive(Debug, Deserialize)]
struct InstalledLibraryArtifact {
    path: String,
}

#[derive(Debug, Deserialize)]
struct InstalledRule {
    action: InstalledRuleAction,
    os: Option<InstalledOsRule>,
}

impl InstalledRule {
    fn matches(&self, platform: &HostPlatform) -> Result<bool, LauncherError> {
        let Some(os) = &self.os else {
            return Ok(true);
        };
        if os
            .name
            .as_deref()
            .is_some_and(|name| name != platform.os.mojang_name())
        {
            return Ok(false);
        }
        if let Some(pattern) = &os.version
            && !regex::Regex::new(&format!("^(?:{pattern})$"))
                .map_err(|error| {
                    LauncherError::InvalidRemoteData(format!(
                        "installed Loader OS version rule is invalid: {error}"
                    ))
                })?
                .is_match(&platform.os_version)
        {
            return Ok(false);
        }
        if let Some(pattern) = &os.arch
            && !regex::Regex::new(&format!("^(?:{pattern})$"))
                .map_err(|error| {
                    LauncherError::InvalidRemoteData(format!(
                        "installed Loader architecture rule is invalid: {error}"
                    ))
                })?
                .is_match(platform.architecture.rule_name())
        {
            return Ok(false);
        }
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum InstalledRuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Deserialize)]
struct InstalledOsRule {
    name: Option<String>,
    version: Option<String>,
    arch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_versions_map_to_their_real_minecraft_line() {
        assert_eq!(
            neoforge_minecraft_version("21.1.244", false).as_deref(),
            Some("1.21.1")
        );
        assert_eq!(
            neoforge_minecraft_version("26.1.2.87", false).as_deref(),
            Some("26.1.2")
        );
        assert_eq!(
            neoforge_minecraft_version("1.20.1-47.1.106", true).as_deref(),
            Some("1.20.1")
        );
        assert_eq!(
            neoforge_minecraft_version("47.1.82", true).as_deref(),
            Some("1.20.1")
        );
        assert_eq!(
            neoforge_minecraft_version("0.25w14craftmine.3-beta", false).as_deref(),
            Some("25w14craftmine")
        );
    }

    #[tokio::test]
    #[ignore = "uses live Forge and NeoForge metadata services"]
    async fn live_installer_resolution_uses_verified_official_artifacts() {
        let client = reqwest::Client::builder()
            .user_agent("orbit-launcher-tests/0.1")
            .build()
            .unwrap();
        for (kind, minecraft, requirement) in [
            (LoaderKind::Forge, "1.21.1", "stable"),
            (LoaderKind::Neoforge, "1.21.1", "latest"),
            (LoaderKind::Neoforge, "26.1.2", "latest"),
        ] {
            let resolved = resolve_loader_installer(&client, kind, minecraft, requirement)
                .await
                .unwrap();
            assert!(matches!(
                resolved.artifact.expected_hash,
                ExpectedHash::Sha256(_)
            ));
        }
    }
}
