//! Discovers the concrete Minecraft and loader artifacts of one game instance.
//!
//! `orbit.toml` records the last observed paths, but discovery never starts
//! from those paths: launchers can replace or rename profiles and JARs. Every
//! command first reads the launcher layout and actual JAR metadata.

use std::path::{Component, Path, PathBuf};

use crate::detection::{Confidence, LoaderDetectionService};
use crate::error::OrbitError;
use crate::manifest::{PlatformArtifact, PlatformArtifacts};
use crate::metadata::mojang::McVersion;
use crate::metadata::version_profile::{MavenCoord, VersionProfile};
use crate::resolver::types::PlatformCandidate;

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredPlatform {
    pub minecraft_version: McVersion,
    pub minecraft_jar: PathBuf,
    pub loader: String,
    pub loader_version: String,
    pub loader_jar: PathBuf,
    pub loader_package: Option<PlatformCandidate>,
}

impl DiscoveredPlatform {
    pub(crate) fn artifacts(&self, instance_dir: &Path) -> Result<PlatformArtifacts, OrbitError> {
        Ok(PlatformArtifacts {
            minecraft_jar: PlatformArtifact::capture(instance_dir, &self.minecraft_jar)?,
            loader_jar: PlatformArtifact::capture(instance_dir, &self.loader_jar)?,
        })
    }
}

pub(crate) fn apply_to_manifest(
    instance_dir: &Path,
    manifest: &mut crate::manifest::OrbitManifest,
    discovered: &DiscoveredPlatform,
) -> Result<bool, OrbitError> {
    let artifacts = discovered.artifacts(instance_dir)?;
    let changed = manifest.project.mc_version != discovered.minecraft_version.id
        || manifest.project.modloader != discovered.loader
        || manifest.project.modloader_version != discovered.loader_version
        || manifest.platform != artifacts;
    manifest.project.mc_version = discovered.minecraft_version.id.clone();
    manifest.project.modloader = discovered.loader.clone();
    manifest.project.modloader_version = discovered.loader_version.clone();
    manifest.platform = artifacts;
    Ok(changed)
}

impl PlatformArtifact {
    fn capture(instance_dir: &Path, artifact_path: &Path) -> Result<Self, OrbitError> {
        let instance_dir = instance_dir.canonicalize().map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "cannot resolve instance directory '{}': {error}",
                instance_dir.display()
            ))
        })?;
        let artifact_path = artifact_path.canonicalize().map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "cannot resolve platform artifact '{}': {error}",
                artifact_path.display()
            ))
        })?;
        Ok(Self {
            path: portable_path(&relative_or_absolute(&instance_dir, &artifact_path)),
            sha256: crate::jar::compute_sha256(&artifact_path)?,
        })
    }
}

/// Performs a fresh platform scan.
///
/// Requested values are selectors used by `init`; callers such as `sync` pass
/// none and receive an error if launcher state is ambiguous.
pub(crate) fn discover_platform(
    instance_dir: &Path,
    requested_mc_version: Option<&str>,
    requested_loader: Option<&str>,
    requested_loader_version: Option<&str>,
) -> Result<DiscoveredPlatform, OrbitError> {
    let layout = crate::launcher::LauncherLayout::discover(instance_dir)?;
    let (minecraft_jar, minecraft_version) = discover_minecraft(&layout, requested_mc_version)?;
    let (loader, loader_version, loader_jar) = discover_loader(
        instance_dir,
        &layout,
        &minecraft_version.id,
        requested_loader,
        requested_loader_version,
    )?;

    let loader_package = match crate::jar::read_mod_metadata(&loader_jar, &loader) {
        Ok(metadata) => {
            let expected_mod_id = loader_mod_id(&loader);
            if metadata.mod_id != expected_mod_id {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "{loader} loader JAR '{}' declares mod_id '{}', expected '{}'",
                    loader_jar.display(),
                    metadata.mod_id,
                    expected_mod_id
                )));
            }
            if crate::versions::Version::parse(&metadata.version, &loader)
                != crate::versions::Version::parse(&loader_version, &loader)
            {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "{loader} loader JAR '{}' declares version '{}', but launcher metadata \
                     selected '{}'",
                    loader_jar.display(),
                    metadata.version,
                    loader_version
                )));
            }
            Some(PlatformCandidate::from_jar_metadata(metadata))
        }
        Err(error) if matches!(loader.as_str(), "fabric" | "quilt") => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "cannot read {loader} loader metadata from '{}': {error}",
                loader_jar.display()
            )));
        }
        Err(_) => None,
    };
    let actual_loader_version = loader_package
        .as_ref()
        .map(|package| package.version.clone())
        .unwrap_or(loader_version);

    Ok(DiscoveredPlatform {
        minecraft_version,
        minecraft_jar,
        loader,
        loader_version: actual_loader_version,
        loader_jar,
        loader_package,
    })
}

