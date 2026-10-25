use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ExternalInstance, inspect_external_instances, invalid};
use crate::{
    CreateInstanceRequest, InstallProgressEvent, InstanceKind, LauncherError, RuntimeContext,
    apply_install_plan, prepare_install,
};

#[derive(Debug, Clone)]
pub struct ExternalImportRequest {
    pub directory: PathBuf,
    pub source: Option<String>,
    pub name: Option<String>,
    pub game_directory: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct ExternalImportPlan {
    pub source: ExternalInstance,
    pub game_directory: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub files: u64,
    pub bytes: u64,
    pub excluded: Vec<PathBuf>,
    pub resuming: bool,
    #[serde(skip)]
    inventory: Vec<DataEntry>,
}

const IMPORT_JOURNAL: &str = ".orbit-launcher/import.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportJournal {
    schema: u32,
    instance_id: uuid::Uuid,
    source_game_directory: PathBuf,
    source_profile_directory: PathBuf,
    source_id: String,
    runtime: super::ExternalRuntime,
}

fn pending_import(
    destination: &Path,
    source: &ExternalInstance,
    game: &Path,
) -> Result<crate::InstanceManifest, LauncherError> {
    let journal: ImportJournal =
        serde_json::from_value(super::read_json(&destination.join(IMPORT_JOURNAL))?)
            .map_err(|e| invalid(e.to_string()))?;
    let file = crate::ManifestFile::open(destination)?;
    if journal.schema != 1
        || journal.instance_id != file.inner.id
        || journal.source_game_directory != game
        || journal.source_profile_directory != source.profile_directory
        || journal.source_id != source.id
        || destination.file_name().and_then(|v| v.to_str()) != Some(file.inner.name.as_str())
        || source.runtime.as_ref() != Some(&journal.runtime)
        || file.inner.minecraft.requirement != journal.runtime.minecraft
        || file.inner.loader.kind != journal.runtime.loader
        || file.inner.loader.requirement != journal.runtime.loader_version
    {
        return Err(invalid(
            "existing destination is not the same unfinished import; refusing to merge or overwrite",
        ));
    }
    Ok(file.inner)
}

#[derive(Debug)]
struct DataEntry {
    relative: PathBuf,
    size: u64,
    modified: SystemTime,
    directory: bool,
}

#[derive(Debug, Clone)]
pub enum ExternalImportEvent {
    Install(InstallProgressEvent),
    Copying { completed: u64, total: u64 },
    Verified,
}

#[derive(Debug, Serialize)]
pub struct ExternalImportResult {
    pub entry: crate::RegistryEntry,
    pub source_directory: PathBuf,
    pub files: u64,
    pub bytes: u64,
    pub warnings: Vec<String>,
}

pub fn plan_external_import(
    runtime: &RuntimeContext,
    request: ExternalImportRequest,
) -> Result<ExternalImportPlan, LauncherError> {
    let catalog = inspect_external_instances(&request.directory)?;
    let source = match request.source {
        Some(id) => catalog.instances.into_iter().find(|c| c.id == id).ok_or_else(|| invalid("selected source no longer exists"))?,
        None if catalog.instances.len() == 1 => catalog.instances.into_iter().next().expect("one candidate"),
        None => return Err(LauncherError::InteractionRequired("multiple versions found; run 'instance inspect --directory <path>', then select --source <id>".into())),
    };
    if source.format == "orbit" {
        return Err(invalid(
            "an Orbit instance is registered in place; copying it is not an external import",
        ));
    }
    source.runtime.as_ref().ok_or_else(|| {
        invalid(
            source
                .error
                .as_deref()
                .unwrap_or("runtime metadata is missing"),
        )
    })?;
    let game_directory = request
        .game_directory
        .or_else(|| source.game_directory.clone())
        .ok_or_else(|| {
            LauncherError::InteractionRequired(format!(
                "{}; pass --game-directory with the actual working directory",
                source.directory_evidence
            ))
        })?;
    let game_directory = dunce::canonicalize(game_directory)?;
    if !game_directory.is_dir() || game_directory.parent().is_none() {
        return Err(invalid("invalid game data directory"));
    }
    let name = request.name.unwrap_or_else(|| source.name.clone());
    crate::instance::validate_instance_name(&name)?;
    crate::layout::validate_directory_name(&name).map_err(invalid)?;
    let destination = runtime.minecraft_directory().join("instances").join(&name);
    // Resolve existing ancestors before overlap checks (including redirected repository roots).
    let destination = resolve_new_path(&destination)?;
    if destination.starts_with(&game_directory) || game_directory.starts_with(&destination) {
        return Err(invalid(
            "import requires a new destination outside the source game directory",
        ));
    }
    crate::InstanceRegistry::load(&runtime.paths().instances_file())?.ensure_available(
        uuid::Uuid::new_v4(),
        &name,
        &destination,
    )?;
    let resuming = destination.exists();
    if resuming {
        pending_import(&destination, &source, &game_directory)?;
    }
    let mut inventory = Vec::new();
    let mut excluded = Vec::new();
    inventory_data(
        &game_directory,
        &game_directory,
        &source,
        &mut inventory,
        &mut excluded,
    )?;
    let files = inventory.iter().filter(|e| !e.directory).count() as u64;
    let bytes = inventory
        .iter()
        .try_fold(0u64, |sum, e| sum.checked_add(e.size))
        .ok_or_else(|| invalid("data size overflow"))?;
    Ok(ExternalImportPlan {
        source,
        game_directory,
        destination,
        name,
        files,
        bytes,
        excluded,
        resuming,
        inventory,
    })
}

