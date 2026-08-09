//! Shared, side-effect-free schemas for Orbit bundles and Modrinth mrpack.
//!
//! This crate owns archive structure and integrity validation. It deliberately
//! does not download files or mutate an instance. Launcher and Orbit consume
//! disjoint projections of the same validated document.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const BUNDLE_MANIFEST_PATH: &str = "bundle.toml";
pub const BUNDLE_FORMAT_VERSION: u32 = 1;
pub const MRPACK_INDEX_PATH: &str = "modrinth.index.json";
pub const MRPACK_FORMAT_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("archive '{path}' does not exist")]
    MissingArchive { path: String },
    #[error("archive I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid ZIP archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid bundle manifest: {0}")]
    BundleToml(#[from] toml::de::Error),
    #[error("invalid mrpack index: {0}")]
    MrpackJson(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstanceTarget {
    Client,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct RuntimeRequirement {
    pub minecraft: String,
    pub loader: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LauncherContent {
    RuntimeOnly,
    RuntimeAndState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrbitContent {
    Mods,
    ModsAndData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct LauncherSection {
    pub content: LauncherContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct OrbitSection {
    pub content: OrbitContent,
    pub manifest: String,
    pub lock: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_manifest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleFileOwner {
    Launcher,
    Orbit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct BundleFile {
    pub path: String,
    pub owner: BundleFileOwner,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct BundleManifest {
    pub format_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub targets: Vec<InstanceTarget>,
    pub runtime: RuntimeRequirement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<LauncherSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit: Option<OrbitSection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<BundleFile>,
}

impl BundleManifest {
    pub fn validate(&self) -> Result<(), FormatError> {
        if self.format_version != BUNDLE_FORMAT_VERSION {
            return invalid(format!(
                "unsupported Orbit bundle format version {}",
                self.format_version
            ));
        }
        if self.launcher.is_none() && self.orbit.is_none() {
            return invalid("Orbit bundle contains neither a Launcher nor an Orbit section");
        }
        if self.id.trim().is_empty()
            || self.name.trim().is_empty()
            || self.version.trim().is_empty()
            || self.runtime.minecraft.trim().is_empty()
            || self.runtime.loader.trim().is_empty()
            || self.targets.is_empty()
        {
            return invalid("Orbit bundle has an empty required identity or runtime field");
        }
        if self.runtime.loader == "vanilla" && self.runtime.loader_version.is_some() {
            return invalid("vanilla runtime must not declare a Loader version");
        }
        if self.runtime.loader != "vanilla"
            && self
                .runtime
                .loader_version
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return invalid("modded runtime must declare an exact Loader version");
        }
        let targets = self.targets.iter().copied().collect::<BTreeSet<_>>();
        if targets.len() != self.targets.len() {
            return invalid("Orbit bundle contains duplicate targets");
        }
        let mut paths = BTreeSet::new();
        for file in &self.files {
            validate_relative_path(&file.path)?;
            if !paths.insert(file.path.as_str()) {
                return invalid(format!(
                    "Orbit bundle contains duplicate path '{}'",
                    file.path
                ));
            }
            let expected_prefix = match file.owner {
                BundleFileOwner::Launcher => "launcher/",
                BundleFileOwner::Orbit => "orbit/",
            };
            if !file.path.starts_with(expected_prefix) {
                return invalid(format!(
                    "bundle file '{}' is outside its {:?} namespace",
                    file.path, file.owner
                ));
            }
            validate_sha256(&file.sha256, &file.path)?;
        }
        if let Some(orbit) = &self.orbit {
            for required in [&orbit.manifest, &orbit.lock] {
                if !paths.contains(required.as_str()) {
                    return invalid(format!(
                        "Orbit section is missing declared file '{required}'"
                    ));
                }
            }
            match orbit.content {
                OrbitContent::Mods => {
                    if orbit.ownership.is_some() || orbit.data_manifest.is_some() {
                        return invalid("mods-only Orbit section must not declare data metadata");
                    }
                }
                OrbitContent::ModsAndData => {
                    for required in [orbit.ownership.as_ref(), orbit.data_manifest.as_ref()] {
                        let Some(required) = required else {
                            return invalid(
                                "mods-and-data Orbit section must declare ownership and data manifest",
                            );
                        };
                        if !paths.contains(required.as_str()) {
                            return invalid(format!(
                                "Orbit data section is missing declared file '{required}'"
                            ));
                        }
                    }
                }
            }
        }
        if self.launcher.as_ref().is_some_and(|section| {
            section.content == LauncherContent::RuntimeOnly
                && self
                    .files
                    .iter()
                    .any(|file| file.owner == BundleFileOwner::Launcher)
        }) {
            return invalid("runtime-only Launcher section must not contain Launcher files");
        }
        if self.launcher.is_none()
            && self
                .files
                .iter()
                .any(|file| file.owner == BundleFileOwner::Launcher)
        {
            return invalid("bundle has Launcher files without a Launcher section");
        }
        if self.orbit.is_none()
            && self
                .files
                .iter()
                .any(|file| file.owner == BundleFileOwner::Orbit)
        {
            return invalid("bundle has Orbit files without an Orbit section");
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BundleArchive {
    pub source: PathBuf,
    pub manifest: BundleManifest,
}

impl BundleArchive {
    pub fn open(source: &Path) -> Result<Self, FormatError> {
        ensure_file(source)?;
        let mut archive = zip::ZipArchive::new(std::fs::File::open(source)?)?;
        let manifest: BundleManifest = read_toml_entry(&mut archive, BUNDLE_MANIFEST_PATH)?;
        manifest.validate()?;
        validate_zip_inventory(&mut archive, &manifest)?;
        Ok(Self {
            source: source.to_path_buf(),
            manifest,
        })
    }

    pub fn verify(&self) -> Result<(), FormatError> {
        self.verify_with_progress(|_, _| {})
    }

    pub fn verify_with_progress<F>(&self, mut progress: F) -> Result<(), FormatError>
    where
        F: FnMut(u64, u64),
    {
        self.verify_matching(|_| true, &mut progress)
    }

    fn verify_matching<P, F>(&self, mut selected: P, mut progress: F) -> Result<(), FormatError>
    where
        P: FnMut(&BundleFile) -> bool,
        F: FnMut(u64, u64),
    {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&self.source)?)?;
        let total = self
            .manifest
            .files
            .iter()
            .filter(|file| selected(file))
            .map(|file| file.size)
            .sum();
        let mut completed = 0_u64;
        let mut buffer = vec![0_u8; 128 * 1024];
        for expected in self.manifest.files.iter().filter(|file| selected(file)) {
            let mut entry = archive.by_name(&expected.path)?;
            if !entry.is_file() || entry.size() != expected.size {
                return invalid(format!(
                    "bundle file '{}' size differs from its manifest",
                    expected.path
                ));
            }
            let mut digest = Sha256::new();
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
                completed = completed.saturating_add(read as u64);
                progress(completed, total);
            }
            let actual = hex::encode(digest.finalize());
            if actual != expected.sha256 {
                return invalid(format!("bundle file '{}' failed SHA-256", expected.path));
            }
        }
        Ok(())
    }

    pub fn extract_owner(
        &self,
        owner: BundleFileOwner,
        destination: &Path,
    ) -> Result<Vec<PathBuf>, FormatError> {
        self.extract_owner_with_progress(owner, destination, |_, _, _, _| {})
    }

    pub fn extract_owner_with_progress<F>(
        &self,
        owner: BundleFileOwner,
        destination: &Path,
        mut progress: F,
    ) -> Result<Vec<PathBuf>, FormatError>
    where
        F: FnMut(u64, u64, usize, usize),
    {
        let selected = self
            .manifest
            .files
            .iter()
            .filter(|file| file.owner == owner)
            .collect::<Vec<_>>();
        let verification_bytes = selected.iter().map(|file| file.size).sum::<u64>();
        let extraction_bytes = selected.iter().map(|file| file.size).sum::<u64>();
        let total_bytes = verification_bytes.saturating_add(extraction_bytes);
        let total_files = selected.len();
        self.verify_matching(
            |file| file.owner == owner,
            |completed, _| {
                progress(completed, total_bytes, 0, total_files);
            },
        )?;
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&self.source)?)?;
        let mut extracted = Vec::new();
        let mut copied_bytes = 0_u64;
        let mut buffer = vec![0_u8; 128 * 1024];
        for (index, file) in selected.into_iter().enumerate() {
            let relative = Path::new(&file.path)
                .strip_prefix(match owner {
                    BundleFileOwner::Launcher => "launcher",
                    BundleFileOwner::Orbit => "orbit",
                })
                .map_err(|_| FormatError::Invalid("invalid bundle namespace".to_string()))?;
            let target = destination.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut entry = archive.by_name(&file.path)?;
            let mut output = std::fs::File::create(&target)?;
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                use std::io::Write as _;
                output.write_all(&buffer[..read])?;
                copied_bytes = copied_bytes.saturating_add(read as u64);
                progress(
                    verification_bytes.saturating_add(copied_bytes),
                    total_bytes,
                    index,
                    total_files,
                );
            }
            output.sync_all()?;
            extracted.push(relative.to_path_buf());
            progress(
                verification_bytes.saturating_add(copied_bytes),
                total_bytes,
                index + 1,
                total_files,
            );
        }
        Ok(extracted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MrpackSideRequirement {
    Required,
    Optional,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MrpackEnvironment {
    pub client: MrpackSideRequirement,
    pub server: MrpackSideRequirement,
}

impl Default for MrpackEnvironment {
    fn default() -> Self {
        Self {
            client: MrpackSideRequirement::Required,
            server: MrpackSideRequirement::Required,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MrpackHashes {
    pub sha1: String,
    pub sha512: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    pub path: String,
    pub hashes: MrpackHashes,
    #[serde(default)]
    pub env: MrpackEnvironment,
    pub downloads: Vec<String>,
    pub file_size: u64,
}

impl MrpackFile {
    pub fn requirement(&self, target: InstanceTarget) -> MrpackSideRequirement {
        match target {
            InstanceTarget::Client => self.env.client,
            InstanceTarget::Server => self.env.server,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub files: Vec<MrpackFile>,
    pub dependencies: BTreeMap<String, String>,
}

impl MrpackIndex {
    pub fn validate(&self) -> Result<(), FormatError> {
        if self.format_version != MRPACK_FORMAT_VERSION || self.game != "minecraft" {
            return invalid(format!(
                "unsupported mrpack formatVersion/game combination {}/{}",
                self.format_version, self.game
            ));
        }
        if self.version_id.is_empty() || self.name.is_empty() {
            return invalid("mrpack has an empty versionId or name");
        }
        let mut paths = BTreeSet::new();
        for file in &self.files {
            validate_relative_path(&file.path)?;
            if !paths.insert(file.path.as_str()) {
                return invalid(format!(
                    "mrpack contains duplicate indexed path '{}'",
                    file.path
                ));
            }
            validate_hex_hash(&file.hashes.sha1, 40, "SHA-1", &file.path)?;
            validate_hex_hash(&file.hashes.sha512, 128, "SHA-512", &file.path)?;
            if file.downloads.is_empty() {
                return invalid(format!("mrpack file '{}' has no download URL", file.path));
            }
            for download in &file.downloads {
                let url = url::Url::parse(download).map_err(|_| {
                    FormatError::Invalid(format!(
                        "mrpack file '{}' has an invalid download URL",
                        file.path
                    ))
                })?;
                if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
                {
                    return invalid(format!(
                        "mrpack file '{}' has a non-HTTPS or credentialed URL",
                        file.path
                    ));
                }
            }
        }
        self.runtime()?;
        Ok(())
    }

    pub fn runtime(&self) -> Result<RuntimeRequirement, FormatError> {
        let minecraft = self
            .dependencies
            .get("minecraft")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                FormatError::Invalid("mrpack has no Minecraft dependency".to_string())
            })?;
        let known = ["fabric-loader", "quilt-loader", "forge", "neoforge"];
        let loaders = known
            .iter()
            .filter_map(|key| self.dependencies.get(*key).map(|version| (*key, version)))
            .collect::<Vec<_>>();
        if loaders.len() > 1 {
            return invalid("mrpack declares multiple mod loaders");
        }
        if loaders
            .first()
            .is_some_and(|(_, version)| version.is_empty())
        {
            return invalid("mrpack declares an empty mod loader version");
        }
        let (loader, loader_version) = loaders.first().map_or_else(
            || ("vanilla".to_string(), None),
            |(loader, version)| {
                let loader = match *loader {
                    "fabric-loader" => "fabric",
                    "quilt-loader" => "quilt",
                    other => other,
                };
                (loader.to_string(), Some((*version).clone()))
            },
        );
        Ok(RuntimeRequirement {
            minecraft: minecraft.clone(),
            loader,
            loader_version,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrpackOverride {
    pub archive_path: String,
    pub relative_path: String,
    pub layer: InstanceTarget,
    pub common: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct MrpackArchive {
    pub source: PathBuf,
    pub index: MrpackIndex,
    pub overrides: Vec<MrpackOverride>,
}

impl MrpackArchive {
    pub fn open(source: &Path) -> Result<Self, FormatError> {
        ensure_file(source)?;
        let mut archive = zip::ZipArchive::new(std::fs::File::open(source)?)?;
        let index: MrpackIndex = read_json_entry(&mut archive, MRPACK_INDEX_PATH)?;
        index.validate()?;
        let overrides = read_mrpack_overrides(&mut archive)?;
        Ok(Self {
            source: source.to_path_buf(),
            index,
            overrides,
        })
    }

    pub fn runtime(&self) -> Result<RuntimeRequirement, FormatError> {
        self.index.runtime()
    }

    pub fn overrides_for(&self, target: InstanceTarget) -> impl Iterator<Item = &MrpackOverride> {
        self.overrides
            .iter()
            .filter(move |entry| entry.common || entry.layer == target)
    }
}

pub fn validate_relative_path(value: &str) -> Result<(), FormatError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || value
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return invalid(format!("unsafe archive path '{value}'"));
    }
    Ok(())
}

fn read_mrpack_overrides<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<Vec<MrpackOverride>, FormatError> {
    let mut output = Vec::new();
    let mut layered_paths = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name();
        let (prefix, layer, common) = if let Some(relative) = name.strip_prefix("overrides/") {
            (relative, InstanceTarget::Client, true)
        } else if let Some(relative) = name.strip_prefix("client-overrides/") {
            (relative, InstanceTarget::Client, false)
        } else if let Some(relative) = name.strip_prefix("server-overrides/") {
            (relative, InstanceTarget::Server, false)
        } else {
            continue;
        };
        validate_relative_path(prefix)?;
        let key = (common, layer, prefix.to_string());
        if !layered_paths.insert(key) {
            return invalid(format!("mrpack contains duplicate override path '{name}'"));
        }
        output.push(MrpackOverride {
            archive_path: name.to_string(),
            relative_path: prefix.to_string(),
            layer,
            common,
            size: entry.size(),
        });
    }
    output.sort_by_key(|entry| (!entry.common, entry.archive_path.clone()));
    Ok(output)
}

fn validate_zip_inventory<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    manifest: &BundleManifest,
) -> Result<(), FormatError> {
    let declared = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if !entry.is_file() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return invalid("Orbit bundle contains a symbolic link");
        }
        let name = entry.name();
        validate_relative_path(name)?;
        if name == BUNDLE_MANIFEST_PATH {
            continue;
        }
        if !actual.insert(name.to_string()) {
            return invalid(format!("Orbit bundle contains duplicate path '{name}'"));
        }
    }
    if actual.iter().map(String::as_str).collect::<BTreeSet<_>>() != declared {
        return invalid("Orbit bundle ZIP inventory differs from bundle.toml");
    }
    Ok(())
}

fn ensure_file(path: &Path) -> Result<(), FormatError> {
    if !path.is_file() {
        return Err(FormatError::MissingArchive {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

fn read_toml_entry<T: for<'de> Deserialize<'de>, R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<T, FormatError> {
    let content = read_bounded_entry(archive, name)?;
    Ok(toml::from_str(&content)?)
}

fn read_json_entry<T: for<'de> Deserialize<'de>, R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<T, FormatError> {
    let content = read_bounded_entry(archive, name)?;
    Ok(serde_json::from_str(&content)?)
}

fn read_bounded_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, FormatError> {
    let mut entry = archive.by_name(name).map_err(|_| {
        FormatError::Invalid(format!("archive is missing required root file '{name}'"))
    })?;
    if !entry.is_file() || entry.size() > MAX_MANIFEST_BYTES {
        return invalid(format!(
            "archive metadata '{name}' is not a bounded regular file"
        ));
    }
    let mut content = String::new();
    entry.read_to_string(&mut content)?;
    Ok(content)
}

fn validate_sha256(value: &str, path: &str) -> Result<(), FormatError> {
    validate_hex_hash(value, 64, "SHA-256", path)
}

fn validate_hex_hash(
    value: &str,
    length: usize,
    algorithm: &str,
    path: &str,
) -> Result<(), FormatError> {
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invalid(format!("file '{path}' has an invalid {algorithm} hash"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, FormatError> {
    Err(FormatError::Invalid(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn launcher_manifest() -> BundleManifest {
        BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            id: "pack".to_string(),
            name: "Pack".to_string(),
            version: "1".to_string(),
            summary: None,
            targets: vec![InstanceTarget::Server],
            runtime: RuntimeRequirement {
                minecraft: "26.2".to_string(),
                loader: "fabric".to_string(),
                loader_version: Some("0.19.2".to_string()),
            },
            launcher: Some(LauncherSection {
                content: LauncherContent::RuntimeOnly,
            }),
            orbit: None,
            files: Vec::new(),
        }
    }

    #[test]
    fn bundle_sections_are_optional_but_not_both_absent() {
        let manifest = BundleManifest {
            launcher: None,
            ..launcher_manifest()
        };
        assert!(manifest.validate().is_err());
        let valid = BundleManifest {
            launcher: Some(LauncherSection {
                content: LauncherContent::RuntimeOnly,
            }),
            ..manifest
        };
        valid.validate().unwrap();
    }

    #[test]
    fn runtime_only_launcher_section_cannot_smuggle_state_files() {
        let mut manifest = launcher_manifest();
        manifest.files.push(BundleFile {
            path: "launcher/state.txt".to_string(),
            owner: BundleFileOwner::Launcher,
            size: 0,
            sha256: "0".repeat(64),
        });
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn bundle_archive_rejects_payload_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let archive_path = directory.path().join("pack.orbitbundle");
        let expected = b"expected";
        let mut manifest = launcher_manifest();
        manifest.launcher = Some(LauncherSection {
            content: LauncherContent::RuntimeAndState,
        });
        manifest.files.push(BundleFile {
            path: "launcher/state.txt".to_string(),
            owner: BundleFileOwner::Launcher,
            size: expected.len() as u64,
            sha256: hex::encode(Sha256::digest(expected)),
        });
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&archive_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("launcher/state.txt", options).unwrap();
        archive.write_all(b"tampered").unwrap();
        archive.start_file(BUNDLE_MANIFEST_PATH, options).unwrap();
        archive
            .write_all(toml::to_string_pretty(&manifest).unwrap().as_bytes())
            .unwrap();
        archive.finish().unwrap();

        let bundle = BundleArchive::open(&archive_path).unwrap();
        assert!(bundle.verify().is_err());
    }

    #[test]
    fn owner_extraction_does_not_read_or_trust_another_projection() {
        let directory = tempfile::tempdir().unwrap();
        let archive_path = directory.path().join("combined.orbitbundle");
        let orbit_manifest = b"manifest";
        let orbit_lock = b"lock";
        let expected_launcher = b"good";
        let actual_launcher = b"evil";
        let mut manifest = launcher_manifest();
        manifest.launcher = Some(LauncherSection {
            content: LauncherContent::RuntimeAndState,
        });
        manifest.orbit = Some(OrbitSection {
            content: OrbitContent::Mods,
            manifest: "orbit/orbit.toml".to_string(),
            lock: "orbit/orbit.lock".to_string(),
            ownership: None,
            data_manifest: None,
        });
        manifest.files = vec![
            BundleFile {
                path: "launcher/state.txt".to_string(),
                owner: BundleFileOwner::Launcher,
                size: expected_launcher.len() as u64,
                sha256: hex::encode(Sha256::digest(expected_launcher)),
            },
            BundleFile {
                path: "orbit/orbit.toml".to_string(),
                owner: BundleFileOwner::Orbit,
                size: orbit_manifest.len() as u64,
                sha256: hex::encode(Sha256::digest(orbit_manifest)),
            },
            BundleFile {
                path: "orbit/orbit.lock".to_string(),
                owner: BundleFileOwner::Orbit,
                size: orbit_lock.len() as u64,
                sha256: hex::encode(Sha256::digest(orbit_lock)),
            },
        ];
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&archive_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (path, content) in [
            ("launcher/state.txt", actual_launcher.as_slice()),
            ("orbit/orbit.toml", orbit_manifest.as_slice()),
            ("orbit/orbit.lock", orbit_lock.as_slice()),
        ] {
            archive.start_file(path, options).unwrap();
            archive.write_all(content).unwrap();
        }
        archive.start_file(BUNDLE_MANIFEST_PATH, options).unwrap();
        archive
            .write_all(toml::to_string_pretty(&manifest).unwrap().as_bytes())
            .unwrap();
        archive.finish().unwrap();

        let bundle = BundleArchive::open(&archive_path).unwrap();
        assert!(bundle.verify().is_err());
        let extracted = directory.path().join("orbit");
        bundle
            .extract_owner(BundleFileOwner::Orbit, &extracted)
            .unwrap();
        assert_eq!(
            std::fs::read(extracted.join("orbit.toml")).unwrap(),
            orbit_manifest
        );
    }

    #[test]
    fn mrpack_runtime_maps_official_loader_keys() {
        let mut index = MrpackIndex {
            format_version: 1,
            game: "minecraft".to_string(),
            version_id: "1".to_string(),
            name: "Pack".to_string(),
            summary: None,
            files: Vec::new(),
            dependencies: BTreeMap::from([
                ("minecraft".to_string(), "1.21.1".to_string()),
                ("fabric-loader".to_string(), "0.16.14".to_string()),
            ]),
        };
        let runtime = index.runtime().unwrap();
        assert_eq!(runtime.loader, "fabric");
        assert_eq!(runtime.loader_version.as_deref(), Some("0.16.14"));

        index
            .dependencies
            .insert("fabric-loader".to_string(), String::new());
        assert!(index.validate().is_err());
    }

    #[test]
    fn paths_are_platform_neutral_and_cannot_escape() {
        assert!(validate_relative_path("mods/example.jar").is_ok());
        for invalid in [
            "../escape",
            "/root",
            "C:/escape",
            "mods\\escape.jar",
            "a/./b",
        ] {
            assert!(validate_relative_path(invalid).is_err(), "{invalid}");
        }
    }
}
