//! Launcher-specific platform discovery used exclusively by `init` and `sync`.
//!
//! This is the deliberately isolated boundary for launcher layouts, profile
//! inheritance, Maven coordinates, and other discovery rules. No normal Orbit
//! operation may call into this module: after initialization it must consume
//! the exact platform snapshot in `orbit.toml`.

use std::path::{Component, Path, PathBuf};

use crate::detection::{Confidence, LoaderDetectionService};
use crate::error::OrbitError;
use crate::manifest::{PlatformArtifact, PlatformSnapshot};
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
    pub physical_environment: crate::metadata::Environment,
}

impl DiscoveredPlatform {
    pub(crate) fn snapshot(&self, instance_dir: &Path) -> Result<PlatformSnapshot, OrbitError> {
        let minecraft_jar = PlatformArtifact::capture(instance_dir, &self.minecraft_jar)?;
        let loader_jar = PlatformArtifact::capture(instance_dir, &self.loader_jar)?;
        let mut runtime_jars = discover_runtime_classpath(instance_dir, self)?
            .into_iter()
            .map(|path| PlatformArtifact::capture(instance_dir, &path))
            .collect::<Result<Vec<_>, _>>()?;
        runtime_jars.retain(|artifact| {
            artifact.sha256 != minecraft_jar.sha256 && artifact.sha256 != loader_jar.sha256
        });
        runtime_jars.sort_by(|left, right| {
            left.sha256
                .cmp(&right.sha256)
                .then_with(|| left.path.cmp(&right.path))
        });
        runtime_jars.dedup_by(|left, right| left.sha256 == right.sha256);
        Ok(PlatformSnapshot {
            minecraft_jar,
            loader_jar,
            runtime_jars,
            physical_environment: self.physical_environment,
        })
    }
}

pub(crate) fn apply_to_manifest(
    manifest: &mut crate::manifest::OrbitManifest,
    discovered: &DiscoveredPlatform,
    artifacts: PlatformSnapshot,
) -> bool {
    let changed = manifest.project.mc_version != discovered.minecraft_version.id
        || manifest.project.modloader != discovered.loader
        || manifest.project.modloader_version != discovered.loader_version
        || manifest.platform != artifacts;
    manifest.project.mc_version = discovered.minecraft_version.id.clone();
    manifest.project.modloader = discovered.loader.clone();
    manifest.project.modloader_version = discovered.loader_version.clone();
    manifest.platform = artifacts;
    changed
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

/// Performs the initial platform scan using the values selected by `init`.
///
/// The selected values only disambiguate launcher-owned candidates. Artifact
/// paths and metadata are still read from the current instance.
pub(crate) fn discover_platform_for_init(
    instance_dir: &Path,
    mc_version: &str,
    loader: &str,
    loader_version: &str,
) -> Result<DiscoveredPlatform, OrbitError> {
    discover_platform(
        instance_dir,
        Some(mc_version),
        Some(loader),
        Some(loader_version),
    )
}

/// Loader evidence exposed to the `init` user interface without leaking the
/// launcher-specific detector API into the rest of Orbit.
#[derive(Debug, Clone)]
pub struct InitLoaderCandidate {
    pub loader: String,
    pub name: String,
    pub versions: Vec<String>,
    pub evidence: Vec<String>,
    pub certain: bool,
}

pub fn detect_loader_candidates(
    instance_dir: &Path,
    minecraft_version: &str,
    requested_loader: Option<&str>,
) -> Result<Vec<InitLoaderCandidate>, OrbitError> {
    let service = LoaderDetectionService::new();
    let detected = if let Some(loader) = requested_loader {
        let detector = service.find_by_name(loader).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "unknown modloader '{loader}'. Supported: {}",
                service
                    .known_loaders()
                    .into_iter()
                    .map(|(loader, _)| loader.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        vec![detector.detect(instance_dir, Some(minecraft_version))?]
    } else {
        service.detect_all(instance_dir, Some(minecraft_version))?
    };
    let names = service
        .known_loaders()
        .into_iter()
        .map(|(loader, name)| (loader.as_str().to_string(), name.to_string()))
        .collect::<std::collections::HashMap<_, _>>();
    detected
        .into_iter()
        .map(|candidate| {
            let loader = candidate.loader.as_str().to_string();
            let name = names.get(&loader).cloned().ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "loader detector returned unregistered loader '{loader}'"
                ))
            })?;
            Ok(InitLoaderCandidate {
                name,
                loader,
                versions: candidate.versions,
                evidence: candidate.evidence,
                certain: candidate.confidence >= Confidence::Certain,
            })
        })
        .collect()
}