fn resolve_new_path(path: &Path) -> Result<PathBuf, LauncherError> {
    if !path.is_absolute() {
        return Err(invalid("destination must be absolute"));
    }
    if path.exists() {
        return Ok(dunce::canonicalize(path)?);
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("invalid destination"))?;
    Ok(resolve_new_path(parent)?.join(
        path.file_name()
            .ok_or_else(|| invalid("invalid destination name"))?,
    ))
}

pub async fn import_external_instance<F>(
    runtime: &RuntimeContext,
    plan: ExternalImportPlan,
    mut progress: F,
) -> Result<ExternalImportResult, LauncherError>
where
    F: FnMut(ExternalImportEvent) + Send,
{
    // Recheck source metadata and inventory before allocating any target files.
    verify_inventory(&plan)?;
    if plan.destination.exists() && !plan.resuming {
        return Err(invalid(
            "destination was created after preview; refusing to merge",
        ));
    }
    let exact = plan
        .source
        .runtime
        .as_ref()
        .ok_or_else(|| invalid("runtime is missing"))?;
    let created = if plan.resuming {
        let manifest = pending_import(&plan.destination, &plan.source, &plan.game_directory)?;
        let location = crate::InstanceLocation::import(&plan.destination, InstanceKind::Client)?;
        crate::operations::CreateInstanceResult {
            entry: crate::RegistryEntry::from_manifest(location, &manifest),
            manifest,
        }
    } else {
        crate::operations::allocate_import_instance(
            runtime.paths(),
            CreateInstanceRequest {
                directory: runtime.minecraft_directory(),
                name: plan.name.clone(),
                kind: InstanceKind::Client,
                minecraft_requirement: exact.minecraft.clone(),
                loader_kind: exact.loader,
                loader_requirement: exact.loader_version.clone(),
            },
        )?
    };
    let root = created.entry.instance_directory();
    let result = async {
        if root != plan.destination {
            return Err(invalid("destination changed since import preview"));
        }
        let journal = ImportJournal {
            schema: 1,
            instance_id: created.entry.id,
            source_game_directory: plan.game_directory.clone(),
            source_profile_directory: plan.source.profile_directory.clone(),
            source_id: plan.source.id.clone(),
            runtime: exact.clone(),
        };
        crate::atomic_io::write_atomic(
            &root.join(IMPORT_JOURNAL),
            &serde_json::to_vec(&journal).map_err(|e| invalid(e.to_string()))?,
        )?;
        let client = runtime.config().download_client()?;
        let install = prepare_install(
            &created.entry.location,
            &client,
            &runtime.config().java.selection,
            |e| progress(ExternalImportEvent::Install(e)),
        )
        .await?;
        apply_install_plan(
            &created.entry.location,
            runtime.paths(),
            &client,
            install,
            usize::from(runtime.config().network.concurrency),
            runtime.config().installer.timeout_seconds,
            |e| progress(ExternalImportEvent::Install(e)),
        )
        .await?;
        let _lease = crate::instance_lifecycle::InstanceLease::acquire(root, true)?;
        copy_data(&plan, root, &mut progress)?;
        verify_inventory(&plan)?;
        crate::operations::import_instance(runtime.paths(), root)?;
        progress(ExternalImportEvent::Verified);
        let mut warnings = plan.source.warnings.clone();
        if let Err(error) = std::fs::remove_file(root.join(IMPORT_JOURNAL)) {
            warnings.push(format!(
                "Import completed but its recovery marker could not be removed: {error}"
            ));
        }
        Ok(ExternalImportResult {
            entry: created.entry.clone(),
            source_directory: plan.game_directory.clone(),
            files: plan.files,
            bytes: plan.bytes,
            warnings,
        })
    }
    .await;
    if let Err(error) = result {
        if plan.resuming {
            return Err(LauncherError::Transaction(format!(
                "import did not complete: {error}; resumable target preserved at '{}'",
                root.display()
            )));
        }
        // Only the newly allocated instance is rolled back. Shared verified runtime objects remain reusable.
        std::fs::remove_dir_all(root).map_err(|cleanup| {
            LauncherError::Transaction(format!(
                "import failed: {error}; cannot clean newly created target '{}': {cleanup}",
                root.display()
            ))
        })?;
        return Err(error);
    }
    result
}