/// Hard platform gate for `install`: Minecraft must still be the declared
/// version. Loader changes are intentionally returned as runtime facts and are
/// left to dependency analysis.
pub(crate) fn discover_install_platform(
    instance_dir: &Path,
    declared_mc_version: &str,
) -> Result<DiscoveredPlatform, OrbitError> {
    let discovered = discover_platform(instance_dir, None, None, None)?;
    if discovered.minecraft_version.id != declared_mc_version {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "Minecraft version changed from '{}' in orbit.toml to '{}' in the launcher instance; \
             run 'orbit sync' before installing",
            declared_mc_version,
            discovered.minecraft_version.id
        )));
    }
    Ok(discovered)
}

fn discover_minecraft(
    layout: &crate::launcher::LauncherLayout,
    requested_version: Option<&str>,
) -> Result<(PathBuf, McVersion), OrbitError> {
    let configured = layout.configured_minecraft_versions();
    let selector = requested_version
        .map(ToString::to_string)
        .or_else(|| (configured.len() == 1).then(|| configured[0].clone()));
    if requested_version.is_none() && configured.len() > 1 {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "launcher metadata selects multiple Minecraft versions: {}; \
             initialize a concrete isolated instance or pass --mc-version",
            configured.join(", ")
        )));
    }

    let mut candidates = minecraft_jar_candidates(layout, selector.as_deref())?;
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    candidates.dedup_by(|left, right| left.0 == right.0);
    if let Some(expected) = selector.as_deref() {
        candidates.retain(|(_, version)| version.id == expected);
    }

    let mut versions = candidates
        .iter()
        .map(|(_, version)| version.id.clone())
        .collect::<Vec<_>>();
    versions.sort();
    versions.dedup();
    if versions.len() > 1 {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "multiple Minecraft client versions are visible to this game directory: {}; \
             pass --mc-version during init or use an isolated instance",
            versions.join(", ")
        )));
    }
    candidates.into_iter().next().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "no Minecraft client JAR{} was found in the {:?} launcher layout",
            selector
                .as_deref()
                .map(|version| format!(" for version '{version}'"))
                .unwrap_or_default(),
            layout.kind
        ))
    })
}

fn minecraft_jar_candidates(
    layout: &crate::launcher::LauncherLayout,
    selected_version: Option<&str>,
) -> Result<Vec<(PathBuf, McVersion)>, OrbitError> {
    let mut paths = Vec::new();
    for directory in &layout.game_jar_directories {
        collect_direct_jars(directory, &mut paths)?;
    }

    if let Some(version) = selected_version {
        for profile_path in &layout.profile_paths {
            if let Some(versions_root) = profile_path
                .parent()
                .and_then(Path::parent)
                .filter(|path| file_name_eq(path, "versions"))
            {
                collect_direct_jars(&versions_root.join(version), &mut paths)?;
            }
        }
    }

    for library_root in &layout.library_roots {
        let minecraft_root = library_root.join("com").join("mojang").join("minecraft");
        if let Some(version) = selected_version {
            collect_direct_jars(&minecraft_root.join(version), &mut paths)?;
        } else if minecraft_root.is_dir() {
            for entry in std::fs::read_dir(&minecraft_root)? {
                let directory = entry?.path();
                if directory.is_dir() {
                    collect_direct_jars(&directory, &mut paths)?;
                }
            }
        }
    }

    paths.sort();
    paths.dedup();
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            crate::init::read_version_json_from_jar(&path)
                .ok()
                .map(|version| (path, version))
        })
        .collect())
}

