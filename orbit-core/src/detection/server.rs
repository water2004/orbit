//! Exact dedicated-server runtime discovery.
//!
//! The orchestrator collects typed candidates from each supported local
//! launch format. Format parsers live in `server/formats.rs`; consumers receive
//! one normalized runtime or an explicit ambiguity error.

mod formats;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::loader::LoaderKind;
use crate::metadata::mojang::McVersion;

#[derive(Debug, Clone)]
pub(crate) struct ServerRuntimeSpec {
    pub loader: LoaderKind,
    pub loader_version: String,
    pub minecraft: McVersion,
    pub minecraft_jar: PathBuf,
    pub loader_jar: PathBuf,
    pub runtime_jars: Vec<PathBuf>,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ServerLaunchFormat {
    FabricInstallerBootstrap,
    DirectLoaderLaunchJar,
    ForgeBootstrapShim,
    ModLauncherArgumentFile,
}

impl std::fmt::Display for ServerLaunchFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::FabricInstallerBootstrap => "Fabric installer bootstrap",
            Self::DirectLoaderLaunchJar => "direct loader launch JAR",
            Self::ForgeBootstrapShim => "Forge bootstrap shim",
            Self::ModLauncherArgumentFile => "ModLauncher argument file",
        })
    }
}

#[derive(Debug)]
struct ServerRuntimeCandidate {
    spec: ServerRuntimeSpec,
    formats: BTreeSet<ServerLaunchFormat>,
}

/// Reads every supported, installed dedicated-server launch format and returns
/// one unambiguous runtime. No provider metadata or network access is used.
pub(crate) fn discover_server_runtime(
    instance_dir: &Path,
) -> Result<Option<ServerRuntimeSpec>, OrbitError> {
    if !crate::launcher::is_dedicated_server(instance_dir) {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    collect(
        &mut candidates,
        ServerLaunchFormat::FabricInstallerBootstrap,
        formats::discover_fabric_bootstraps(instance_dir)?,
    );
    collect(
        &mut candidates,
        ServerLaunchFormat::DirectLoaderLaunchJar,
        formats::discover_direct_launch_jars(instance_dir)?,
    );
    collect(
        &mut candidates,
        ServerLaunchFormat::ForgeBootstrapShim,
        formats::discover_forge_shims(instance_dir)?,
    );
    collect(
        &mut candidates,
        ServerLaunchFormat::ModLauncherArgumentFile,
        formats::discover_modlauncher_argfiles(instance_dir)?,
    );
    merge_equivalent_candidates(&mut candidates);

    match candidates.len() {
        0 => Err(other(format!(
            "dedicated-server directory '{}' contains no complete supported loader runtime; \
             install Fabric, Quilt, Forge, or NeoForge with its official server installer first",
            instance_dir.display()
        ))),
        1 => Ok(candidates.pop().map(|candidate| candidate.spec)),
        _ => Err(other(format!(
            "multiple installed dedicated-server runtimes are active candidates: {}",
            candidates
                .iter()
                .map(|candidate| format!(
                    "{} {} for Minecraft {} via {} ({})",
                    candidate.spec.loader,
                    candidate.spec.loader_version,
                    candidate.spec.minecraft.id,
                    candidate
                        .formats
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" + "),
                    candidate.spec.evidence
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ))),
    }
}

fn collect(
    candidates: &mut Vec<ServerRuntimeCandidate>,
    format: ServerLaunchFormat,
    specs: Vec<ServerRuntimeSpec>,
) {
    candidates.extend(specs.into_iter().map(|spec| ServerRuntimeCandidate {
        spec,
        formats: BTreeSet::from([format]),
    }));
}

fn merge_equivalent_candidates(candidates: &mut Vec<ServerRuntimeCandidate>) {
    let mut merged = Vec::<ServerRuntimeCandidate>::new();
    for mut candidate in candidates.drain(..) {
        if let Some(existing) = merged.iter_mut().find(|existing| {
            existing.spec.loader == candidate.spec.loader
                && existing.spec.loader_version == candidate.spec.loader_version
                && existing.spec.minecraft.id == candidate.spec.minecraft.id
                && existing.spec.minecraft_jar == candidate.spec.minecraft_jar
                && existing.spec.loader_jar == candidate.spec.loader_jar
        }) {
            existing
                .spec
                .runtime_jars
                .append(&mut candidate.spec.runtime_jars);
            existing.spec.runtime_jars.sort();
            existing.spec.runtime_jars.dedup();
            existing.formats.append(&mut candidate.formats);
            if !existing.spec.evidence.contains(&candidate.spec.evidence) {
                existing.spec.evidence.push_str(" + ");
                existing.spec.evidence.push_str(&candidate.spec.evidence);
            }
        } else {
            merged.push(candidate);
        }
    }
    merged.sort_by(|left, right| {
        left.spec
            .loader
            .cmp(&right.spec.loader)
            .then_with(|| left.spec.loader_version.cmp(&right.spec.loader_version))
            .then_with(|| left.spec.minecraft.id.cmp(&right.spec.minecraft.id))
            .then_with(|| left.spec.loader_jar.cmp(&right.spec.loader_jar))
    });
    *candidates = merged;
}

fn other(message: impl Into<String>) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(message.into()))
}
