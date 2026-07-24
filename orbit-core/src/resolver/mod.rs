//! Dependency resolution orchestration and public resolver utilities.

mod constraints;
mod diagnostics;
mod graph;
mod local;
mod ordering;
pub mod provider;
mod retry;
pub mod types;

use std::collections::HashMap;

use crate::lockfile::{OrbitLockfile, PackageEntry};
use crate::manifest::OrbitManifest;
use crate::metadata::Environment;
use crate::providers::ModProvider;
use crate::resolver::graph::{build_solver_graph, build_solver_graph_for_target};
use crate::resolver::ordering::resolution_warnings;
use crate::resolver::retry::{SolveOutcome, SolveRequest, solve_with_fetch_retry};
use crate::resolver::types::{CandidateVersion, ResolutionReport};

pub(crate) use graph::is_builtin_package;
pub use provider::ProviderError as FetchRetryError;

pub fn check_local_graph(
    manifest: &OrbitManifest,
    local_mods: &[crate::identification::IdentifiedMod],
) -> Result<(), String> {
    local::check_local_graph(manifest, local_mods)
}

pub fn check_lockfile_graph(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
) -> Result<(), String> {
    let graph = build_solver_graph(manifest, lockfile, &HashMap::new());
    match pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version) {
        Ok(_) => Ok(()),
        Err(pubgrub::PubGrubError::NoSolution(derivation_tree)) => {
            Err(diagnostics::describe_no_solution(&derivation_tree))
        }
        Err(pubgrub::PubGrubError::ErrorChoosingVersion { source, .. })
        | Err(pubgrub::PubGrubError::ErrorRetrievingDependencies { source, .. }) => {
            Err(format!("internal resolver error: {source}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn resolve_lockfile_for_target(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    target: Environment,
) -> Result<pubgrub::SelectedDependencies<String, crate::versions::Version>, String> {
    let graph = build_solver_graph_for_target(manifest, lockfile, &HashMap::new(), target);
    match pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version) {
        Ok(solution) => Ok(solution),
        Err(pubgrub::PubGrubError::NoSolution(derivation_tree)) => {
            Err(diagnostics::describe_no_solution(&derivation_tree))
        }
        Err(pubgrub::PubGrubError::ErrorChoosingVersion { source, .. })
        | Err(pubgrub::PubGrubError::ErrorRetrievingDependencies { source, .. }) => {
            Err(format!("internal resolver error: {source}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

pub fn dependents<'a>(slug: &str, entries: &'a [PackageEntry]) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|entry| {
            entry
                .dependencies
                .iter()
                .flat_map(|dependency| dependency.relations())
                .any(|dependency| dependency.kind.installs_target() && dependency.id == slug)
        })
        .map(|entry| entry.mod_id.as_str())
        .collect()
}

pub fn find_entry<'a>(slug: &str, entries: &'a [PackageEntry]) -> Option<&'a PackageEntry> {
    entries
        .iter()
        .find(|entry| entry.mod_id == slug || entry.source_slug() == Some(slug))
}

pub fn check_version_conflict(
    slug: &str,
    new_version: &str,
    entries: &[PackageEntry],
) -> Result<(), String> {
    if let Some(entry) = find_entry(slug, entries)
        && entry.version != new_version
    {
        return Err(format!(
            "'{}' version conflict: lock has '{}', resolved '{}'",
            entry.mod_id, entry.version, new_version
        ));
    }
    Ok(())
}

/// Resolve candidate versions, fetching missing transitive dependencies between attempts.
///
/// Candidate lists are extended when missing dependency metadata is downloaded.
pub async fn resolve_with_candidates(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    candidates: &mut HashMap<String, Vec<CandidateVersion>>,
    providers: &[Box<dyn ModProvider>],
) -> Result<HashMap<String, String>, String> {
    Ok(
        resolve_with_candidates_report(manifest, lockfile, candidates, providers)
            .await?
            .upgrades,
    )
}

/// Resolve candidates and retain structured explanations for skipped versions.
pub async fn resolve_with_candidates_report(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    candidates: &mut HashMap<String, Vec<CandidateVersion>>,
    providers: &[Box<dyn ModProvider>],
) -> Result<ResolutionReport, String> {
    let graph = build_solver_graph(manifest, lockfile, candidates);
    let mut provider = graph.provider;
    let outcome = solve_with_fetch_retry(SolveRequest {
        provider: &mut provider,
        root_package: &graph.root_package,
        root_version: &graph.root_version,
        candidates,
        lockfile,
        providers,
        minecraft_version: &manifest.project.mc_version,
        loader: &manifest.project.modloader,
        exclusions: &graph.exclusions,
        overrides: &graph.overrides,
        target: graph.target,
    })
    .await?;

    Ok(collect_report(
        lockfile,
        candidates,
        outcome,
        &manifest.project.modloader,
        &graph.exclusions,
        &graph.overrides,
        graph.target,
    ))
}

fn collect_report(
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    outcome: SolveOutcome,
    loader: &str,
    exclusions: &graph::ExclusionMap,
    overrides: &graph::OverrideMap,
    target: Environment,
) -> ResolutionReport {
    let SolveOutcome { solution, trace } = outcome;
    let warnings = resolution_warnings(
        lockfile, candidates, &solution, loader, exclusions, overrides, target,
    );

    let mut upgrades = HashMap::new();
    let mut diagnostics = Vec::new();
    for package in candidates.keys() {
        let Some(selected) = solution.get(package) else {
            continue;
        };
        let current = lockfile
            .find(package)
            .map(|entry| entry.version.as_str())
            .unwrap_or("?");
        let selected_version = selected.to_string();
        if selected_version != current {
            upgrades.insert(package.clone(), selected_version);
            continue;
        }

        let Some(candidate) = candidates
            .get(package)
            .and_then(|versions| versions.first())
        else {
            continue;
        };
        if candidate.jar_version != current {
            diagnostics.push(trace.diagnose_skipped(package, selected));
        }
    }
    ResolutionReport {
        upgrades,
        diagnostics,
        warnings,
    }
}
