//! Normalizes common launcher directory layouts around one Orbit game instance.
//!
//! The Orbit instance directory is always the directory Minecraft receives as
//! its game directory (the directory containing `mods/`, `config/`, and saves).
//! Launcher metadata and shared libraries may live outside that directory.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::OrbitError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LauncherLayoutKind {
    /// A normal game root whose launcher profiles live under `versions/`.
    SharedGameRoot,
    /// A version-isolated game directory under `<game-root>/versions/<instance>`.
    IsolatedVersion,
    /// A Prism Launcher, MultiMC, or compatible component-based instance.
    MultiMc,
    /// A CurseForge profile game directory.
    CurseForge,
    /// A GDLauncher game directory nested below its instance metadata.
    GdLauncher,
    /// A standalone directory containing its own launcher profile/JAR.
    Standalone,
    /// A dedicated server directory with server-owned runtime markers.
    DedicatedServer,
}

#[derive(Debug, Clone)]
pub(crate) struct LauncherLayout {
    pub kind: LauncherLayoutKind,
    pub profile_paths: Vec<PathBuf>,
    pub game_jar_directories: Vec<PathBuf>,
    pub library_roots: Vec<PathBuf>,
    pub components: Vec<LauncherComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LauncherComponent {
    pub uid: String,
    pub version: String,
}

impl LauncherLayout {
    /// Discovers a supported launcher layout without accepting an empty or
    /// arbitrary directory as a Minecraft game directory.
    pub(crate) fn discover(instance_dir: &Path) -> Result<Self, OrbitError> {
        if !instance_dir.is_dir() {
            return Err(invalid_game_directory(
                instance_dir,
                "the directory does not exist",
            ));
        }

        if let Some(layout) = discover_multimc(instance_dir)? {
            return Ok(layout);
        }
        if let Some(layout) = discover_curseforge(instance_dir)? {
            return Ok(layout);
        }
        if let Some(layout) = discover_gdlauncher(instance_dir)? {
            return Ok(layout);
        }
        if let Some(layout) = discover_isolated_version(instance_dir)? {
            return Ok(layout);
        }
        if let Some(layout) = discover_shared_game_root(instance_dir)? {
            return Ok(layout);
        }
        if let Some(layout) = discover_standalone(instance_dir)? {
            return Ok(layout);
        }
        if is_dedicated_server(instance_dir) {
            return Ok(LauncherLayout {
                kind: LauncherLayoutKind::DedicatedServer,
                profile_paths: Vec::new(),
                game_jar_directories: vec![instance_dir.to_path_buf()],
                library_roots: existing_library_roots(instance_dir, 1),
                components: Vec::new(),
            });
        }

        Err(invalid_game_directory(
            instance_dir,
            "no launcher profile, launcher instance metadata, Minecraft version JAR, or \
             dedicated-server marker was found",
        ))
    }

    pub(crate) fn component_version(&self, uid: &str) -> Option<&str> {
        self.components
            .iter()
            .find(|component| component.uid == uid)
            .map(|component| component.version.as_str())
    }

