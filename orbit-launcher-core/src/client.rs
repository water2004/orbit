use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use serde::Deserialize;
use sha1::{Digest, Sha1};

use crate::artifact::{ArtifactRequest, ExpectedHash};
use crate::error::LauncherError;
use crate::lockfile::ArtifactOwner;
use crate::maven::artifact_url;
use crate::mojang::{MojangClient, MojangJavaRequirement};
use crate::platform::HostPlatform;

const MAX_ASSET_INDEX_BYTES: u64 = 64 * 1024 * 1024;
const ASSET_OBJECT_ROOT: &str = "https://resources.download.minecraft.net/";
const DEFAULT_LIBRARY_ROOT: &str = "https://libraries.minecraft.net/";

#[derive(Debug, Clone)]
pub struct ResolvedVanillaClient {
    pub minecraft_version: String,
    pub version_type: String,
    pub version_manifest_sha256: String,
    pub version_json_url: String,
    pub version_json_sha1: String,
    pub version_json_bytes: Vec<u8>,
    pub main_class: String,
    pub java: Option<MojangJavaRequirement>,
    pub game_arguments: Vec<String>,
    pub jvm_arguments: Vec<String>,
    pub downloads: Vec<ClientDownload>,
    pub asset_mappings: Vec<AssetMapping>,
    pub classpath: Vec<String>,
    pub asset_index_id: String,
    pub legacy_virtual_assets: bool,
    pub map_assets_to_resources: bool,
}