fn discover_loader(
    instance_dir: &Path,
    layout: &crate::launcher::LauncherLayout,
    mc_version: &str,
    requested_loader: Option<&str>,
    requested_version: Option<&str>,
) -> Result<(String, String, PathBuf), OrbitError> {
    let service = LoaderDetectionService::new();
    let detected = if let Some(loader) = requested_loader {
        let detector = service.find_by_name(loader).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!("unsupported modloader '{loader}'"))
        })?;
        vec![detector.detect(instance_dir, Some(mc_version))?]
    } else {
        service
            .detect_all(instance_dir, Some(mc_version))?
            .into_iter()
            .filter(|info| info.confidence >= Confidence::Certain)
            .collect()
    };
    if detected.len() != 1 {
        let names = detected
            .iter()
            .map(|info| info.loader.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(OrbitError::Other(anyhow::anyhow!(
            "could not determine one modloader from current launcher metadata{}",
            if names.is_empty() {
                String::new()
            } else {
                format!("; candidates: {names}")
            }
        )));
    }
    let info = &detected[0];
    let loader = info.loader.as_str().to_string();
    let mut versions = info.versions.clone();
    versions.sort();
    versions.dedup();
    let selected_version = if let Some(requested) = requested_version {
        if !versions.iter().any(|version| {
            normalized_loader_version(&loader, version) == requested || version == requested
        }) {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "launcher metadata does not contain {loader} loader version '{requested}'"
            )));
        }
        requested.to_string()
    } else {
        match versions.as_slice() {
            [version] => normalized_loader_version(&loader, version),
            [] => {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "launcher metadata identifies {loader}, but not its version"
                )));
            }
            versions => {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "multiple {loader} loader versions match Minecraft {mc_version}: {}",
                    versions.join(", ")
                )));
            }
        }
    };

    let jar = find_loader_jar(layout, &loader, &selected_version)?.ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "could not find the {loader} loader JAR for version '{selected_version}' \
             in the launcher's libraries"
        ))
    })?;
    Ok((loader, selected_version, jar))
}

fn find_loader_jar(
    layout: &crate::launcher::LauncherLayout,
    loader: &str,
    version: &str,
) -> Result<Option<PathBuf>, OrbitError> {
    let (group, artifacts) = loader_signature(loader)
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("unsupported modloader '{loader}'")))?;
    let mut coordinates = Vec::new();
    for profile_path in &layout.profile_paths {
        let Ok(profile) = VersionProfile::from_path(profile_path) else {
            continue;
        };
        for library in &profile.libraries {
            let Some(coordinate) = MavenCoord::parse(&library.name) else {
                continue;
            };
            if coordinate.group_id == group
                && artifacts.contains(&coordinate.artifact_id.as_str())
                && coordinate_version_matches(loader, &coordinate.version, version)
            {
                coordinates.push(coordinate);
            }
        }
    }
    if coordinates.is_empty() {
        let artifact = artifacts[0];
        let coordinate_version = version.to_string();
        coordinates.push(MavenCoord {
            group_id: group.to_string(),
            artifact_id: artifact.to_string(),
            version: coordinate_version,
            classifier: None,
        });
    }

    let mut preferred = Vec::new();
    let mut jars = Vec::new();
    for coordinate in coordinates {
        let relative_dir = maven_directory(&coordinate);
        for root in &layout.library_roots {
            let directory = root.join(&relative_dir);
            if !directory.is_dir() {
                continue;
            }
            let exact = directory.join(maven_filename(&coordinate));
            if exact.is_file() {
                preferred.push(exact.clone());
                jars.push(exact);
            }
            collect_direct_jars(&directory, &mut jars)?;
        }
    }
    jars.sort();
    jars.dedup();
    preferred.sort();
    preferred.dedup();

    let expected_mod_id = loader_mod_id(loader);
    Ok(jars
        .iter()
        .find(|path| {
            crate::jar::read_mod_metadata(path, loader).is_ok_and(|metadata| {
                metadata.mod_id == expected_mod_id
                    && crate::versions::Version::parse(&metadata.version, loader)
                        == crate::versions::Version::parse(version, loader)
            })
        })
        .cloned()
        .or_else(|| preferred.into_iter().next())
        .or_else(|| {
            jars.iter()
                .find(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with("-universal.jar"))
                })
                .cloned()
        })
        .or_else(|| jars.into_iter().next()))
}

fn loader_signature(loader: &str) -> Option<(&'static str, &'static [&'static str])> {
    match loader {
        "fabric" => Some(("net.fabricmc", &["fabric-loader"])),
        "quilt" => Some(("org.quiltmc", &["quilt-loader"])),
        "forge" => Some(("net.minecraftforge", &["forge"])),
        "neoforge" => Some(("net.neoforged", &["neoforge", "forge"])),
        _ => None,
    }
}

fn loader_mod_id(loader: &str) -> &'static str {
    match loader {
        "fabric" => "fabricloader",
        "quilt" => "quilt_loader",
        "forge" => "forge",
        "neoforge" => "neoforge",
        _ => "",
    }
}