    /// Minecraft versions selected by launcher-owned metadata. An empty result
    /// means that the launcher marker does not expose this information and the
    /// installed client JARs must be inspected instead.
    pub(crate) fn configured_minecraft_versions(&self) -> Vec<String> {
        let mut versions = self
            .component_version("net.minecraft")
            .map(ToString::to_string)
            .into_iter()
            .collect::<Vec<_>>();
        for path in &self.profile_paths {
            let Ok(profile) = crate::metadata::version_profile::VersionProfile::from_path(path)
            else {
                continue;
            };
            if let Some(version) = profile.inherits_from {
                versions.push(version);
            }
        }
        versions.sort();
        versions.dedup();
        versions
    }
}

fn discover_isolated_version(instance_dir: &Path) -> Result<Option<LauncherLayout>, OrbitError> {
    let Some(versions_dir) = instance_dir.parent() else {
        return Ok(None);
    };
    if !file_name_eq(versions_dir, "versions") {
        return Ok(None);
    }
    let profile_paths = launcher_profile_paths(instance_dir)?;
    if profile_paths.is_empty() && !contains_minecraft_version_jar(instance_dir) {
        return Ok(None);
    }
    let Some(game_root) = versions_dir.parent() else {
        return Ok(None);
    };
    Ok(Some(LauncherLayout {
        kind: LauncherLayoutKind::IsolatedVersion,
        profile_paths,
        game_jar_directories: vec![instance_dir.to_path_buf()],
        library_roots: existing_directories([game_root.join("libraries")]),
        components: Vec::new(),
    }))
}

fn discover_shared_game_root(instance_dir: &Path) -> Result<Option<LauncherLayout>, OrbitError> {
    let versions_dir = instance_dir.join("versions");
    if !versions_dir.is_dir() {
        return Ok(None);
    }
    let mut version_directories = child_directories(&versions_dir)?;
    version_directories.sort();
    let mut profile_paths = launcher_profile_paths(instance_dir)?;
    let mut game_jar_directories = Vec::new();
    for directory in version_directories {
        let profiles = launcher_profile_paths(&directory)?;
        if !profiles.is_empty() || contains_minecraft_version_jar(&directory) {
            profile_paths.extend(profiles);
            game_jar_directories.push(directory);
        }
    }
    profile_paths.sort();
    profile_paths.dedup();
    if profile_paths.is_empty() && game_jar_directories.is_empty() {
        return Ok(None);
    }
    Ok(Some(LauncherLayout {
        kind: LauncherLayoutKind::SharedGameRoot,
        profile_paths,
        game_jar_directories,
        library_roots: existing_directories([instance_dir.join("libraries")]),
        components: Vec::new(),
    }))
}

fn discover_standalone(instance_dir: &Path) -> Result<Option<LauncherLayout>, OrbitError> {
    let profile_paths = launcher_profile_paths(instance_dir)?;
    if profile_paths.is_empty() && !contains_minecraft_version_jar(instance_dir) {
        return Ok(None);
    }
    Ok(Some(LauncherLayout {
        kind: LauncherLayoutKind::Standalone,
        profile_paths,
        game_jar_directories: vec![instance_dir.to_path_buf()],
        library_roots: existing_library_roots(instance_dir, 2),
        components: Vec::new(),
    }))
}

fn discover_multimc(instance_dir: &Path) -> Result<Option<LauncherLayout>, OrbitError> {
    let Some(instance_root) = instance_dir.parent() else {
        return Ok(None);
    };
    let pack_path = instance_root.join("mmc-pack.json");
    if !pack_path.is_file() {
        return Ok(None);
    }
    if !matches!(
        instance_dir.file_name().and_then(|name| name.to_str()),
        Some(name) if name.eq_ignore_ascii_case(".minecraft")
            || name.eq_ignore_ascii_case("minecraft")
    ) {
        return Err(invalid_game_directory(
            instance_dir,
            "mmc-pack.json belongs to the parent instance, but Orbit must be initialized in \
             that instance's .minecraft/ or minecraft/ game directory",
        ));
    }
    let pack: MultiMcPack =
        serde_json::from_slice(&std::fs::read(&pack_path)?).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "invalid Prism Launcher/MultiMC component file '{}': {error}",
                pack_path.display()
            ))
        })?;
    if pack.format_version != 1 {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported mmc-pack.json formatVersion {} in '{}'; expected 1",
            pack.format_version,
            pack_path.display()
        )));
    }
    let components = pack
        .components
        .into_iter()
        .filter(|component| !component.disabled)
        .filter_map(|component| {
            Some(LauncherComponent {
                uid: component.uid,
                version: component.version.or(component.cached_version)?,
            })
        })
        .collect::<Vec<_>>();
    if !components
        .iter()
        .any(|component| component.uid == "net.minecraft")
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "Prism Launcher/MultiMC component file '{}' has no enabled net.minecraft component",
            pack_path.display()
        )));
    }
    Ok(Some(LauncherLayout {
        kind: LauncherLayoutKind::MultiMc,
        profile_paths: Vec::new(),
        game_jar_directories: vec![instance_dir.to_path_buf()],
        library_roots: existing_library_roots(instance_root, 2),
        components,
    }))
}

fn discover_curseforge(instance_dir: &Path) -> Result<Option<LauncherLayout>, OrbitError> {
    let metadata = instance_dir.join("minecraftinstance.json");
    if !metadata.is_file() {
        return Ok(None);
    }
    validate_json_marker(&metadata, "CurseForge instance metadata")?;
    let (profile_paths, game_jar_directories) = launcher_assets(instance_dir)?;
    let mut library_roots = existing_library_roots(instance_dir, 1);
    if let Some(instances_dir) = instance_dir.parent()
        && file_name_eq(instances_dir, "Instances")
        && let Some(modding_root) = instances_dir.parent()
    {
        library_roots.push(modding_root.join("Install").join("libraries"));
    }
    library_roots = existing_directories(library_roots);
    Ok(Some(LauncherLayout {
        kind: LauncherLayoutKind::CurseForge,
        profile_paths,
        game_jar_directories,
        library_roots,
        components: Vec::new(),
    }))
}

fn discover_gdlauncher(instance_dir: &Path) -> Result<Option<LauncherLayout>, OrbitError> {
    let Some(instance_root) = instance_dir.parent() else {
        return Ok(None);
    };
    let metadata = instance_root.join("instance.json");
    if !file_name_eq(instance_dir, "instance") || !metadata.is_file() {
        return Ok(None);
    }
    validate_json_marker(&metadata, "GDLauncher instance metadata")?;
    let (profile_paths, game_jar_directories) = launcher_assets(instance_dir)?;
    Ok(Some(LauncherLayout {
        kind: LauncherLayoutKind::GdLauncher,
        profile_paths,
        game_jar_directories,
        library_roots: existing_library_roots(instance_root, 2),
        components: Vec::new(),
    }))
}

