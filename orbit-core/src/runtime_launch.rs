//! Joint Orbit + Orbit Launcher process orchestration.
//!
//! Orbit owns runtime observation. Orbit Launcher remains an independent
//! runtime launcher and receives the Java agent only through the child process
//! environment.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;

use crate::error::{OrbitError, RuntimeComponent, RuntimeDataError};
use crate::runtime_agent::capabilities_for;
use crate::runtime_data::{
    RESERVED_INSTANCE_ROOTS, merge_observation_sessions, observation_session_path,
    ownership_context, prune_missing_ownership,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLaunchTarget {
    Client,
    Server,
}

#[derive(Debug, Clone)]
pub struct RuntimeLaunchRequest {
    pub instance_dir: PathBuf,
    pub launcher_program: PathBuf,
    pub runtime_agent: PathBuf,
    pub launcher_instance: Option<String>,
    pub target: RuntimeLaunchTarget,
    pub language: String,
    pub output_format: String,
    pub progress_format: String,
    pub non_interactive: bool,
    pub dry_run: bool,
}

pub fn launch_with_runtime_observation(request: &RuntimeLaunchRequest) -> Result<(), OrbitError> {
    let instance_dir = dunce::canonicalize(&request.instance_dir)?;
    if !instance_dir.join("orbit.toml").is_file() {
        return Err(OrbitError::ManifestNotFound);
    }
    if !instance_dir.join("orbit.lock").is_file() {
        return Err(OrbitError::LockfileNotFound);
    }
    let launcher_program = absolute_file(&request.launcher_program, RuntimeComponent::Launcher)?;
    let runtime_agent = absolute_file(&request.runtime_agent, RuntimeComponent::Agent)?;
    if request.target == RuntimeLaunchTarget::Server && request.dry_run {
        return Err(OrbitError::RuntimeData(RuntimeDataError::ServerDryRun));
    }

    let java_tool_options = if request.dry_run {
        None
    } else {
        merge_observation_sessions(&instance_dir)?;
        prune_missing_ownership(&instance_dir)?;
        let session = observation_session_path(&instance_dir)?;
        let context = write_agent_context(&instance_dir)?;
        let agent_option = java_agent_option(&runtime_agent, &instance_dir, &session, &context)?;
        Some(append_java_tool_option(
            std::env::var_os("JAVA_TOOL_OPTIONS").as_deref(),
            &agent_option,
        )?)
    };

    let mut command = Command::new(launcher_program);
    command
        .current_dir(&instance_dir)
        .arg("--language")
        .arg(&request.language)
        .arg("--output-format")
        .arg(&request.output_format)
        .arg("--progress-format")
        .arg(&request.progress_format);
    if let Some(java_tool_options) = java_tool_options {
        command.env("JAVA_TOOL_OPTIONS", java_tool_options);
    }
    if request.non_interactive {
        command.arg("--non-interactive");
    }
    if let Some(instance) = &request.launcher_instance {
        command.arg("--instance").arg(instance);
    }
    match request.target {
        RuntimeLaunchTarget::Client => {
            command.arg("launch");
            if request.dry_run {
                command.arg("--dry-run");
            }
        }
        RuntimeLaunchTarget::Server => {
            command.args(["server", "start"]);
        }
    }

    let status = command.status()?;
    // Client launch normally blocks until Java exits. Server start normally
    // detaches; its snapshot is merged by the next Orbit launch/purge command.
    if !request.dry_run {
        merge_observation_sessions(&instance_dir)?;
    }
    if status.success() {
        Ok(())
    } else {
        Err(OrbitError::ForwardedProcessExit(status.code().unwrap_or(1)))
    }
}

fn absolute_file(path: &Path, component: RuntimeComponent) -> Result<PathBuf, OrbitError> {
    if !path.is_absolute() {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::ComponentPathNotAbsolute {
                component,
                path: path.display().to_string(),
            },
        ));
    }
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    Err(OrbitError::RuntimeData(
        RuntimeDataError::ComponentNotFound {
            component,
            path: path.display().to_string(),
        },
    ))
}

fn java_agent_option(
    agent: &Path,
    instance: &Path,
    session: &Path,
    context: &Path,
) -> Result<String, OrbitError> {
    let agent = agent.to_string_lossy();
    if agent.contains('"') {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::AgentPathContainsQuote,
        ));
    }
    let encode = |path: &Path| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.to_string_lossy().as_bytes())
    };
    Ok(format!(
        "-javaagent:\"{agent}\"=root={};session={};context={}",
        encode(instance),
        encode(session),
        encode(context)
    ))
}