pub fn known_loader_choices() -> Vec<(String, String)> {
    LoaderDetectionService::new()
        .known_loaders()
        .into_iter()
        .map(|(loader, name)| (loader.as_str().to_string(), name.to_string()))
        .collect()
}

/// Detects the single unambiguous Minecraft version for `orbit init`.
pub fn detect_mc_version(instance_dir: &Path) -> Result<McVersion, OrbitError> {
    let versions = detect_mc_versions(instance_dir)?;
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => Err(OrbitError::Other(anyhow::anyhow!(
            "no Minecraft client JAR with version.json was found for '{}'",
            instance_dir.display()
        ))),
        versions => Err(OrbitError::Other(anyhow::anyhow!(
            "multiple Minecraft versions are available for '{}': {}; pass --mc-version",
            instance_dir.display(),
            versions
                .iter()
                .map(|version| version.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Returns every actual Minecraft client version visible to this instance.
pub fn detect_mc_versions(instance_dir: &Path) -> Result<Vec<McVersion>, OrbitError> {
    let layout = crate::launcher::LauncherLayout::discover(instance_dir)?;
    let configured_versions = layout.configured_minecraft_versions();
    let expected_version =
        (configured_versions.len() == 1).then(|| configured_versions[0].as_str());
    let mut jar_paths = Vec::new();
    for directory in &layout.game_jar_directories {
        collect_direct_jars(directory, &mut jar_paths)?;
    }
    for library_root in &layout.library_roots {
        let minecraft_root = library_root.join("com").join("mojang").join("minecraft");
        if !minecraft_root.is_dir() {
            continue;
        }
        for version_dir in std::fs::read_dir(&minecraft_root)? {
            let version_dir = version_dir?.path();
            if version_dir.is_dir() {
                collect_direct_jars(&version_dir, &mut jar_paths)?;
            }
        }
    }
    if let Some(version) = expected_version {
        for profile_path in &layout.profile_paths {
            if let Some(versions_root) =
                profile_path.parent().and_then(Path::parent).filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case("versions"))
                })
            {
                collect_direct_jars(&versions_root.join(version), &mut jar_paths)?;
            }
        }
    }
    jar_paths.sort();
    jar_paths.dedup();

    let mut versions = Vec::new();
    for path in jar_paths {
        let Ok(version) = crate::jar::read_minecraft_version(&path) else {
            continue;
        };
        if expected_version.is_some_and(|expected| expected != version.id) {
            continue;
        }
        if !versions
            .iter()
            .any(|existing: &McVersion| existing.id == version.id)
        {
            versions.push(version);
        }
    }
    versions.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(versions)
}

/// Re-discovers the current platform without consulting manifest snapshots.
///
/// This is the only entry point reconciliation commands should use. In
/// particular, it deliberately accepts no old version or path values, so a
/// launcher may rename, move, replace, or upgrade either platform JAR between
/// invocations.
pub(crate) fn rediscover_current_platform(
    instance_dir: &Path,
) -> Result<DiscoveredPlatform, OrbitError> {
    discover_platform(instance_dir, None, None, None)
}

/// Resolves the launcher-declared runtime libraries that accompany the
/// selected platform. Only concrete, existing JARs are returned; no Maven
/// download or provider metadata is consulted.
fn discover_runtime_classpath(
    instance_dir: &Path,
    discovered: &DiscoveredPlatform,
) -> Result<Vec<PathBuf>, OrbitError> {
    let layout = crate::launcher::LauncherLayout::discover(instance_dir)?;
    let mut coordinates = Vec::new();
    for profile_path in &layout.profile_paths {
        let Ok(profile) = VersionProfile::from_path(profile_path) else {
            continue;
        };
        if !profile_matches_runtime(
            &profile,
            &discovered.loader,
            &discovered.minecraft_version.id,
        ) {
            continue;
        }
        coordinates.extend(
            profile
                .libraries
                .iter()
                .filter_map(|library| MavenCoord::parse(&library.name)),
        );
    }
    coordinates.extend(multimc_patch_coordinates(instance_dir)?);
    coordinates.extend(multimc_cached_component_coordinates(
        instance_dir,
        &layout.components,
    )?);

    // Some component metadata only records the selected component, while its
    // patched dependency list lives in a launcher cache outside the instance.
    // The component artifact itself is still exact and can be resolved from
    // the instance's declared library roots.
    for component in &layout.components {
        let coordinate = match component.uid.as_str() {
            "net.fabricmc.fabric-loader" => Some(MavenCoord {
                group_id: "net.fabricmc".to_string(),
                artifact_id: "fabric-loader".to_string(),
                version: component.version.clone(),
                classifier: None,
            }),
            "org.quiltmc.quilt-loader" => Some(MavenCoord {
                group_id: "org.quiltmc".to_string(),
                artifact_id: "quilt-loader".to_string(),
                version: component.version.clone(),
                classifier: None,
            }),
            "net.minecraftforge" => Some(MavenCoord {
                group_id: "net.minecraftforge".to_string(),
                artifact_id: "forge".to_string(),
                version: component.version.clone(),
                classifier: None,
            }),
            "net.neoforged" => Some(MavenCoord {
                group_id: "net.neoforged".to_string(),
                artifact_id: "neoforge".to_string(),
                version: component.version.clone(),
                classifier: None,
            }),
            _ => None,
        };
        if let Some(coordinate) = coordinate {
            coordinates.push(coordinate);
        }
    }

    let mut jars = Vec::new();
    for coordinate in coordinates {
        let relative_dir = maven_directory(&coordinate);
        for root in &layout.library_roots {
            let exact = root.join(&relative_dir).join(maven_filename(&coordinate));
            if exact.is_file() {
                jars.push(exact);
            }
        }
    }
    jars.retain(|path| path != &discovered.minecraft_jar && path != &discovered.loader_jar);
    jars.sort();
    jars.dedup();
    Ok(jars)
}

fn multimc_patch_coordinates(instance_dir: &Path) -> Result<Vec<MavenCoord>, OrbitError> {
    let Some(instance_root) = instance_dir.parent() else {
        return Ok(Vec::new());
    };
    let patches = instance_root.join("patches");
    if !patches.is_dir() {
        return Ok(Vec::new());
    }
    let mut coordinates = Vec::new();
    for entry in std::fs::read_dir(patches)? {
        let path = entry?.path();
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path)?) else {
            continue;
        };
        for library in value
            .get("libraries")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(name) = library.get("name").and_then(serde_json::Value::as_str)
                && let Some(coordinate) = MavenCoord::parse(name)
            {
                coordinates.push(coordinate);
            }
        }
    }
    Ok(coordinates)
}