#[derive(Debug, Clone)]
pub struct ClientDownload {
    pub request: ArtifactRequest,
    pub target: String,
    pub owner: ArtifactOwner,
    pub native_extract: Option<NativeExtract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetMapping {
    pub logical_path: String,
    pub object_path: String,
}

#[derive(Debug, Clone)]
pub struct NativeExtract {
    pub excludes: Vec<String>,
}

pub async fn resolve_vanilla_client(
    mojang: &MojangClient,
    requirement: &str,
    platform: &HostPlatform,
) -> Result<ResolvedVanillaClient, LauncherError> {
    let document = mojang.fetch_version_document(requirement).await?;
    let version: ClientVersionJson = serde_json::from_slice(&document.bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse Mojang client version JSON '{}': {error}",
            document.id
        ))
    })?;
    if version.main_class.trim().is_empty() || version.main_class.chars().any(char::is_control) {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Minecraft '{}' has an invalid main class",
            document.id
        )));
    }
    let (game_arguments, mut jvm_arguments) = resolve_arguments(&version, platform)?;

    let mut downloads = Vec::new();
    let mut classpath = Vec::new();
    let client = version.downloads.client.ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Minecraft '{}' does not publish a client JAR",
            document.id
        ))
    })?;
    client.validate("client JAR")?;
    let client_target = format!("versions/{0}/{0}.jar", document.id);

    for library in version.libraries {
        if !rules_allow(&library.rules, platform, &default_features())? {
            continue;
        }
        if let Some(download) = library
            .downloads
            .as_ref()
            .and_then(|downloads| downloads.artifact.as_ref())
        {
            let resolved =
                library_download(&library.name, download, ArtifactOwner::Minecraft, None)?;
            classpath.push(resolved.target.clone());
            downloads.push(resolved);
        } else if library.downloads.is_none() {
            let resolved = legacy_library_download(&library, None, false)?;
            classpath.push(resolved.target.clone());
            downloads.push(resolved);
        }

        let native_classifier = library
            .natives
            .as_ref()
            .and_then(|natives| natives.get(platform.os.mojang_name()))
            .map(|classifier| classifier.replace("${arch}", platform.architecture.bits()));
        if let Some(classifier) = native_classifier {
            let extract = Some(NativeExtract {
                excludes: library
                    .extract
                    .as_ref()
                    .map(|extract| extract.exclude.clone())
                    .unwrap_or_default(),
            });
            if let Some(download) = library
                .downloads
                .as_ref()
                .and_then(|downloads| downloads.classifiers.get(&classifier))
            {
                downloads.push(library_download(
                    &library.name,
                    download,
                    ArtifactOwner::Minecraft,
                    extract,
                )?);
            } else if library.downloads.is_none() {
                downloads.push(legacy_library_download(&library, Some(&classifier), true)?);
            } else {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "library '{}' declares native classifier '{}' without download metadata",
                    library.name, classifier
                )));
            }
        }
    }

    classpath.push(client_target.clone());
    downloads.push(ClientDownload {
        request: client.request(format!("Minecraft {} client", document.id))?,
        target: client_target,
        owner: ArtifactOwner::Minecraft,
        native_extract: None,
    });

    let asset_index = version.asset_index.ok_or_else(|| {
        LauncherError::UnsupportedRequirement(format!(
            "Minecraft '{}' does not publish an asset index",
            document.id
        ))
    })?;
    asset_index.download.validate("asset index")?;
    let asset_bytes = fetch_bounded(
        mojang.http_client(),
        &asset_index.download.url,
        MAX_ASSET_INDEX_BYTES,
        "asset index",
    )
    .await?;
    if hex::encode(Sha1::digest(&asset_bytes)) != asset_index.download.sha1 {
        return Err(LauncherError::ArtifactIntegrity(format!(
            "Minecraft '{}' asset index did not match its SHA-1",
            document.id
        )));
    }
    let assets: AssetIndex = serde_json::from_slice(&asset_bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to parse Minecraft '{}' asset index: {error}",
            document.id
        ))
    })?;
    downloads.push(ClientDownload {
        request: asset_index
            .download
            .request(format!("Minecraft {} asset index", asset_index.id))?,
        target: format!("assets/indexes/{}.json", asset_index.id),
        owner: ArtifactOwner::Minecraft,
        native_extract: None,
    });
    let mut asset_hashes = HashMap::new();
    let mut asset_mappings = Vec::new();
    for (logical_path, object) in assets.objects {
        validate_asset_logical_path(&logical_path)?;
        validate_sha1(&object.hash, "asset object")?;
        if object.size == 0 {
            return Err(LauncherError::InvalidRemoteData(format!(
                "asset '{logical_path}' has zero size"
            )));
        }
        let prefix = &object.hash[..2];
        let object_path = format!("assets/objects/{prefix}/{}", object.hash);
        asset_mappings.push(AssetMapping {
            logical_path: logical_path.clone(),
            object_path: object_path.clone(),
        });
        match asset_hashes.entry(object.hash.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(object.size);
                downloads.push(ClientDownload {
                    request: ArtifactRequest {
                        logical_name: format!("Minecraft asset {logical_path}"),
                        url: format!("{ASSET_OBJECT_ROOT}{prefix}/{}", object.hash),
                        expected_hash: ExpectedHash::Sha1(object.hash),
                        expected_size: Some(object.size),
                    },
                    target: object_path,
                    owner: ArtifactOwner::Minecraft,
                    native_extract: None,
                });
            }
            std::collections::hash_map::Entry::Occupied(entry) if *entry.get() != object.size => {
                return Err(LauncherError::InvalidRemoteData(format!(
                    "asset index assigns conflicting sizes to object hash '{}'",
                    entry.key()
                )));
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
    }

    let mut logging_argument = None;
    if let Some(logging) = version.logging.and_then(|logging| logging.client) {
        logging.file.validate("logging configuration")?;
        let target = format!("log_configs/{}", logging.file.id);
        if !logging.argument.contains("${path}") {
            return Err(LauncherError::InvalidRemoteData(
                "Minecraft logging argument does not contain the required '${path}' placeholder"
                    .to_string(),
            ));
        }
        logging_argument = Some(logging.argument.replace("${path}", &target));
        downloads.push(ClientDownload {
            request: logging.file.request(format!(
                "Minecraft logging configuration {}",
                logging.file.id
            ))?,
            target,
            owner: ArtifactOwner::Minecraft,
            native_extract: None,
        });
    }

    if let Some(argument) = logging_argument {
        jvm_arguments.push(argument);
    }
    let java = version.java_version.map(|java| MojangJavaRequirement {
        component: java.component,
        major: java.major_version,
    });
    if java
        .as_ref()
        .is_some_and(|java| java.component.trim().is_empty() || java.major == 0)
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Minecraft '{}' declares an invalid Java requirement",
            document.id
        )));
    }
    for download in &downloads {
        download.request.validate()?;
        validate_portable_target(&download.target)?;
    }
    if classpath.is_empty() {
        return Err(LauncherError::InvalidRemoteData(
            "resolved Minecraft classpath is empty".to_string(),
        ));
    }
    Ok(ResolvedVanillaClient {
        minecraft_version: document.id,
        version_type: document.version_type,
        version_manifest_sha256: document.version_manifest_sha256,
        version_json_url: document.version_json_url,
        version_json_sha1: document.version_json_sha1,
        version_json_bytes: document.bytes,
        main_class: version.main_class,
        java,
        game_arguments,
        jvm_arguments,
        downloads,
        asset_mappings,
        classpath,
        asset_index_id: asset_index.id,
        legacy_virtual_assets: assets.virtual_assets,
        map_assets_to_resources: assets.map_to_resources,
    })
}

