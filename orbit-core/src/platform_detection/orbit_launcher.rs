//! Strict adapter for client runtimes managed by `orbit-launcher`.
//!
//! This module is only reached by `init` and `sync`. Normal Orbit operations
//! continue to consume the platform snapshot already recorded in `orbit.toml`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use super::{DiscoveredPlatform, build_discovered_platform, find_loader_jar};
use crate::error::OrbitError;
use crate::launcher::{LauncherLayout, LauncherLayoutKind};
use crate::loader::LoaderKind;
use crate::metadata::Environment;

const MANIFEST_FILE: &str = "orbit-launcher.toml";
const LOCK_FILE: &str = "orbit-launcher.lock";
const MANIFEST_SCHEMA: u32 = 1;
const LOCK_SCHEMA: u32 = 5;

pub(super) fn discover(
    instance_dir: &Path,
    requested_minecraft: Option<&str>,
    requested_loader: Option<LoaderKind>,
    requested_loader_version: Option<&str>,
) -> Result<Option<DiscoveredPlatform>, OrbitError> {
    let Some(runtime) = read(instance_dir)? else {
        return Ok(None);
    };
    runtime.validate_requests(
        requested_minecraft,
        requested_loader,
        requested_loader_version,
    )?;
    build_discovered_platform(
        runtime.minecraft,
        runtime.minecraft_jar,
        runtime.loader,
        runtime.loader_version,
        runtime.loader_jar,
        Environment::Client,
        Some(runtime.runtime_jars),
    )
    .map(Some)
}

pub(super) fn minecraft_version(
    instance_dir: &Path,
) -> Result<Option<crate::metadata::mojang::McVersion>, OrbitError> {
    read(instance_dir).map(|runtime| runtime.map(|runtime| runtime.minecraft))
}

struct ManagedRuntime {
    minecraft: crate::metadata::mojang::McVersion,
    minecraft_jar: PathBuf,
    loader: LoaderKind,
    loader_version: String,
    loader_jar: PathBuf,
    runtime_jars: Vec<PathBuf>,
}

impl ManagedRuntime {
    fn validate_requests(
        &self,
        minecraft: Option<&str>,
        loader: Option<LoaderKind>,
        loader_version: Option<&str>,
    ) -> Result<(), OrbitError> {
        if minecraft.is_some_and(|requested| requested != self.minecraft.id) {
            return Err(invalid(format!(
                "lock selects Minecraft '{}', not requested '{}'",
                self.minecraft.id,
                minecraft.expect("checked Some")
            )));
        }
        if loader.is_some_and(|requested| requested != self.loader) {
            return Err(invalid(format!(
                "lock selects loader '{}', not requested '{}'",
                self.loader,
                loader.expect("checked Some")
            )));
        }
        if loader_version.is_some_and(|requested| {
            crate::versions::Version::parse(requested, self.loader)
                != crate::versions::Version::parse(&self.loader_version, self.loader)
        }) {
            return Err(invalid(format!(
                "lock selects {} loader version '{}', not requested '{}'",
                self.loader,
                self.loader_version,
                loader_version.expect("checked Some")
            )));
        }
        Ok(())
    }
}