fn multimc_cached_component_coordinates(
    instance_dir: &Path,
    components: &[crate::launcher::LauncherComponent],
) -> Result<Vec<MavenCoord>, OrbitError> {
    let Some(instance_root) = instance_dir.parent() else {
        return Ok(Vec::new());
    };
    let Some(instances_root) = instance_root.parent() else {
        return Ok(Vec::new());
    };
    let Some(launcher_root) = instances_root.parent() else {
        return Ok(Vec::new());
    };
    let meta_root = launcher_root.join("meta");
    if !meta_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut coordinates = Vec::new();
    for component in components {
        let candidates = [
            meta_root
                .join(&component.uid)
                .join(format!("{}.json", component.version)),
            meta_root.join(format!("{}.json", component.uid)),
        ];
        for path in candidates.into_iter().filter(|path| path.is_file()) {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&std::fs::read(path)?)
            else {
                continue;
            };
            coordinates.extend(coordinates_from_json(&value));
        }
    }
    Ok(coordinates)
}

fn coordinates_from_json(value: &serde_json::Value) -> Vec<MavenCoord> {
    value
        .get("libraries")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|library| {
            library
                .get("name")
                .and_then(serde_json::Value::as_str)
                .and_then(MavenCoord::parse)
        })
        .collect()
}