fn coordinate_version_matches(loader: &str, actual: &str, expected: &str) -> bool {
    actual == expected || normalized_loader_version(loader, actual) == expected
}

fn normalized_loader_version(loader: &str, version: &str) -> String {
    if matches!(loader, "forge" | "neoforge") {
        version
            .split_once('-')
            .filter(|(minecraft, loader_version)| {
                minecraft.contains('.')
                    && minecraft
                        .chars()
                        .all(|character| character.is_ascii_digit() || character == '.')
                    && loader_version
                        .chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_digit())
            })
            .map(|(_, loader_version)| loader_version.to_string())
            .unwrap_or_else(|| version.to_string())
    } else {
        version.to_string()
    }
}

fn maven_directory(coordinate: &MavenCoord) -> PathBuf {
    let mut relative = PathBuf::new();
    for component in coordinate.group_id.split('.') {
        relative.push(component);
    }
    relative.push(&coordinate.artifact_id);
    relative.push(&coordinate.version);
    relative
}

fn maven_filename(coordinate: &MavenCoord) -> String {
    let classifier = coordinate
        .classifier
        .as_deref()
        .map(|classifier| format!("-{classifier}"))
        .unwrap_or_default();
    format!(
        "{}-{}{}.jar",
        coordinate.artifact_id, coordinate.version, classifier
    )
}

fn collect_direct_jars(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), OrbitError> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jar"))
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