fn read(instance_dir: &Path) -> Result<Option<ManagedRuntime>, OrbitError> {
    let manifest_path = instance_dir.join(MANIFEST_FILE);
    let lock_path = instance_dir.join(LOCK_FILE);
    match (manifest_path.is_file(), lock_path.is_file()) {
        (false, false) => return Ok(None),
        (true, false) => {
            return Err(invalid(format!(
                "'{}' exists but '{}' is missing; run 'orbit-launcher install'",
                manifest_path.display(),
                lock_path.display()
            )));
        }
        (false, true) => {
            return Err(invalid(format!(
                "'{}' exists but '{}' is missing",
                lock_path.display(),
                manifest_path.display()
            )));
        }
        (true, true) => {}
    }

    let manifest: LauncherManifest = parse_toml(&manifest_path, "launcher manifest")?;
    let lock: LauncherLock = parse_toml(&lock_path, "launcher lock")?;
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(invalid(format!(
            "launcher manifest schema {} is unsupported; expected {MANIFEST_SCHEMA}",
            manifest.schema
        )));
    }
    if lock.schema != LOCK_SCHEMA {
        return Err(invalid(format!(
            "launcher lock schema {} is unsupported; expected {LOCK_SCHEMA}; reinstall the instance",
            lock.schema
        )));
    }
    if manifest.id != lock.instance_id {
        return Err(invalid(
            "orbit-launcher.toml and orbit-launcher.lock identify different instances",
        ));
    }
    if manifest.kind != "client" || lock.kind != "client" {
        return Ok(None);
    }
    if lock.loader.kind == "vanilla" {
        return Err(invalid(
            "the managed client is Vanilla and therefore is not an Orbit mod instance",
        ));
    }
    let loader = lock.loader.kind.parse::<LoaderKind>().map_err(invalid)?;
    let loader_version = lock.loader.version.ok_or_else(|| {
        invalid(format!(
            "managed {} client lock has no Loader version",
            loader.as_str()
        ))
    })?;
    validate_text(&lock.minecraft.version, "Minecraft version")?;
    validate_text(&loader_version, "Loader version")?;

    let versions = instance_dir.parent().ok_or_else(|| {
        invalid(format!(
            "managed client directory '{}' has no versions parent",
            instance_dir.display()
        ))
    })?;
    if !file_name_eq(versions, "versions") {
        return Err(invalid(format!(
            "managed client directory '{}' is not an immediate child of a versions directory",
            instance_dir.display()
        )));
    }
    let repository = versions.parent().ok_or_else(|| {
        invalid(format!(
            "versions directory '{}' has no Minecraft repository parent",
            versions.display()
        ))
    })?;
    let repository = repository.canonicalize().map_err(|error| {
        invalid(format!(
            "cannot resolve Minecraft repository '{}': {error}",
            repository.display()
        ))
    })?;

    let classpath = match lock.entrypoint {
        LockedEntrypoint::Classpath { classpath } if !classpath.is_empty() => classpath,
        LockedEntrypoint::Classpath { .. } => {
            return Err(invalid("managed client classpath is empty"));
        }
        LockedEntrypoint::Other => {
            return Err(invalid(
                "managed client does not use a locked classpath entrypoint",
            ));
        }
    };
    let mut inventory = BTreeMap::new();
    for artifact in lock.artifacts {
        validate_relative_path(&artifact.path)?;
        validate_digest(&artifact.sha256)?;
        if inventory.insert(artifact.path.clone(), artifact).is_some() {
            return Err(invalid(
                "managed client lock contains duplicate artifact paths",
            ));
        }
    }

    let mut classpath_paths = Vec::with_capacity(classpath.len());
    for relative in &classpath {
        validate_relative_path(relative)?;
        let artifact = inventory.get(relative).ok_or_else(|| {
            invalid(format!(
                "classpath entry '{relative}' is absent from the artifact inventory"
            ))
        })?;
        let path = resolve_locked_artifact(&repository, artifact)?;
        classpath_paths.push((relative.clone(), artifact.owner.as_str(), path));
    }

    let mut minecraft_candidates = classpath_paths
        .iter()
        .filter(|(_, owner, _)| *owner == "minecraft")
        .filter_map(|(_, _, path)| {
            crate::jar::read_minecraft_version(path)
                .ok()
                .filter(|version| version.id == lock.minecraft.version)
                .map(|version| (path.clone(), version))
        })
        .collect::<Vec<_>>();
    minecraft_candidates.sort_by(|left, right| left.0.cmp(&right.0));
    minecraft_candidates.dedup_by(|left, right| left.0 == right.0);
    let (minecraft_jar, minecraft) = match minecraft_candidates.as_slice() {
        [(path, version)] => (path.clone(), version.clone()),
        [] => {
            return Err(invalid(format!(
                "locked classpath contains no Minecraft JAR declaring '{}'",
                lock.minecraft.version
            )));
        }
        candidates => {
            return Err(invalid(format!(
                "locked classpath contains multiple Minecraft JARs declaring '{}': {}",
                lock.minecraft.version,
                candidates
                    .iter()
                    .map(|(path, _)| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    };

    let layout = LauncherLayout {
        kind: LauncherLayoutKind::IsolatedVersion,
        profile_paths: Vec::new(),
        game_jar_directories: Vec::new(),
        library_roots: vec![repository.join("libraries")],
        components: Vec::new(),
    };
    let loader_jar = find_loader_jar(&layout, loader, &loader_version)?.ok_or_else(|| {
        invalid(format!(
            "locked repository contains no primary {loader} JAR for version '{loader_version}'"
        ))
    })?;
    let loader_jar = loader_jar.canonicalize().map_err(|error| {
        invalid(format!(
            "cannot resolve locked Loader JAR '{}': {error}",
            loader_jar.display()
        ))
    })?;
    let loader_entry = classpath_paths
        .iter()
        .find(|(_, owner, path)| *owner == "loader" && path == &loader_jar)
        .ok_or_else(|| {
            invalid(format!(
                "primary Loader JAR '{}' is not a Loader-owned classpath entry",
                loader_jar.display()
            ))
        })?;
    let loader_artifact = inventory
        .get(&loader_entry.0)
        .expect("classpath inventory was validated");
    verify_digest(&loader_jar, &loader_artifact.sha256)?;

    let mut runtime_jars = classpath_paths
        .into_iter()
        .map(|(_, _, path)| path)
        .filter(|path| path != &minecraft_jar && path != &loader_jar)
        .collect::<Vec<_>>();
    runtime_jars.sort();
    runtime_jars.dedup();

    Ok(Some(ManagedRuntime {
        minecraft,
        minecraft_jar,
        loader,
        loader_version,
        loader_jar,
        runtime_jars,
    }))
}

fn parse_toml<T: for<'de> Deserialize<'de>>(path: &Path, label: &str) -> Result<T, OrbitError> {
    let content = std::fs::read_to_string(path)?;
    toml::from_str(&content).map_err(|error| {
        invalid(format!(
            "cannot parse {label} '{}': {error}",
            path.display()
        ))
    })
}

fn resolve_locked_artifact(
    repository: &Path,
    artifact: &LockedArtifact,
) -> Result<PathBuf, OrbitError> {
    let path = repository.join(Path::new(&artifact.path));
    let path = path.canonicalize().map_err(|error| {
        invalid(format!(
            "locked artifact '{}' cannot be resolved: {error}",
            artifact.path
        ))
    })?;
    if !path.starts_with(repository) || !path.is_file() {
        return Err(invalid(format!(
            "locked artifact '{}' is outside the Minecraft repository or is not a file",
            artifact.path
        )));
    }
    verify_digest(&path, &artifact.sha256)?;
    Ok(path)
}

fn verify_digest(path: &Path, expected: &str) -> Result<(), OrbitError> {
    let actual = crate::jar::compute_sha256(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(invalid(format!(
            "locked artifact '{}' does not match its SHA-256",
            path.display()
        )));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), OrbitError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(part) if !part.is_empty()))
    {
        return Err(invalid(format!(
            "locked artifact path '{value}' is not a normalized portable relative path"
        )));
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), OrbitError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(format!(
            "locked artifact SHA-256 '{value}' is invalid"
        )));
    }
    Ok(())
}