fn resolve_arguments(
    version: &ClientVersionJson,
    platform: &HostPlatform,
) -> Result<(Vec<String>, Vec<String>), LauncherError> {
    if let Some(arguments) = &version.arguments {
        let features = default_features();
        Ok((
            flatten_arguments(&arguments.game, platform, &features)?,
            flatten_arguments(&arguments.jvm, platform, &features)?,
        ))
    } else if let Some(arguments) = &version.minecraft_arguments {
        Ok((
            shell_words::split(arguments).map_err(|error| {
                LauncherError::InvalidRemoteData(format!(
                    "legacy Minecraft arguments are invalid: {error}"
                ))
            })?,
            Vec::new(),
        ))
    } else {
        Err(LauncherError::InvalidRemoteData(
            "Minecraft version JSON contains neither arguments nor minecraftArguments".to_string(),
        ))
    }
}

fn flatten_arguments(
    arguments: &[Argument],
    platform: &HostPlatform,
    features: &HashMap<String, bool>,
) -> Result<Vec<String>, LauncherError> {
    let mut result = Vec::new();
    for argument in arguments {
        match argument {
            Argument::Plain(value) => result.push(value.clone()),
            Argument::Conditional { rules, value } if rules_allow(rules, platform, features)? => {
                match value {
                    ArgumentValue::One(value) => result.push(value.clone()),
                    ArgumentValue::Many(values) => result.extend(values.iter().cloned()),
                }
            }
            Argument::Conditional { .. } => {}
        }
    }
    Ok(result)
}

fn rules_allow(
    rules: &[Rule],
    platform: &HostPlatform,
    features: &HashMap<String, bool>,
) -> Result<bool, LauncherError> {
    if rules.is_empty() {
        return Ok(true);
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(platform, features)? {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    Ok(allowed)
}

impl Rule {
    fn matches(
        &self,
        platform: &HostPlatform,
        features: &HashMap<String, bool>,
    ) -> Result<bool, LauncherError> {
        if let Some(os) = &self.os {
            if let Some(name) = &os.name
                && name != platform.os.mojang_name()
            {
                return Ok(false);
            }
            if let Some(pattern) = &os.version
                && !full_regex(pattern, "OS version")?.is_match(&platform.os_version)
            {
                return Ok(false);
            }
            if let Some(pattern) = &os.arch
                && !full_regex(pattern, "architecture")?.is_match(platform.architecture.rule_name())
            {
                return Ok(false);
            }
        }
        Ok(self
            .features
            .iter()
            .all(|(name, expected)| features.get(name) == Some(expected)))
    }
}

fn full_regex(pattern: &str, subject: &str) -> Result<Regex, LauncherError> {
    Regex::new(&format!("^(?:{pattern})$")).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("invalid Mojang {subject} rule: {error}"))
    })
}