fn excluded_root(name: &str, source: &ExternalInstance) -> bool {
    let name = name.to_ascii_lowercase();
    // These are launcher/runtime state, not opaque game data. No mod-specific allowlist.
    matches!(
        name.as_str(),
        "assets"
            | "bin"
            | "libraries"
            | "versions"
            | "instances"
            | "natives"
            | "runtime"
            | "runtimes"
            | ".hmcl"
            | "pcl"
            | ".orbit-launcher"
            | "logs"
            | "crash-reports"
            | "launcher_profiles.json"
            | "launcher_profiles_microsoft_store.json"
            | "launcher_accounts.json"
            | "launcher_msa_credentials.bin"
            | "accounts.json"
            | "hmcl.json"
            | ".hmcl.json"
            | "hmclversion.cfg"
            | "modpack.cfg"
            | "instance.cfg"
            | "mmc-pack.json"
            | "minecraftinstance.json"
            | "instance.json"
            | "orbit-launcher.toml"
            | "orbit-launcher.lock"
            | "minecraft.jar"
    ) || name.starts_with("natives-")
        || name.ends_with(".exe")
        || name == format!("{}.json", source.id.to_ascii_lowercase())
        || name == format!("{}.jar", source.id.to_ascii_lowercase())
}

fn inventory_data(
    root: &Path,
    directory: &Path,
    source: &ExternalInstance,
    output: &mut Vec<DataEntry>,
    excluded: &mut Vec<PathBuf>,
) -> Result<(), LauncherError> {
    let mut children = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|e| invalid(e.to_string()))?
            .to_path_buf();
        if directory == root && excluded_root(&child.file_name().to_string_lossy(), source) {
            excluded.push(relative);
            continue;
        }
        let meta = std::fs::symlink_metadata(&path)?;
        if crate::instance_lifecycle::is_redirected(&meta) || (!meta.is_file() && !meta.is_dir()) {
            return Err(invalid(format!(
                "linked or special data path '{}' cannot be imported; replace it with ordinary files first",
                path.display()
            )));
        }
        output.push(DataEntry {
            relative,
            size: if meta.is_file() { meta.len() } else { 0 },
            modified: meta.modified()?,
            directory: meta.is_dir(),
        });
        if meta.is_dir() {
            inventory_data(root, &path, source, output, excluded)?;
        }
    }
    Ok(())
}

fn verify_inventory(plan: &ExternalImportPlan) -> Result<(), LauncherError> {
    let mut now = Vec::new();
    inventory_data(
        &plan.game_directory,
        &plan.game_directory,
        &plan.source,
        &mut now,
        &mut Vec::new(),
    )?;
    if now.len() != plan.inventory.len()
        || now.iter().zip(&plan.inventory).any(|(a, b)| {
            a.relative != b.relative
                || a.size != b.size
                || a.modified != b.modified
                || a.directory != b.directory
        })
    {
        return Err(invalid(
            "source data changed during import; close the game and retry",
        ));
    }
    Ok(())
}