fn validate_text(value: &str, subject: &str) -> Result<(), OrbitError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(invalid(format!("{subject} '{value}' is invalid")));
    }
    Ok(())
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

fn invalid(message: impl std::fmt::Display) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!("invalid Orbit Launcher runtime: {message}"))
}

#[derive(Deserialize)]
struct LauncherManifest {
    schema: u32,
    id: String,
    kind: String,
}

#[derive(Deserialize)]
struct LauncherLock {
    schema: u32,
    instance_id: String,
    kind: String,
    minecraft: LockedMinecraft,
    loader: LockedLoader,
    entrypoint: LockedEntrypoint,
    artifacts: Vec<LockedArtifact>,
}

#[derive(Deserialize)]
struct LockedMinecraft {
    version: String,
}

#[derive(Deserialize)]
struct LockedLoader {
    kind: String,
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum LockedEntrypoint {
    Classpath {
        classpath: Vec<String>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct LockedArtifact {
    owner: LockedArtifactOwner,
    sha256: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LockedArtifactOwner {
    Minecraft,
    Loader,
    Java,
    AuthlibInjector,
}

impl LockedArtifactOwner {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Minecraft => "minecraft",
            Self::Loader => "loader",
            Self::Java => "java",
            Self::AuthlibInjector => "authlib-injector",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn write_jar(path: &Path, entries: &[(&str, &str)]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let file = std::fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in entries {
            archive.start_file(*name, options).unwrap();
            archive.write_all(body.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn partial_launcher_runtime_is_an_error_instead_of_a_fallback() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join(MANIFEST_FILE),
            "schema = 1\nid = 'f42a6bda-35dc-4ca7-8883-ec74814f0b8f'\nkind = 'client'\n",
        )
        .unwrap();

        let error = minecraft_version(directory.path()).unwrap_err().to_string();
        assert!(error.contains("orbit-launcher.lock"));
        assert!(error.contains("missing"));
    }

    #[test]
    fn locked_paths_reject_parent_traversal() {
        assert!(validate_relative_path("../libraries/escape.jar").is_err());
        assert!(validate_relative_path("libraries/ok.jar").is_ok());
    }

    #[test]
    fn reads_exact_runtime_from_lock_without_a_derived_version_profile() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("minecraft");
        let instance = repository.join("versions/26.2");
        let minecraft_relative = "versions/26.2/26.2.jar";
        let loader_relative =
            "libraries/net/fabricmc/fabric-loader/0.19.3/fabric-loader-0.19.3.jar";
        let minecraft_jar = repository.join(minecraft_relative);
        let loader_jar = repository.join(loader_relative);
        write_jar(
            &minecraft_jar,
            &[(
                "version.json",
                r#"{
                    "id":"26.2",
                    "name":"26.2",
                    "world_version":5000,
                    "protocol_version":800,
                    "pack_version":{
                        "resource_major":80,
                        "resource_minor":0,
                        "data_major":100,
                        "data_minor":0
                    },
                    "java_version":25,
                    "stable":true
                }"#,
            )],
        );
        write_jar(
            &loader_jar,
            &[(
                "fabric.mod.json",
                r#"{"schemaVersion":1,"id":"fabricloader","version":"0.19.3","name":"Fabric Loader"}"#,
            )],
        );
        std::fs::write(
            instance.join(MANIFEST_FILE),
            "schema = 1\nid = 'f42a6bda-35dc-4ca7-8883-ec74814f0b8f'\nname = '26.2'\nkind = 'client'\n",
        )
        .unwrap();
        let minecraft_hash = crate::jar::compute_sha256(&minecraft_jar).unwrap();
        let loader_hash = crate::jar::compute_sha256(&loader_jar).unwrap();
        std::fs::write(
            instance.join(LOCK_FILE),
            format!(
                r#"schema = 5
instance_id = "f42a6bda-35dc-4ca7-8883-ec74814f0b8f"
kind = "client"

[minecraft]
version = "26.2"

[loader]
kind = "fabric"
version = "0.19.3"

[entrypoint]
kind = "classpath"
classpath = ["{loader_relative}", "{minecraft_relative}"]

[[artifacts]]
owner = "loader"
sha256 = "{loader_hash}"
path = "{loader_relative}"

[[artifacts]]
owner = "minecraft"
sha256 = "{minecraft_hash}"
path = "{minecraft_relative}"
"#
            ),
        )
        .unwrap();

        let runtime = discover(
            &instance,
            Some("26.2"),
            Some(LoaderKind::Fabric),
            Some("0.19.3"),
        )
        .unwrap()
        .unwrap();

        assert_eq!(runtime.minecraft_version.id, "26.2");
        assert_eq!(runtime.loader, LoaderKind::Fabric);
        assert_eq!(runtime.loader_version, "0.19.3");
        assert_eq!(runtime.minecraft_jar, minecraft_jar.canonicalize().unwrap());
        assert_eq!(runtime.loader_jar, loader_jar.canonicalize().unwrap());
        assert!(runtime.runtime_jars.as_ref().unwrap().is_empty());
        assert!(!instance.join("26.2.json").exists());
    }
}