fn default_features() -> HashMap<String, bool> {
    [
        ("is_demo_user", false),
        ("has_custom_resolution", false),
        ("has_quick_plays_support", false),
        ("is_quick_play_singleplayer", false),
        ("is_quick_play_multiplayer", false),
        ("is_quick_play_realms", false),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect()
}

fn library_download(
    name: &str,
    download: &Download,
    owner: ArtifactOwner,
    native_extract: Option<NativeExtract>,
) -> Result<ClientDownload, LauncherError> {
    download.validate("library")?;
    validate_portable_target(&download.path)?;
    Ok(ClientDownload {
        request: download.request(format!("library {name}"))?,
        target: format!("libraries/{}", download.path),
        owner,
        native_extract,
    })
}

fn legacy_library_download(
    library: &Library,
    classifier: Option<&str>,
    native: bool,
) -> Result<ClientDownload, LauncherError> {
    let (path, url) = artifact_url(
        library.url.as_deref().unwrap_or(DEFAULT_LIBRARY_ROOT),
        &library.name,
        classifier,
    )?;
    let expected_hash = library
        .checksums
        .first()
        .map(|sha1| {
            validate_sha1(sha1, "legacy library")?;
            Ok::<ExpectedHash, LauncherError>(ExpectedHash::Sha1(sha1.clone()))
        })
        .transpose()?
        .unwrap_or(ExpectedHash::Unverified);
    Ok(ClientDownload {
        request: ArtifactRequest {
            logical_name: format!("library {}", library.name),
            url,
            expected_hash,
            expected_size: None,
        },
        target: format!("libraries/{path}"),
        owner: ArtifactOwner::Minecraft,
        native_extract: native.then(|| NativeExtract {
            excludes: library.extract.clone().unwrap_or_default().exclude,
        }),
    })
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

fn validate_asset_logical_path(path: &str) -> Result<(), LauncherError> {
    if path.is_empty()
        || path.contains('\\')
        || path.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "asset logical path '{path}' is unsafe"
        )));
    }
    Ok(())
}

fn validate_portable_target(path: &str) -> Result<(), LauncherError> {
    validate_asset_logical_path(path)
}

fn validate_sha1(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.len() != 40
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "{subject} SHA-1 '{value}' is invalid"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ClientVersionJson {
    #[serde(rename = "mainClass")]
    main_class: String,
    downloads: ClientDownloads,
    #[serde(default)]
    libraries: Vec<Library>,
    #[serde(rename = "assetIndex")]
    asset_index: Option<AssetIndexInfo>,
    arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    #[serde(rename = "javaVersion")]
    java_version: Option<JavaVersion>,
    logging: Option<Logging>,
}

#[derive(Debug, Deserialize)]
struct ClientDownloads {
    client: Option<Download>,
}

#[derive(Debug, Clone, Deserialize)]
struct Download {
    #[serde(default)]
    id: String,
    #[serde(default)]
    path: String,
    sha1: String,
    size: u64,
    url: String,
}

impl Download {
    fn validate(&self, subject: &str) -> Result<(), LauncherError> {
        validate_sha1(&self.sha1, subject)?;
        if self.size == 0 {
            return Err(LauncherError::InvalidRemoteData(format!(
                "{subject} has zero size"
            )));
        }
        let url = url::Url::parse(&self.url).map_err(|error| {
            LauncherError::InvalidRemoteData(format!("{subject} URL is invalid: {error}"))
        })?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err(LauncherError::InvalidRemoteData(format!(
                "{subject} URL must use HTTPS"
            )));
        }
        Ok(())
    }

    fn request(&self, logical_name: String) -> Result<ArtifactRequest, LauncherError> {
        self.validate(&logical_name)?;
        Ok(ArtifactRequest {
            logical_name,
            url: self.url.clone(),
            expected_hash: ExpectedHash::Sha1(self.sha1.clone()),
            expected_size: Some(self.size),
        })
    }
}

#[derive(Debug, Deserialize)]
struct Library {
    name: String,
    url: Option<String>,
    #[serde(default)]
    checksums: Vec<String>,
    downloads: Option<LibraryDownloads>,
    natives: Option<BTreeMap<String, String>>,
    #[serde(default)]
    rules: Vec<Rule>,
    extract: Option<Extract>,
}

