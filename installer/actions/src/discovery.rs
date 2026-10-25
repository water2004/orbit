use crate::transaction::{self, Entry, Result};
use orbit_launcher_core::{InstanceRegistry, RuntimePathOptions, RuntimePaths};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CleanupLevel {
    Cache,
    Settings,
    All,
}
impl std::str::FromStr for CleanupLevel {
    type Err = Box<dyn std::error::Error + Send + Sync>;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "cache" => Ok(Self::Cache),
            "settings" => Ok(Self::Settings),
            "all" => Ok(Self::All),
            _ => Err("UNINSTALL_DATA_LEVEL must be cache, settings or all".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roots {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub gui_config: PathBuf,
    pub gui_data: PathBuf,
    pub gui_cache: PathBuf,
    pub gui_webview: PathBuf,
    pub forbidden: Vec<PathBuf>,
}
impl Roots {
    pub fn launcher_paths(&self) -> Result<RuntimePaths> {
        Ok(RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(self.config.join("orbit-launcher")),
            data_dir: Some(self.data.join("orbit-launcher")),
            cache_dir: Some(self.cache.join("orbit-launcher")),
        })?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Game {
    pub name: String,
    pub directory: PathBuf,
    pub launcher_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub roots: Roots,
    pub level: CleanupLevel,
    pub games: Vec<Game>,
    pub repositories: Vec<PathBuf>,
    pub game_entries: Vec<Entry>,
    pub data_entries: Vec<Entry>,
    pub prune: Vec<PathBuf>,
    pub snapshots: Vec<(PathBuf, Option<[u8; 32]>)>,
}

fn canonical_if_present(path: PathBuf) -> Result<PathBuf> {
    transaction::check_root(&path)?;
    if transaction::metadata(&path)?.is_some() {
        Ok(dunce::canonicalize(path)?)
    } else {
        Ok(path)
    }
}

pub fn discover(roots: Roots, level: CleanupLevel) -> Result<Plan> {
    for root in [&roots.config, &roots.data, &roots.cache] {
        transaction::check_root(root)?;
    }
    let paths = roots.launcher_paths()?;
    let orbit_dir = roots.config.join("orbit");
    let snapshots = [
        paths.instances_file(),
        paths.config_file(),
        orbit_dir.join("instances.toml"),
        orbit_dir.join("config.toml"),
    ]
    .into_iter()
    .map(|path| Ok((path.clone(), fingerprint(&path)?)))
    .collect::<Result<Vec<_>>>()?;
    let launcher_config = orbit_launcher_core::config::GlobalConfig::inspect(&paths.config_file())
        .map_err(|_| {
            format!(
                "cannot read Launcher configuration '{}'; repair it before cleaning data",
                paths.config_file().display()
            )
        })?;
    let registry = InstanceRegistry::load(&paths.instances_file())?;
    let orbit_registry =
        orbit_core::config::InstancesRegistry::load(&orbit_dir.join("instances.toml"))?;
    let default_minecraft = paths.data_dir().join("minecraft");
    let mut repositories = vec![
        default_minecraft.clone(),
        launcher_config
            .minecraft
            .directory
            .clone()
            .unwrap_or(default_minecraft),
    ];
    let mut protected_games = Vec::new();
    let mut games = Vec::new();
    for entry in &registry.instances {
        let directory = entry.instance_directory().to_path_buf();
        protected_games.push(directory.clone());
        if let Some(repository) = entry.location.minecraft_directory() {
            repositories.push(repository.to_path_buf());
        }
        if level == CleanupLevel::All && transaction::metadata(&directory)?.is_some() {
            let verified =
                orbit_launcher_core::plan_instance_deletion(&paths, &entry.id.to_string())?;
            games.push(Game {
                name: entry.name.clone(),
                directory: verified.directory,
                launcher_id: Some(entry.id.to_string()),
            });
        }
    }
    for entry in &orbit_registry.instances {
        let directory = PathBuf::from(&entry.path);
        protected_games.push(directory.clone());
        if level == CleanupLevel::All
            && transaction::metadata(&directory)?.is_some()
            && !games.iter().any(|game| game.directory == directory)
        {
            transaction::check_root(&directory)?;
            orbit_core::ManifestFile::open(&directory)?;
            games.push(Game {
                name: entry.name.clone(),
                directory,
                launcher_id: None,
            });
        }
    }
    repositories = repositories
        .into_iter()
        .map(canonical_if_present)
        .collect::<Result<_>>()?;
    repositories.sort();
    repositories.dedup();
    protected_games.extend(repositories.clone());
    // Installed runtimes are not disposable downloads: retained instances may
    // refer to these exact Java executables in their lock files.
    protected_games.push(paths.data_dir().join("runtimes"));
    protected_games.push(paths.instances_file());
    protected_games.push(orbit_dir.join("instances.toml"));
    let owned = vec![
        roots.config.join("orbit"),
        roots.cache.join("orbit"),
        paths.config_dir().into(),
        paths.data_dir().into(),
        paths.cache_dir().into(),
        roots.gui_config.clone(),
        roots.gui_data.clone(),
        roots.gui_cache.clone(),
        roots.gui_webview.clone(),
    ]
    .into_iter()
    .map(canonical_if_present)
    .collect::<Result<Vec<_>>>()?;
    let mut data_paths = Vec::new();
    let mut forbidden = roots.forbidden.clone();
    forbidden.extend([
        roots.config.clone(),
        roots.data.clone(),
        roots.cache.clone(),
    ]);
    let protected_games = protected_games
        .into_iter()
        .map(canonical_if_present)
        .collect::<Result<Vec<_>>>()?;
    if level == CleanupLevel::All {
        // The top tier removes whole managed repositories, including residual
        // unregistered folders. They are explicitly named in the confirmation.
        for root in owned.iter().chain(&repositories) {
            if transaction::metadata(root)?.is_some() {
                data_paths.push(root.clone());
            }
        }
    } else {
        let mut protected = protected_games;
        if level == CleanupLevel::Cache {
            protected.extend([
                roots.config.join("orbit"),
                paths.config_dir().to_path_buf(),
                paths.accounts_file(),
                paths.instances_file(),
                paths.data_dir().join("credentials"),
                roots.gui_config.clone(),
                roots.gui_data.clone(),
            ]);
        }
        let protected = protected
            .into_iter()
            .map(canonical_if_present)
            .collect::<Result<Vec<_>>>()?;
        if level == CleanupLevel::Settings {
            for root in &owned {
                transaction::partition(root, &protected, &mut data_paths)?;
            }
        } else {
            let mut caches = vec![
                roots.cache.join("orbit"),
                paths.account_avatars_dir(),
                paths.auth_sessions_dir(),
                roots.gui_cache.clone(),
                roots.gui_webview.clone(),
            ];
            if paths.cache_dir() != paths.data_dir() && paths.cache_dir() != paths.config_dir() {
                caches.push(paths.cache_dir().into());
            }
            caches.extend(
                orbit_launcher_core::artifact::ArtifactCache::new(paths.cache_dir())
                    .cleanup_paths(),
            );
            for root in caches {
                transaction::partition(&root, &protected, &mut data_paths)?;
            }
        }
    }
    // Configured caches outside AppData cannot swallow retained game storage.
    let config_file = orbit_dir.join("config.toml");
    if transaction::metadata(&config_file)?.is_some() {
        let config = orbit_core::config::GlobalConfig::load_stored(&config_file).map_err(|_| {
            format!(
                "cannot read Orbit configuration '{}'; repair it before cleaning data",
                config_file.display()
            )
        })?;
        let mut keep = if level == CleanupLevel::All {
            vec![]
        } else {
            let mut keep = repositories.clone();
            keep.extend(
                registry
                    .instances
                    .iter()
                    .map(|entry| entry.instance_directory().to_path_buf()),
            );
            keep.extend(
                orbit_registry
                    .instances
                    .iter()
                    .map(|entry| PathBuf::from(&entry.path)),
            );
            keep
        };
        if level == CleanupLevel::Cache {
            keep.extend([
                orbit_dir,
                paths.config_dir().into(),
                roots.gui_config.clone(),
                roots.gui_data.clone(),
            ]);
        }
        let keep = keep
            .into_iter()
            .map(canonical_if_present)
            .collect::<Result<Vec<_>>>()?;
        for root in [config.cache.dir, config.repository.dir]
            .into_iter()
            .flatten()
        {
            transaction::check_target(Path::new(&root), &forbidden)?;
            transaction::partition(Path::new(&root), &keep, &mut data_paths)?;
        }
    }
    let mut leaf_games = Vec::new();
    for game in &games {
        // An Orbit-only workspace may be a parent of Launcher instances.
        // Stage that parent only after its lease-bound children are detached.
        let contains_leased_child = game.launcher_id.is_none()
            && games.iter().any(|child| {
                child.launcher_id.is_some()
                    && child.directory != game.directory
                    && child.directory.starts_with(&game.directory)
            });
        if contains_leased_child {
            data_paths.push(game.directory.clone());
        } else {
            leaf_games.push(game.directory.clone());
        }
    }
    let game_entries = transaction::entries(leaf_games, &forbidden)?;
    data_paths.retain(|path| {
        !game_entries
            .iter()
            .any(|game| path.starts_with(&game.source))
    });
    let data_entries = transaction::entries(data_paths, &forbidden)?;
    let mut prune = Vec::new();
    for game in &games {
        if game.launcher_id.is_some()
            && let Some(parent) = game.directory.parent()
        {
            prune.push(
                parent.join(orbit_launcher_core::instance_deletion::INSTANCE_LEASE_DIRECTORY),
            );
        }
    }
    for entry in game_entries.iter().chain(&data_entries) {
        for parent in entry.source.ancestors().skip(1) {
            if !forbidden.iter().any(|root| root == parent)
                && [&roots.config, &roots.data, &roots.cache]
                    .iter()
                    .any(|base| parent.starts_with(base))
            {
                prune.push(parent.to_path_buf());
            } else {
                break;
            }
        }
    }
    prune.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    prune.dedup();
    Ok(Plan {
        roots,
        level,
        games,
        repositories,
        game_entries,
        data_entries,
        prune,
        snapshots,
    })
}

pub fn leases(
    plan: &Plan,
) -> Result<Vec<orbit_launcher_core::instance_deletion::InstanceDeletionLease>> {
    for (path, expected) in &plan.snapshots {
        if fingerprint(path)? != *expected {
            return Err(format!(
                "settings or instance registry changed after confirmation: {}",
                path.display()
            )
            .into());
        }
    }
    let paths = plan.roots.launcher_paths()?;
    let mut leases = Vec::new();
    for game in &plan.games {
        if let Some(id) = &game.launcher_id {
            let current = orbit_launcher_core::plan_instance_deletion(&paths, id)?;
            if current.directory != game.directory {
                return Err("instance moved after uninstall confirmation".into());
            }
            leases.push(
                orbit_launcher_core::instance_deletion::lease_instance_deletion(&paths, &current)?,
            );
        } else {
            orbit_core::ManifestFile::open(&game.directory)?;
        }
    }
    Ok(leases)
}
fn fingerprint(path: &Path) -> Result<Option<[u8; 32]>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(Sha256::digest(&bytes).into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
pub fn prune_empty(plan: &Plan) -> Result<()> {
    for path in &plan.prune {
        transaction::check_root(path)?;
        match std::fs::remove_dir(path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
pub fn preview(plan: &Plan) -> String {
    use orbit_i18n::text as t;
    let level = match plan.level {
        CleanupLevel::Cache => "All caches only",
        CleanupLevel::Settings => "Configuration and caches (keep instances)",
        CleanupLevel::All => "Everything, including instances and worlds",
    };
    let mut text = format!(
        "{} {}\n\n{}\n",
        t("Uninstall cleanup:"),
        t(level),
        t("The listed data will be permanently deleted. Review all paths before confirming.")
    );
    if plan.level == CleanupLevel::All {
        text.push_str(&format!(
            "\n{}\n",
            t("Game instances (worlds, mods and all instance files):")
        ));
        for game in &plan.games {
            text.push_str(&format!("{}: {}\n", game.name, game.directory.display()));
        }
        text.push_str(&format!(
            "\n{}\n",
            t("Whole managed Minecraft repositories (including residual files):")
        ));
        for root in &plan.repositories {
            if root.exists() {
                text.push_str(&format!("{}\n", root.display()));
            }
        }
    } else {
        text.push_str(&format!("\n{}\n", t("All game instances, shared installed Minecraft resources and Java runtimes will be kept.")));
    }
    text.push_str(&format!("\n{}\n", t("Exact cleanup locations:")));
    for entry in plan.game_entries.iter().chain(&plan.data_entries) {
        text.push_str(&format!("{}\n", entry.source.display()));
    }
    text.push_str(&format!(
        "\n{}",
        t("Other users' data is not included. Continue?")
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Roots, orbit_launcher_core::RegistryEntry) {
        fixture_layout(false)
    }
    fn fixture_layout(
        split_cache: bool,
    ) -> (tempfile::TempDir, Roots, orbit_launcher_core::RegistryEntry) {
        use orbit_launcher_core::{CreateInstanceRequest, InstanceKind, LoaderKind};
        let temp = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(temp.path()).unwrap();
        let roots = Roots {
            config: base.join("config"),
            data: base.join("data"),
            cache: base.join(if split_cache { "cache" } else { "data" }),
            gui_config: base.join("config/Orbit/Orbit GUI/config"),
            gui_data: base.join("data/Orbit/Orbit GUI/data"),
            gui_cache: base.join(if split_cache {
                "cache/Orbit/Orbit GUI"
            } else {
                "data/Orbit/Orbit GUI/cache"
            }),
            gui_webview: base.join("data/dev.orbit.gui"),
            forbidden: vec![base.clone()],
        };
        let paths = roots.launcher_paths().unwrap();
        let created = orbit_launcher_core::create_instance(
            &paths,
            CreateInstanceRequest {
                directory: roots.data.join("orbit-launcher/minecraft"),
                name: "Test".into(),
                kind: InstanceKind::Client,
                minecraft_requirement: "26.2".into(),
                loader_kind: LoaderKind::Vanilla,
                loader_requirement: None,
            },
        )
        .unwrap();
        for (path, content) in [
            (
                created
                    .entry
                    .instance_directory()
                    .join("saves/world/level.dat"),
                "world",
            ),
            (
                paths.data_dir().join("runtimes/java/bin/java.exe"),
                "installed java",
            ),
            (paths.cache_dir().join("objects/cache"), "cached jar"),
            (roots.gui_webview.join("EBWebView/cache"), "webview cache"),
            (roots.gui_config.join("preferences.json"), "{}"),
            (
                roots
                    .data
                    .join("orbit-launcher/minecraft/instances/Residual/saves/level.dat"),
                "residual world",
            ),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        orbit_launcher_core::config::GlobalConfig::default()
            .save(&paths.config_file())
            .unwrap();
        (temp, roots, created.entry)
    }
    #[test]
    fn three_levels_remove_all_caches_but_only_all_removes_games() {
        for split_cache in [false, true] {
            for level in [
                CleanupLevel::Cache,
                CleanupLevel::Settings,
                CleanupLevel::All,
            ] {
                let (_temp, roots, game) = fixture_layout(split_cache);
                let paths = roots.launcher_paths().unwrap();
                let plan = discover(roots.clone(), level).unwrap();
                crate::execution::stage(&plan).unwrap();
                crate::execution::commit(&plan).unwrap();
                assert!(!paths.cache_dir().join("objects/cache").exists());
                assert!(!roots.gui_webview.join("EBWebView/cache").exists());
                assert_eq!(paths.config_file().exists(), level == CleanupLevel::Cache);
                assert_eq!(
                    roots.gui_config.join("preferences.json").exists(),
                    level == CleanupLevel::Cache
                );
                assert_eq!(
                    game.instance_directory()
                        .join("saves/world/level.dat")
                        .exists(),
                    level != CleanupLevel::All
                );
                assert_eq!(
                    paths.data_dir().join("runtimes/java/bin/java.exe").exists(),
                    level != CleanupLevel::All
                );
                assert_eq!(
                    paths
                        .data_dir()
                        .join("minecraft/instances/Residual/saves/level.dat")
                        .exists(),
                    level != CleanupLevel::All
                );
                assert_eq!(paths.instances_file().exists(), level != CleanupLevel::All);
            }
        }
    }
    #[test]
    fn rollback_restores_nested_worlds_and_settings_at_every_level() {
        for level in [
            CleanupLevel::Cache,
            CleanupLevel::Settings,
            CleanupLevel::All,
        ] {
            let (_temp, roots, game) = fixture();
            let paths = roots.launcher_paths().unwrap();
            let original = std::fs::read(paths.instances_file()).unwrap();
            let plan = discover(roots, level).unwrap();
            crate::execution::stage(&plan).unwrap();
            crate::execution::rollback(&plan).unwrap();
            assert_eq!(std::fs::read(paths.instances_file()).unwrap(), original);
            assert_eq!(
                std::fs::read_to_string(game.instance_directory().join("saves/world/level.dat"))
                    .unwrap(),
                "world"
            );
            assert!(paths.config_file().exists());
            assert!(paths.cache_dir().join("objects/cache").exists());
        }
    }
    #[test]
    fn edits_after_confirmation_abort_before_staging() {
        let (_temp, roots, game) = fixture();
        let paths = roots.launcher_paths().unwrap();
        let plan = discover(roots, CleanupLevel::All).unwrap();
        let mut registry = InstanceRegistry::load(&paths.instances_file()).unwrap();
        registry.instances[0].name = "Changed".into();
        registry.save(&paths.instances_file()).unwrap();
        assert!(crate::execution::stage(&plan).is_err());
        assert!(
            game.instance_directory()
                .join("saves/world/level.dat")
                .exists()
        );
    }
    #[test]
    fn preview_keeps_source_bytes_and_never_serializes_api_keys() {
        let (_temp, roots, _game) = fixture();
        let paths = roots.launcher_paths().unwrap();
        std::fs::write(paths.config_file(), "schema = 2\n").unwrap();
        let mut config = orbit_core::config::GlobalConfig::default();
        config.auth.modrinth_token = Some("never-copy-this-token-into-msi-properties".into());
        let core_path = roots.config.join("orbit/config.toml");
        config.save(&core_path).unwrap();
        let original = std::fs::read(&core_path).unwrap();
        let plan = discover(roots, CleanupLevel::All).unwrap();
        assert!(
            !serde_json::to_string(&plan)
                .unwrap()
                .contains("never-copy-this-token")
        );
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"schema = 2\n");
        assert_eq!(std::fs::read(core_path).unwrap(), original);
        assert_eq!(std::fs::read_dir(paths.config_dir()).unwrap().count(), 1);
    }
    #[test]
    fn configured_broad_cache_cannot_be_partitioned_into_unrelated_applications() {
        let (_temp, roots, _game) = fixture_layout(true);
        let unrelated = roots.config.join("unrelated/settings");
        std::fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        std::fs::write(&unrelated, "keep").unwrap();
        let mut config = orbit_core::config::GlobalConfig::default();
        config.cache.dir = Some(roots.config.to_string_lossy().into_owned());
        config
            .save(&roots.config.join("orbit/config.toml"))
            .unwrap();
        assert!(discover(roots, CleanupLevel::Cache).is_err());
        assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "keep");
    }
}