fn discover_platform(
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

    let loader_package = match crate::jar::read_mod_metadata_if_present(&loader_jar, &loader) {
        Ok(Some(metadata)) => {
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
        Ok(None) if matches!(loader.as_str(), "fabric" | "quilt") => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "no {loader} loader metadata found in '{}'",
                loader_jar.display()
            )));
        }
        Ok(None) => None,
        Err(error) => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "cannot parse {loader} loader JAR '{}': {error}",
                loader_jar.display()
            )));
        }
    };
    let actual_loader_version = loader_package
        .as_ref()
        .map(|package| package.version.clone())
        .unwrap_or(loader_version);
    let physical_environment = physical_environment(&layout, &loader, &minecraft_version.id);

    Ok(DiscoveredPlatform {
        minecraft_version,
        minecraft_jar,
        loader,
        loader_version: actual_loader_version,
        loader_jar,
        loader_package,
        physical_environment,
    })
}

fn physical_environment(
    layout: &crate::launcher::LauncherLayout,
    loader: &str,
    minecraft_version: &str,
) -> crate::metadata::Environment {
    use crate::launcher::LauncherLayoutKind;
    use crate::metadata::Environment;

    if layout.kind == LauncherLayoutKind::DedicatedServer
        || layout.game_jar_directories.iter().any(|directory| {
            directory.join("server.properties").is_file() || directory.join("eula.txt").is_file()
        })
    {
        return Environment::Server;
    }

    let mut saw_client = false;
    let mut saw_server = false;
    for profile_path in &layout.profile_paths {
        let Ok(profile) = VersionProfile::from_path(profile_path) else {
            continue;
        };
        if !profile_matches_runtime(&profile, loader, minecraft_version) {
            continue;
        }
        let main_class = profile
            .main_class
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        saw_client |= main_class.contains("client");
        saw_server |= main_class.contains("server");
    }
    match (saw_client, saw_server) {
        (false, true) => Environment::Server,
        (true, false) => Environment::Client,
        // Preserve uncertainty. Consumers must not turn a side-neutral or
        // contradictory launcher profile into a guessed client environment.
        _ => Environment::Both,
    }
}