#[derive(Debug, Deserialize)]
struct LibraryDownloads {
    artifact: Option<Download>,
    #[serde(default)]
    classifiers: BTreeMap<String, Download>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Extract {
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AssetIndexInfo {
    id: String,
    #[serde(flatten)]
    download: Download,
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
    objects: BTreeMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    virtual_assets: bool,
    #[serde(default, rename = "map_to_resources")]
    map_to_resources: bool,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct Arguments {
    #[serde(default)]
    game: Vec<Argument>,
    #[serde(default)]
    jvm: Vec<Argument>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Argument {
    Plain(String),
    Conditional {
        rules: Vec<Rule>,
        value: ArgumentValue,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct Rule {
    action: RuleAction,
    os: Option<OsRule>,
    #[serde(default)]
    features: HashMap<String, bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Deserialize)]
struct OsRule {
    name: Option<String>,
    version: Option<String>,
    arch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JavaVersion {
    component: String,
    #[serde(rename = "majorVersion")]
    major_version: u32,
}

#[derive(Debug, Deserialize)]
struct Logging {
    client: Option<LoggingClient>,
}

#[derive(Debug, Deserialize)]
struct LoggingClient {
    file: Download,
    argument: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::artifact_path;
    use crate::platform::{Architecture, OperatingSystem};

    fn platform() -> HostPlatform {
        HostPlatform {
            os: OperatingSystem::Windows,
            architecture: Architecture::X86_64,
            os_version: "10.0".to_string(),
        }
    }

    #[test]
    fn rule_order_uses_the_last_matching_action() {
        let rules: Vec<Rule> = serde_json::from_str(
            r#"[
                {"action":"allow"},
                {"action":"disallow","os":{"name":"windows"}},
                {"action":"allow","os":{"name":"linux"}}
            ]"#,
        )
        .unwrap();
        assert!(!rules_allow(&rules, &platform(), &default_features()).unwrap());
    }

    #[test]
    fn maven_coordinates_produce_standard_paths_without_traversal() {
        assert_eq!(
            artifact_path("com.example:demo:1.0:natives-windows@zip", None).unwrap(),
            "com/example/demo/1.0/demo-1.0-natives-windows.zip"
        );
        assert!(artifact_path("../evil:demo:1.0", None).is_err());
    }

    #[test]
    fn conditional_arguments_keep_array_values_together() {
        let arguments: Vec<Argument> = serde_json::from_str(
            r#"[
                "--plain",
                {"rules":[{"action":"allow","os":{"name":"windows"}}],"value":["--pair","value"]}
            ]"#,
        )
        .unwrap();
        assert_eq!(
            flatten_arguments(&arguments, &platform(), &default_features()).unwrap(),
            ["--plain", "--pair", "value"]
        );
    }

    #[tokio::test]
    #[ignore = "uses live Mojang metadata services"]
    async fn live_modern_client_resolution_preserves_classpath_and_asset_mappings() {
        let client = reqwest::Client::builder().build().unwrap();
        let resolved = resolve_vanilla_client(&MojangClient::new(client), "1.21.1", &platform())
            .await
            .unwrap();
        assert!(
            resolved
                .classpath
                .iter()
                .any(|path| path.ends_with("lwjgl-3.3.3.jar"))
        );
        assert!(resolved.asset_mappings.len() >= 1000);
        assert!(resolved.downloads.len() <= resolved.asset_mappings.len() + 100);
    }

    #[tokio::test]
    #[ignore = "uses live Mojang metadata services"]
    async fn live_legacy_client_resolution_extracts_classifier_natives() {
        let client = reqwest::Client::builder().build().unwrap();
        let resolved = resolve_vanilla_client(&MojangClient::new(client), "1.7.10", &platform())
            .await
            .unwrap();
        assert!(resolved.downloads.iter().any(|download| {
            download.native_extract.is_some()
                && download
                    .target
                    .ends_with("lwjgl-platform-2.9.1-natives-windows.jar")
        }));
        assert!(
            resolved
                .classpath
                .iter()
                .all(|path| { !path.ends_with("lwjgl-platform-2.9.1-natives-windows.jar") })
        );
    }
}