fn launcher_assets(instance_dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), OrbitError> {
    let mut profiles = launcher_profile_paths(instance_dir)?;
    let mut jar_directories = vec![instance_dir.to_path_buf()];
    let versions = instance_dir.join("versions");
    if versions.is_dir() {
        for directory in child_directories(&versions)? {
            profiles.extend(launcher_profile_paths(&directory)?);
            jar_directories.push(directory);
        }
    }
    profiles.sort();
    profiles.dedup();
    jar_directories.sort();
    jar_directories.dedup();
    Ok((profiles, jar_directories))
}

fn validate_json_marker(path: &Path, label: &str) -> Result<(), OrbitError> {
    serde_json::from_slice::<serde_json::Value>(&std::fs::read(path)?).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "invalid {label} '{}': {error}",
            path.display()
        ))
    })?;
    Ok(())
}

fn launcher_profile_paths(directory: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && crate::metadata::version_profile::VersionProfile::from_path(&path).is_ok()
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn contains_minecraft_version_jar(directory: &Path) -> bool {
    std::fs::read_dir(directory).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let path = entry.path();
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jar"))
                && crate::init::read_version_json_from_jar(&path).is_ok()
        })
    })
}

fn child_directories(directory: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    std::fs::read_dir(directory)?
        .filter_map(|entry| match entry {
            Ok(entry) if entry.path().is_dir() => Some(Ok(entry.path())),
            Ok(_) => None,
            Err(error) => Some(Err(OrbitError::Io(error))),
        })
        .collect()
}

fn existing_library_roots(start: &Path, parent_levels: usize) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut current = Some(start);
    for _ in 0..=parent_levels {
        let Some(directory) = current else {
            break;
        };
        roots.push(directory.join("libraries"));
        current = directory.parent();
    }
    existing_directories(roots)
}

fn existing_directories(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut paths = paths
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn is_dedicated_server(instance_dir: &Path) -> bool {
    instance_dir.join("server.properties").is_file() || instance_dir.join("eula.txt").is_file()
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

fn invalid_game_directory(instance_dir: &Path, reason: &str) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(
        "'{}' is not a supported Minecraft game directory: {reason}",
        instance_dir.display()
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultiMcPack {
    format_version: u32,
    #[serde(default)]
    components: Vec<MultiMcComponent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultiMcComponent {
    uid: String,
    version: Option<String>,
    cached_version: Option<String>,
    #[serde(default)]
    disabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "orbit-launcher-layout-{name}-{}",
            std::process::id()
        ));
        if path.exists() {
            std::fs::remove_dir_all(&path).unwrap();
        }
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn rejects_an_empty_directory() {
        let root = temp_dir("empty");

        let error = LauncherLayout::discover(&root).unwrap_err().to_string();

        assert!(error.contains("not a supported Minecraft game directory"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn isolated_version_does_not_include_sibling_profiles() {
        let root = temp_dir("isolated");
        let current = root.join("versions").join("fabric-current");
        let sibling = root.join("versions").join("forge-sibling");
        std::fs::create_dir_all(root.join("libraries")).unwrap();
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(
            current.join("current.json"),
            r#"{"id":"current","libraries":[{"name":"net.fabricmc:fabric-loader:0.19.2"}]}"#,
        )
        .unwrap();
        std::fs::write(
            sibling.join("sibling.json"),
            r#"{"id":"sibling","libraries":[{"name":"net.minecraftforge:forge:1.20.1-47.2.0"}]}"#,
        )
        .unwrap();

        let layout = LauncherLayout::discover(&current).unwrap();

        assert_eq!(layout.kind, LauncherLayoutKind::IsolatedVersion);
        assert_eq!(layout.profile_paths, vec![current.join("current.json")]);
        assert_eq!(layout.library_roots, vec![root.join("libraries")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_multimc_from_the_actual_game_directory() {
        let root = temp_dir("multimc");
        let instance = root.join("instances").join("example");
        let game_dir = instance.join(".minecraft");
        std::fs::create_dir_all(root.join("libraries")).unwrap();
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(
            instance.join("mmc-pack.json"),
            r#"{
  "formatVersion": 1,
  "components": [
    {"uid":"net.minecraft","version":"1.21.1"},
    {"uid":"net.fabricmc.fabric-loader","version":"0.16.14"}
  ]
}"#,
        )
        .unwrap();

        let layout = LauncherLayout::discover(&game_dir).unwrap();

        assert_eq!(layout.kind, LauncherLayoutKind::MultiMc);
        assert_eq!(layout.component_version("net.minecraft"), Some("1.21.1"));
        assert_eq!(
            layout.component_version("net.fabricmc.fabric-loader"),
            Some("0.16.14")
        );
        assert_eq!(layout.library_roots, vec![root.join("libraries")]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