fn write_agent_context(instance: &Path) -> Result<PathBuf, OrbitError> {
    const MAX_NESTED_DEPTH: usize = 16;
    const MAX_NESTED_BYTES: u64 = 1024 * 1024 * 1024;

    let manifest = crate::workspace::ManifestFile::open(instance)?.inner;
    let loader = manifest.project.loader_kind()?;
    let capabilities = capabilities_for(
        loader,
        &manifest.project.mc_version,
        &manifest.project.modloader_version,
    )?;
    let lock = crate::workspace::Lockfile::open(instance)?.inner;
    let mut sources: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut modules: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut package_owners: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut nested_bytes = 0_u64;
    for package in &lock.packages {
        register_package_identities(package, &package.sha256, &mut package_owners);
        let artifact = instance.join("mods").join(&package.filename);
        if !artifact.is_file() {
            continue;
        }
        register_source(&mut sources, package.sha256.clone(), &package.sha256);
        register_source(&mut modules, package.mod_id.clone(), &package.sha256);
        collect_module_owners(&package.bundled, &package.sha256, &mut modules);
        let file = std::fs::File::open(&artifact)?;
        let mut archive = zip::ZipArchive::new(file)?;
        collect_nested_sources(
            &mut archive,
            &package.sha256,
            0,
            MAX_NESTED_DEPTH,
            &mut nested_bytes,
            MAX_NESTED_BYTES,
            &mut sources,
        )?;
    }
    let mut delegations = BTreeSet::new();
    for package in &lock.packages {
        collect_package_delegations(
            &package.sha256,
            &package.dependencies,
            &package.bundled,
            &package_owners,
            &mut delegations,
        );
    }

    let mut document = String::from("3\tcontext\tend\n");
    document.push_str(&format!(
        "capability\tjava\t{}-{}\tend\n",
        capabilities.java_range[0], capabilities.java_range[1]
    ));
    for source in capabilities.code_sources {
        document.push_str(&format!("capability\tsource\t{}\tend\n", source.as_str()));
    }
    if let Some(identity) = capabilities.module_identity {
        document.push_str(&format!("capability\tmodule\t{}\tend\n", identity.as_str()));
    }
    if let Some(property) = capabilities.system_library_property {
        document.push_str(&format!("system-library\t{property}\tend\n"));
    }
    for (source, owner) in sources {
        if let Some(owner) = owner {
            document.push_str(&format!("source\t{source}\t{owner}\tend\n"));
        }
    }
    for (module, owner) in modules {
        if let Some(owner) = owner {
            document.push_str(&format!(
                "module\t{}\t{}\tend\n",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(module.as_bytes()),
                owner
            ));
        }
    }
    for (caller, library) in delegations {
        document.push_str(&format!("delegation\t{caller}\t{library}\tend\n"));
    }
    for (path, kind, owner) in ownership_context(instance)? {
        document.push_str(&format!(
            "node\t{}\t{}\t{}\tend\n",
            match kind {
                crate::runtime_data::OwnedDataKind::File => "file",
                crate::runtime_data::OwnedDataKind::Tree => "tree",
            },
            owner.as_deref().unwrap_or("-"),
            encode_agent_path(&path)
        ));
    }
    for root in RESERVED_INSTANCE_ROOTS {
        document.push_str(&format!(
            "reserved\t{}\tend\n",
            encode_agent_path(&instance.join(root))
        ));
    }
    let path = instance.join(".orbit/runtime-data/agent-context.tsv");
    crate::atomic_io::write_atomic(&path, document.as_bytes())?;
    Ok(path)
}

fn register_package_identities(
    package: &crate::lockfile::PackageEntry,
    owner: &str,
    identities: &mut BTreeMap<String, Option<String>>,
) {
    register_source(identities, package.mod_id.clone(), owner);
    for provided in &package.provides {
        register_source(identities, provided.id.clone(), owner);
    }
    register_bundled_identities(&package.bundled, owner, identities);
}

fn register_bundled_identities(
    bundled: &[crate::lockfile::BundledMod],
    owner: &str,
    identities: &mut BTreeMap<String, Option<String>>,
) {
    for module in bundled {
        register_source(identities, module.mod_id.clone(), owner);
        for provided in &module.provides {
            register_source(identities, provided.id.clone(), owner);
        }
        register_bundled_identities(&module.bundled, owner, identities);
    }
}

fn collect_package_delegations(
    caller: &str,
    dependencies: &[crate::metadata::DependencyExpression],
    bundled: &[crate::lockfile::BundledMod],
    identities: &BTreeMap<String, Option<String>>,
    output: &mut BTreeSet<(String, String)>,
) {
    collect_dependency_delegations(caller, dependencies, identities, output);
    collect_bundled_delegations(caller, bundled, identities, output);
}

fn collect_bundled_delegations(
    caller: &str,
    bundled: &[crate::lockfile::BundledMod],
    identities: &BTreeMap<String, Option<String>>,
    output: &mut BTreeSet<(String, String)>,
) {
    for module in bundled {
        collect_dependency_delegations(caller, &module.dependencies, identities, output);
        collect_bundled_delegations(caller, &module.bundled, identities, output);
    }
}

fn collect_dependency_delegations(
    caller: &str,
    dependencies: &[crate::metadata::DependencyExpression],
    identities: &BTreeMap<String, Option<String>>,
    output: &mut BTreeSet<(String, String)>,
) {
    use crate::metadata::DependencyKind;

    for relation in dependencies
        .iter()
        .flat_map(|expression| expression.relations())
    {
        if matches!(
            relation.kind,
            DependencyKind::Incompatible | DependencyKind::Discouraged
        ) {
            continue;
        }
        let Some(Some(library)) = identities.get(&relation.id) else {
            continue;
        };
        if library != caller {
            output.insert((caller.to_string(), library.clone()));
        }
    }
}