fn relative_or_absolute(base: &Path, target: &Path) -> PathBuf {
    let base_components = base.components().collect::<Vec<_>>();
    let target_components = target.components().collect::<Vec<_>>();
    let mut common = 0;
    while common < base_components.len()
        && common < target_components.len()
        && base_components[common] == target_components[common]
    {
        common += 1;
    }
    if common == 0
        || matches!(
            (base_components.first(), target_components.first()),
            (Some(Component::Prefix(_)), Some(Component::Prefix(_)))
        ) && base_components.first() != target_components.first()
    {
        return target.to_path_buf();
    }

    let mut relative = PathBuf::new();
    for component in &base_components[common..] {
        if matches!(component, Component::Normal(_)) {
            relative.push("..");
        }
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    relative
}

fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn discovers_fabric_platform_from_an_isolated_launcher_instance() {
        let root =
            std::env::temp_dir().join(format!("orbit-platform-loader-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        let version_dir = root.join("versions").join("fabric-test");
        let vanilla_dir = root.join("versions").join("1.21.1");
        let library_dir = root
            .join("libraries")
            .join("net")
            .join("fabricmc")
            .join("fabric-loader")
            .join("0.19.2");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::create_dir_all(&vanilla_dir).unwrap();
        std::fs::create_dir_all(&library_dir).unwrap();
        std::fs::write(
            version_dir.join("fabric-test.json"),
            r#"{"id":"fabric-test","inheritsFrom":"1.21.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.19.2"}]}"#,
        )
        .unwrap();
        std::fs::write(
            vanilla_dir.join("1.21.1.jar"),
            jar_bytes(&[("version.json", minecraft_version_json().as_bytes())]),
        )
        .unwrap();

        let nested = jar_bytes(&[(
            "fabric.mod.json",
            br#"{"schemaVersion":1,"id":"mixinextras","version":"0.5.4","name":"MixinExtras"}"#,
        )]);
        let loader_metadata = br#"{
  "schemaVersion": 1,
  "id": "fabricloader",
  "version": "0.19.2",
  "name": "Fabric Loader",
  "jars": [{"file": "META-INF/jars/mixinextras-fabric-0.5.4.jar"}]
}"#;
        std::fs::write(
            library_dir.join("fabric-loader-0.19.2.jar"),
            jar_bytes(&[
                ("fabric.mod.json", loader_metadata),
                ("META-INF/jars/mixinextras-fabric-0.5.4.jar", &nested),
            ]),
        )
        .unwrap();

        assert_eq!(
            crate::init::detect_mc_versions(&version_dir)
                .unwrap()
                .into_iter()
                .map(|version| version.id)
                .collect::<Vec<_>>(),
            vec!["1.21.1"]
        );
        let platform =
            discover_platform(&version_dir, Some("1.21.1"), Some("fabric"), Some("0.19.2"))
                .unwrap();

        assert_eq!(platform.minecraft_version.id, "1.21.1");
        assert_eq!(platform.loader_version, "0.19.2");
        assert_eq!(
            platform.loader_package.as_ref().unwrap().bundled[0].mod_id,
            "mixinextras"
        );
        let artifacts = platform.artifacts(&version_dir).unwrap();
        assert!(artifacts.minecraft_jar.path.contains("../1.21.1/"));
        assert!(artifacts.loader_jar.path.contains("../../libraries/"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn install_accepts_a_loader_version_change_as_runtime_state() {
        let root = std::env::temp_dir().join(format!(
            "orbit-platform-loader-change-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        test_support::write_platform(&root, "1.21.1", "fabric", "0.16.10");
        test_support::write_platform(&root, "1.21.1", "fabric", "0.17.0");

        let platform = discover_install_platform(&root, "1.21.1").unwrap();

        assert_eq!(platform.loader, "fabric");
        assert_eq!(platform.loader_version, "0.17.0");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn install_rejects_a_changed_minecraft_version() {
        let root = std::env::temp_dir().join(format!(
            "orbit-platform-minecraft-change-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        test_support::write_platform(&root, "1.21.1", "fabric", "0.16.10");
        test_support::write_platform(&root, "1.21.2", "fabric", "0.16.10");

        let error = discover_install_platform(&root, "1.21.1")
            .unwrap_err()
            .to_string();

        assert!(error.contains("Minecraft version changed"));
        assert!(error.contains("orbit sync"));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn minecraft_version_json() -> String {
        r#"{
  "id":"1.21.1",
  "name":"1.21.1",
  "world_version":1,
  "protocol_version":1,
  "pack_version":{"resource_major":1,"resource_minor":0,"data_major":1,"data_minor":0},
  "java_version":21,
  "stable":true
}"#
        .to_string()
    }

    fn jar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        for (path, bytes) in entries {
            archive
                .start_file(*path, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    pub(crate) fn write_platform(
        instance_dir: &Path,
        mc_version: &str,
        loader: &str,
        loader_version: &str,
    ) {
        std::fs::create_dir_all(instance_dir).unwrap();
        let (group, artifact, coordinate_version) = match loader {
            "fabric" => ("net.fabricmc", "fabric-loader", loader_version.to_string()),
            "quilt" => ("org.quiltmc", "quilt-loader", loader_version.to_string()),
            "forge" => (
                "net.minecraftforge",
                "forge",
                format!("{mc_version}-{loader_version}"),
            ),
            "neoforge" => ("net.neoforged", "neoforge", loader_version.to_string()),
            other => panic!("unsupported test loader {other}"),
        };
        std::fs::write(
            instance_dir.join("orbit-test.json"),
            format!(
                r#"{{"id":"orbit-test","inheritsFrom":"{mc_version}","libraries":[{{"name":"{group}:{artifact}:{coordinate_version}"}}]}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            instance_dir.join(format!("{mc_version}.jar")),
            jar_bytes(&[(
                "version.json",
                minecraft_version_json(mc_version).as_bytes(),
            )]),
        )
        .unwrap();

        let mut loader_dir = instance_dir.join("libraries");
        for part in group.split('.') {
            loader_dir.push(part);
        }
        loader_dir.push(artifact);
        loader_dir.push(&coordinate_version);
        std::fs::create_dir_all(&loader_dir).unwrap();
        let loader_jar = loader_dir.join(format!("{artifact}-{coordinate_version}.jar"));
        let bytes = match loader {
            "fabric" => jar_bytes(&[(
                "fabric.mod.json",
                format!(
                    r#"{{"schemaVersion":1,"id":"fabricloader","version":"{loader_version}","name":"Fabric Loader"}}"#
                )
                .as_bytes(),
            )]),
            "quilt" => jar_bytes(&[(
                "quilt.mod.json",
                format!(
                    r#"{{"schema_version":1,"quilt_loader":{{"group":"org.quiltmc","id":"quilt_loader","version":"{loader_version}","metadata":{{"name":"Quilt Loader"}}}}}}"#
                )
                .as_bytes(),
            )]),
            _ => jar_bytes(&[("META-INF/orbit-platform-test", b"loader")]),
        };
        std::fs::write(loader_jar, bytes).unwrap();
    }

    fn minecraft_version_json(version: &str) -> String {
        format!(
            r#"{{
  "id":"{version}",
  "name":"{version}",
  "world_version":1,
  "protocol_version":1,
  "pack_version":{{"resource_major":1,"resource_minor":0,"data_major":1,"data_minor":0}},
  "java_version":21,
  "stable":true
}}"#
        )
    }

    fn jar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        for (path, bytes) in entries {
            archive
                .start_file(*path, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }
}