fn profile_matches_runtime(
    profile: &VersionProfile,
    loader: &str,
    minecraft_version: &str,
) -> bool {
    profile.id == minecraft_version
        || profile.inherits_from.as_deref() == Some(minecraft_version)
        || profile.libraries.iter().any(|library| {
            MavenCoord::parse(&library.name).is_some_and(|coordinate| {
                loader_signature(loader).is_some_and(|(group, artifacts)| {
                    coordinate.group_id == group
                        && artifacts.contains(&coordinate.artifact_id.as_str())
                })
            })
        })
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
            crate::jar::read_minecraft_version(&path)
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
    let metadata_matches = jars
        .iter()
        .filter(|path| {
            crate::jar::read_mod_metadata(path, loader).is_ok_and(|metadata| {
                metadata.mod_id == expected_mod_id
                    && crate::versions::Version::parse(&metadata.version, loader)
                        == crate::versions::Version::parse(version, loader)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(path) = one_candidate(loader, version, "metadata-matching", &metadata_matches)? {
        return Ok(Some(path));
    }
    if let Some(path) = one_candidate(loader, version, "coordinate-exact", &preferred)? {
        return Ok(Some(path));
    }
    let universal = jars
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-universal.jar"))
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(path) = one_candidate(loader, version, "universal", &universal)? {
        return Ok(Some(path));
    }
    one_candidate(loader, version, "library", &jars)
}

fn one_candidate(
    loader: &str,
    version: &str,
    kind: &str,
    candidates: &[PathBuf],
) -> Result<Option<PathBuf>, OrbitError> {
    match candidates {
        [] => Ok(None),
        [candidate] => Ok(Some(candidate.clone())),
        candidates => Err(OrbitError::Other(anyhow::anyhow!(
            "multiple {kind} JARs match {loader} loader version '{version}': {}",
            candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
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
            r#"{"id":"fabric-test","inheritsFrom":"1.21.1","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","libraries":[{"name":"net.fabricmc:fabric-loader:0.19.2"},{"name":"net.fabricmc:sponge-mixin:0.16.3+mixin.0.8.7"}]}"#,
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
        let mixin_dir = root
            .join("libraries")
            .join("net")
            .join("fabricmc")
            .join("sponge-mixin")
            .join("0.16.3+mixin.0.8.7");
        std::fs::create_dir_all(&mixin_dir).unwrap();
        let mixin_jar = mixin_dir.join("sponge-mixin-0.16.3+mixin.0.8.7.jar");
        std::fs::write(
            &mixin_jar,
            jar_bytes(&[("org/spongepowered/asm/mixin/Mixin.class", b"class")]),
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
            discover_platform_for_init(&version_dir, "1.21.1", "fabric", "0.19.2").unwrap();

        assert_eq!(platform.minecraft_version.id, "1.21.1");
        assert_eq!(platform.loader_version, "0.19.2");
        assert_eq!(
            platform.loader_package.as_ref().unwrap().bundled[0].mod_id,
            "mixinextras"
        );
        assert_eq!(
            discover_runtime_classpath(&version_dir, &platform).unwrap(),
            vec![mixin_jar]
        );
        let snapshot = platform.snapshot(&version_dir).unwrap();
        assert!(snapshot.minecraft_jar.path.contains("../1.21.1/"));
        assert!(snapshot.loader_jar.path.contains("../../libraries/"));
        assert_eq!(snapshot.runtime_jars.len(), 1);
        assert!(
            snapshot.runtime_jars[0]
                .path
                .contains("sponge-mixin/0.16.3+mixin.0.8.7")
        );
        assert_eq!(
            snapshot.physical_environment,
            crate::metadata::Environment::Client
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_prism_cached_component_runtime_libraries() {
        let directory = tempfile::tempdir().unwrap();
        let game = directory
            .path()
            .join("instances")
            .join("example")
            .join(".minecraft");
        let metadata = directory
            .path()
            .join("meta")
            .join("net.fabricmc.fabric-loader");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::write(
            metadata.join("0.19.2.json"),
            r#"{"libraries":[{"name":"net.fabricmc:sponge-mixin:0.16.3+mixin.0.8.7"}]}"#,
        )
        .unwrap();

        let coordinates = multimc_cached_component_coordinates(
            &game,
            &[crate::launcher::LauncherComponent {
                uid: "net.fabricmc.fabric-loader".to_string(),
                version: "0.19.2".to_string(),
            }],
        )
        .unwrap();

        assert_eq!(coordinates.len(), 1);
        assert_eq!(coordinates[0].artifact_id, "sponge-mixin");
    }

    #[test]
    fn server_markers_override_a_side_neutral_launcher_main_class() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("server.properties"),
            b"online-mode=true",
        )
        .unwrap();
        let profile = directory.path().join("server.json");
        std::fs::write(
            &profile,
            r#"{"id":"server","inheritsFrom":"1.21.1","mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher","libraries":[{"name":"net.minecraftforge:forge:1.21.1-52.0.1"}]}"#,
        )
        .unwrap();
        let layout = crate::launcher::LauncherLayout {
            kind: crate::launcher::LauncherLayoutKind::Standalone,
            profile_paths: vec![profile],
            game_jar_directories: vec![directory.path().to_path_buf()],
            library_roots: Vec::new(),
            components: Vec::new(),
        };

        assert_eq!(
            physical_environment(&layout, "forge", "1.21.1"),
            crate::metadata::Environment::Server
        );
    }

    #[test]
    fn side_neutral_launcher_metadata_remains_unknown() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("client.json");
        std::fs::write(
            &profile,
            r#"{"id":"client","inheritsFrom":"1.21.1","mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher","libraries":[{"name":"net.minecraftforge:forge:1.21.1-52.0.1"}]}"#,
        )
        .unwrap();
        let layout = crate::launcher::LauncherLayout {
            kind: crate::launcher::LauncherLayoutKind::Standalone,
            profile_paths: vec![profile],
            game_jar_directories: vec![directory.path().to_path_buf()],
            library_roots: Vec::new(),
            components: Vec::new(),
        };

        assert_eq!(
            physical_environment(&layout, "forge", "1.21.1"),
            crate::metadata::Environment::Both
        );
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
                r#"{{"id":"orbit-test","inheritsFrom":"{mc_version}","mainClass":"net.minecraft.client.main.Main","libraries":[{{"name":"{group}:{artifact}:{coordinate_version}"}}]}}"#
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
