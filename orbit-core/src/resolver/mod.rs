//! Dependency resolution orchestration and public resolver utilities.

mod diagnostics;
mod graph;
mod local;
pub mod provider;
mod retry;
pub mod types;

use std::collections::HashMap;

use crate::lockfile::{OrbitLockfile, PackageEntry};
use crate::manifest::OrbitManifest;
use crate::providers::ModProvider;
use crate::resolver::graph::build_solver_graph;
use crate::resolver::retry::{SolveOutcome, SolveRequest, solve_with_fetch_retry};
use crate::resolver::types::{CandidateVersion, ResolutionReport};

pub use provider::ProviderError as FetchRetryError;

pub fn check_local_graph(
    manifest: &OrbitManifest,
    local_mods: &[crate::identification::IdentifiedMod],
) -> Result<(), String> {
    local::check_local_graph(manifest, local_mods)
}

pub fn dependents<'a>(slug: &str, entries: &'a [PackageEntry]) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|entry| {
            entry
                .dependencies
                .iter()
                .any(|dependency| dependency.name == slug)
        })
        .map(|entry| entry.mod_id.as_str())
        .collect()
}

pub fn find_entry<'a>(slug: &str, entries: &'a [PackageEntry]) -> Option<&'a PackageEntry> {
    entries.iter().find(|entry| {
        entry.mod_id == slug
            || entry
                .modrinth
                .as_ref()
                .map(|modrinth| modrinth.slug.as_str())
                == Some(slug)
    })
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
    })
    .await?;

    Ok(collect_report(lockfile, candidates, outcome))
}

fn collect_report(
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    outcome: SolveOutcome,
) -> ResolutionReport {
    let SolveOutcome { solution, trace } = outcome;

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
    }
}