fn copy_data(
    plan: &ExternalImportPlan,
    destination: &Path,
    progress: &mut impl FnMut(ExternalImportEvent),
) -> Result<(), LauncherError> {
    let staging = destination.join(".orbit-launcher/import-files");
    if staging.exists() {
        let metadata = std::fs::symlink_metadata(&staging)?;
        if crate::instance_lifecycle::is_redirected(&metadata)
            || !metadata.is_dir()
            || !dunce::canonicalize(&staging)?.starts_with(dunce::canonicalize(destination)?)
        {
            return Err(invalid(
                "import copy staging was redirected outside the target",
            ));
        }
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let mut completed = 0;
    progress(ExternalImportEvent::Copying {
        completed,
        total: plan.bytes,
    });
    let mut buffer = vec![0u8; 256 * 1024];
    for entry in &plan.inventory {
        let target = destination.join(&entry.relative);
        if entry.directory {
            if !target.exists() {
                std::fs::create_dir(&target)?;
            }
            let metadata = std::fs::symlink_metadata(&target)?;
            if !metadata.is_dir() || crate::instance_lifecycle::is_redirected(&metadata) {
                return Err(invalid(
                    "destination data directory was replaced by a link or file",
                ));
            }
            continue;
        }
        let source = plan.game_directory.join(&entry.relative);
        if plan.resuming && target.exists() {
            let metadata = std::fs::symlink_metadata(&target)?;
            if crate::instance_lifecycle::is_redirected(&metadata)
                || !metadata.is_file()
                || crate::hash_file_sha256(&target)? != crate::hash_file_sha256(&source)?
            {
                return Err(invalid(format!(
                    "unfinished target data '{}' differs from the source; preserved without overwriting",
                    target.display()
                )));
            }
            completed += entry.size;
            progress(ExternalImportEvent::Copying {
                completed,
                total: plan.bytes,
            });
            continue;
        }
        let mut input = std::fs::File::open(&source)?;
        let mut output = tempfile::NamedTempFile::new_in(&staging)?;
        let mut digest = Sha256::new();
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            digest.update(&buffer[..read]);
            completed += read as u64;
            progress(ExternalImportEvent::Copying {
                completed,
                total: plan.bytes,
            });
        }
        output.as_file().sync_all()?;
        std::fs::set_permissions(output.path(), input.metadata()?.permissions())?;
        if hex::encode(digest.finalize()) != crate::hash_file_sha256(output.path())? {
            return Err(invalid("copied data failed verification"));
        }
        output
            .persist_noclobber(&target)
            .map_err(|e| LauncherError::Io(e.error))?;
    }
    std::fs::remove_dir(&staging)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuntimePathOptions;

    fn fixture() -> (tempfile::TempDir, RuntimeContext, ExternalImportRequest) {
        let temp = tempfile::tempdir().unwrap();
        let runtime = RuntimeContext::load(RuntimePathOptions {
            config_dir: Some(temp.path().join("config")),
            data_dir: Some(temp.path().join("data")),
            cache_dir: Some(temp.path().join("cache")),
        })
        .unwrap();
        let source = temp.path().join("foreign");
        let version = source.join("versions/demo");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(version.join("demo.json"),r#"{"id":"demo","inheritsFrom":"1.21.1","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.14"}]}"#).unwrap();
        std::fs::write(
            version.join("hmclversion.cfg"),
            r#"{"usesGlobal":false,"gameDirType":1}"#,
        )
        .unwrap();
        for (path, content) in [
            ("mods/example.jar", "opaque mod"),
            ("saves/world/level.dat", "world"),
            ("custom-mod-data/user.schematic", "user data"),
            ("assets/shared", "runtime"),
            ("launcher_accounts.json", "secret"),
            ("launcher_msa_credentials.bin", "secret"),
        ] {
            let file = version.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, content).unwrap();
        }
        (
            temp,
            runtime,
            ExternalImportRequest {
                directory: source,
                source: None,
                name: Some("Imported".into()),
                game_directory: None,
            },
        )
    }

    #[test]
    fn preview_is_readonly_and_opaque_data_copy_excludes_runtime_and_credentials() {
        let (temp, runtime, request) = fixture();
        let plan = plan_external_import(&runtime, request).unwrap();
        assert_eq!(plan.files, 3);
        assert!(!runtime.paths().instances_file().exists());
        assert!(!plan.destination.exists());
        let target = temp.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let mut events = Vec::new();
        copy_data(&plan, &target, &mut |e| events.push(e)).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("custom-mod-data/user.schematic")).unwrap(),
            "user data"
        );
        assert!(target.join("mods/example.jar").is_file());
        assert!(target.join("saves/world/level.dat").is_file());
        assert!(!target.join("assets").exists());
        assert!(!target.join("launcher_accounts.json").exists());
        assert!(!target.join("launcher_msa_credentials.bin").exists());
        assert!(plan.game_directory.join("launcher_accounts.json").is_file());
        verify_inventory(&plan).unwrap();
        assert!(
            matches!(events.last(),Some(ExternalImportEvent::Copying{completed,total}) if completed==total)
        );
    }

    #[test]
    fn data_changes_and_existing_targets_are_not_silently_merged() {
        let (temp, runtime, request) = fixture();
        let plan = plan_external_import(&runtime, request.clone()).unwrap();
        std::fs::write(plan.game_directory.join("new-user-file"), "new").unwrap();
        assert!(verify_inventory(&plan).is_err());
        std::fs::create_dir_all(&plan.destination).unwrap();
        assert!(plan_external_import(&runtime, request).is_err());
        assert!(
            temp.path()
                .join("foreign/versions/demo/mods/example.jar")
                .is_file()
        );
    }

    #[test]
    fn interrupted_import_reuses_only_verified_files_and_is_not_yet_registered() {
        let (_temp, runtime, request) = fixture();
        let plan = plan_external_import(&runtime, request.clone()).unwrap();
        let exact = plan.source.runtime.as_ref().unwrap().clone();
        let created = crate::operations::allocate_import_instance(
            runtime.paths(),
            CreateInstanceRequest {
                directory: runtime.minecraft_directory(),
                name: plan.name.clone(),
                kind: InstanceKind::Client,
                minecraft_requirement: exact.minecraft.clone(),
                loader_kind: exact.loader,
                loader_requirement: exact.loader_version.clone(),
            },
        )
        .unwrap();
        let journal = ImportJournal {
            schema: 1,
            instance_id: created.entry.id,
            source_game_directory: plan.game_directory.clone(),
            source_profile_directory: plan.source.profile_directory.clone(),
            source_id: plan.source.id.clone(),
            runtime: exact,
        };
        crate::atomic_io::write_atomic(
            &plan.destination.join(IMPORT_JOURNAL),
            &serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        copy_data(&plan, &plan.destination, &mut |_| {}).unwrap();
        // A process killed mid-copy leaves only staging, not a partial published user file.
        std::fs::create_dir_all(plan.destination.join(".orbit-launcher/import-files")).unwrap();
        std::fs::write(
            plan.destination
                .join(".orbit-launcher/import-files/partial"),
            "partial",
        )
        .unwrap();
        let resumed = plan_external_import(&runtime, request).unwrap();
        assert!(resumed.resuming);
        copy_data(&resumed, &resumed.destination, &mut |_| {}).unwrap();
        assert!(
            !resumed
                .destination
                .join(".orbit-launcher/import-files")
                .exists()
        );
        assert!(
            crate::InstanceRegistry::load(&runtime.paths().instances_file())
                .unwrap()
                .instances
                .is_empty()
        );
        std::fs::write(
            resumed.destination.join("mods/example.jar"),
            "user modified target",
        )
        .unwrap();
        assert!(copy_data(&resumed, &resumed.destination, &mut |_| {}).is_err());
        assert_eq!(
            std::fs::read_to_string(resumed.destination.join("mods/example.jar")).unwrap(),
            "user modified target"
        );
    }

    #[test]
    fn redirected_destination_cannot_copy_the_source_into_itself() {
        let (_temp, runtime, mut request) = fixture();
        request.game_directory = Some(
            runtime
                .minecraft_directory()
                .parent()
                .unwrap()
                .to_path_buf(),
        );
        std::fs::create_dir_all(request.game_directory.as_ref().unwrap()).unwrap();
        assert!(plan_external_import(&runtime, request).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn linked_data_is_not_followed_outside_the_instance() {
        let (temp, runtime, request) = fixture();
        std::os::unix::fs::symlink(
            temp.path(),
            temp.path().join("foreign/versions/demo/linked"),
        )
        .unwrap();
        assert!(
            plan_external_import(&runtime, request)
                .unwrap_err()
                .to_string()
                .contains("linked")
        );
    }
}