fn collect_module_owners(
    bundled: &[crate::lockfile::BundledMod],
    owner: &str,
    modules: &mut BTreeMap<String, Option<String>>,
) {
    for module in bundled {
        register_source(modules, module.mod_id.clone(), owner);
        collect_module_owners(&module.bundled, owner, modules);
    }
}

fn register_source(sources: &mut BTreeMap<String, Option<String>>, source: String, owner: &str) {
    match sources.entry(source) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(Some(owner.to_string()));
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            if entry.get().as_deref() != Some(owner) {
                entry.insert(None);
            }
        }
    }
}

fn collect_nested_sources<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    owner: &str,
    depth: usize,
    max_depth: usize,
    total_bytes: &mut u64,
    max_bytes: u64,
    sources: &mut BTreeMap<String, Option<String>>,
) -> Result<(), OrbitError> {
    if depth >= max_depth {
        return Ok(());
    }
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() || !entry.name().to_ascii_lowercase().ends_with(".jar") {
            continue;
        }
        *total_bytes = total_bytes.checked_add(entry.size()).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!("nested runtime source size overflowed"))
        })?;
        if *total_bytes > max_bytes {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "nested runtime sources exceed the 1 GiB launch safety limit"
            )));
        }
        let mut bytes = Vec::with_capacity(entry.size().try_into().unwrap_or(0));
        entry.read_to_end(&mut bytes)?;
        register_source(sources, crate::jar::sha256_digest(&bytes), owner);
        if let Ok(mut nested) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
            collect_nested_sources(
                &mut nested,
                owner,
                depth + 1,
                max_depth,
                total_bytes,
                max_bytes,
                sources,
            )?;
        }
    }
    Ok(())
}

fn encode_agent_path(path: &Path) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.to_string_lossy().as_bytes())
}

fn append_java_tool_option(
    existing: Option<&std::ffi::OsStr>,
    agent_option: &str,
) -> Result<std::ffi::OsString, OrbitError> {
    let mut value = existing.map(std::ffi::OsString::from).unwrap_or_default();
    if value
        .to_string_lossy()
        .contains("dev.orbit.agent.OrbitRuntimeAgent")
        || value.to_string_lossy().contains("orbit-runtime-agent")
    {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::AgentAlreadyPresent,
        ));
    }
    if !value.is_empty() {
        value.push(" ");
    }
    value.push(agent_option);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_existing_java_tool_options() {
        let combined = append_java_tool_option(
            Some(std::ffi::OsStr::new("-Xmx2G")),
            "-javaagent:agent.jar=example",
        )
        .unwrap();
        assert_eq!(combined, "-Xmx2G -javaagent:agent.jar=example");
    }

    #[test]
    fn quotes_agent_paths_and_encodes_runtime_paths() {
        let option = java_agent_option(
            Path::new("C:/Program Files/Orbit/orbit-runtime-agent.jar"),
            Path::new("C:/Games/Example"),
            Path::new("C:/Games/Example/.orbit/session.events"),
            Path::new("C:/Games/Example/.orbit/agent-context.tsv"),
        )
        .unwrap();
        assert!(option.starts_with("-javaagent:\"C:/Program Files/Orbit/"));
        assert!(!option.contains("C:/Games/Example"));
    }

    #[test]
    fn rejects_relative_components_before_switching_to_the_instance_directory() {
        assert!(absolute_file(Path::new("component"), RuntimeComponent::Agent).is_err());
    }

    #[test]
    fn ambiguous_runtime_identity_is_removed_instead_of_guessed() {
        let mut identities = BTreeMap::new();
        register_source(&mut identities, "shared-id".into(), "owner-a");
        register_source(&mut identities, "shared-id".into(), "owner-a");
        assert_eq!(identities["shared-id"].as_deref(), Some("owner-a"));

        register_source(&mut identities, "shared-id".into(), "owner-b");
        assert_eq!(identities["shared-id"], None);
    }

    #[test]
    fn declared_library_edges_drive_delegated_writer_attribution() {
        use crate::metadata::{DependencyExpression, DependencyKind, ModDependency};

        let caller = "a".repeat(64);
        let library = "b".repeat(64);
        let blocked = "c".repeat(64);
        let identities = BTreeMap::from([
            ("library".to_string(), Some(library.clone())),
            ("blocked".to_string(), Some(blocked.clone())),
        ]);
        let mut incompatible = ModDependency::required("blocked", "*");
        incompatible.kind = DependencyKind::Incompatible;
        let dependencies = vec![
            DependencyExpression::from(ModDependency::required("library", "*")),
            DependencyExpression::from(incompatible),
        ];
        let mut output = BTreeSet::new();
        collect_dependency_delegations(&caller, &dependencies, &identities, &mut output);

        assert_eq!(output, BTreeSet::from([(caller, library)]));
    }
}
