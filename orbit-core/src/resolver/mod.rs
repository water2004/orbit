//! Dependency resolution orchestration and public resolver utilities.

mod constraints;
mod diagnostics;
mod graph;
mod local;
mod ordering;
mod provider;
pub mod types;

use std::collections::{BTreeMap, HashMap};

use pubgrub::Ranges;

use crate::loader::LoaderKind;
use crate::lockfile::{OrbitLockfile, PackageEntry};
use crate::manifest::OrbitManifest;
use crate::metadata::Environment;
use crate::progress::ProgressReporter;
use crate::resolver::graph::{
    ManifestPackageRoots, build_solver_graph, build_solver_graph_for_target,
    build_solver_graph_with_package_roots, manifest_package_versions,
};
use crate::resolver::ordering::resolution_warnings;
use crate::resolver::types::{
    CandidateCatalog, CandidateIdentity, CandidateLocation, CandidateVersion, PackageChange,
    PackageChangeKind, ResolutionPortfolio, ResolutionReport, ResolutionSelector, SolverPackage,
    SolverVersion, solver_range,
};
use crate::versions::Version;

pub(crate) use graph::locked_source;

pub(crate) fn select_resolution(
    mut portfolio: ResolutionPortfolio,
    selector: Option<ResolutionSelector>,
) -> Result<ResolutionReport, String> {
    if portfolio.alternatives.is_empty() {
        return Err("internal error: dependency solver returned no alternatives".to_string());
    }
    let index = if portfolio.alternatives.len() == 1 {
        0
    } else {
        match selector {
            Some(select) => select(&portfolio.alternatives)?,
            None => 0,
        }
    };
    if index >= portfolio.alternatives.len() {
        return Err(format!(
            "dependency solution selector returned invalid choice {} for {} alternatives",
            index + 1,
            portfolio.alternatives.len()
        ));
    }
    Ok(portfolio.alternatives.remove(index))
}

/// Select a Pareto-maximal solution that performs an upgrade.
///
/// When `package` is set, upgrading some unrelated package is not sufficient:
/// the requested logical package itself must move to a newer version. If no
/// eligible solution exists, the returned empty report retains the solver's
/// explanations for newer candidates instead of silently discarding them.
pub(crate) fn select_upgrade_resolution(
    mut portfolio: ResolutionPortfolio,
    package: Option<&str>,
    selector: Option<ResolutionSelector>,
) -> Result<ResolutionReport, String> {
    let diagnostics = aggregate_candidate_diagnostics(&portfolio.alternatives, package);
    portfolio.alternatives.retain(|alternative| {
        alternative.changes.iter().any(|change| {
            change.kind == PackageChangeKind::Upgrade
                && package.is_none_or(|package| change.package == package)
        })
    });
    if portfolio.alternatives.is_empty() {
        return Ok(ResolutionReport {
            diagnostics,
            ..ResolutionReport::default()
        });
    }
    select_resolution(portfolio, selector)
}

fn aggregate_candidate_diagnostics(
    alternatives: &[ResolutionReport],
    package: Option<&str>,
) -> Vec<types::CandidateDiagnostic> {
    let diagnostics = alternatives
        .iter()
        .flat_map(|alternative| alternative.diagnostics.iter().cloned())
        .filter(|diagnostic| package.is_none_or(|package| diagnostic.package == package))
        .collect();
    normalize_candidate_diagnostics(diagnostics)
}

pub(crate) fn normalize_candidate_diagnostics(
    mut diagnostics: Vec<types::CandidateDiagnostic>,
) -> Vec<types::CandidateDiagnostic> {
    diagnostics.sort_by(|left, right| {
        left.package
            .cmp(&right.package)
            .then_with(|| left.candidate_version.cmp(&right.candidate_version))
            .then_with(|| left.selected_version.cmp(&right.selected_version))
            .then_with(|| left.facts.cmp(&right.facts))
    });
    diagnostics.dedup();
    diagnostics
}

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
    check_lockfile_graph_with_loader(manifest, lockfile, None)
}

pub(crate) fn check_lockfile_graph_with_loader(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    loader_package: Option<&types::PlatformCandidate>,
) -> Result<(), String> {
    let graph = build_solver_graph(manifest, lockfile, &HashMap::new(), loader_package)?;
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
    loader_package: Option<&types::PlatformCandidate>,
) -> Result<pubgrub::SelectedDependencies<SolverPackage, SolverVersion>, String> {
    let graph =
        build_solver_graph_for_target(manifest, lockfile, &HashMap::new(), loader_package, target)?;
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

#[derive(Debug, Default)]
pub(crate) struct RuntimeLoadSelection {
    /// JAR-declared logical Mod IDs selected for the current physical side.
    pub active_mod_ids: std::collections::BTreeSet<String>,
    /// Top-level package files which carry at least one selected module.
    pub top_level_jars: std::collections::BTreeSet<String>,
    /// Active nested archive paths keyed by their top-level package filename.
    pub nested_jars: HashMap<String, std::collections::BTreeSet<String>>,
    /// Active nested archive paths carried by the actual Loader JAR.
    pub loader_nested_jars: std::collections::BTreeSet<String>,
}

/// Returns the runtime content selected by the same graph that validates the
/// installed package set for the current physical side.
///
/// This is runtime classpath construction, not compatibility evidence:
/// `orbit audit` must not parse inactive versions in a loader-managed
/// multi-version JAR.
pub(crate) fn selected_runtime_load(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    loader_package: Option<&types::PlatformCandidate>,
    target: Environment,
) -> Result<RuntimeLoadSelection, String> {
    let solution = resolve_lockfile_for_target(manifest, lockfile, target, loader_package)?;
    let mut selected = RuntimeLoadSelection::default();
    if let Some(loader_package) = loader_package {
        selected
            .active_mod_ids
            .insert(loader_package.mod_id.clone());
        selected.active_mod_ids.extend(
            loader_package
                .provides
                .iter()
                .map(|provided| provided.id.clone()),
        );
    }
    for (package, version) in solution.iter() {
        if let SolverPackage::Mod(mod_id) = package
            && version.candidate_identity().is_some()
        {
            selected.active_mod_ids.insert(mod_id.clone());
        }
        let Some(identity) = version.candidate_identity() else {
            continue;
        };
        if let Some(loader_package) = loader_package.filter(|loader_package| {
            identity.owner == loader_package.mod_id
                && identity.source
                    == format!(
                        "platform:{}:{}",
                        loader_package.mod_id, loader_package.version
                    )
        }) {
            if let Some(bundled) = bundled_at_path(&loader_package.bundled, &identity.path) {
                selected.active_mod_ids.insert(bundled.mod_id.clone());
                selected
                    .active_mod_ids
                    .extend(bundled.provides.iter().map(|provided| provided.id.clone()));
            }
            if identity.location == CandidateLocation::Nested
                && let Some(path) =
                    nested_archive_path_from(&loader_package.bundled, &identity.path)
            {
                selected.loader_nested_jars.insert(path);
            }
            if let SolverPackage::EmbeddedArtifact(artifact_id) = package
                && let Some(selected_version) = version.domain()
                && let Some((prefix, artifacts)) = embedded_artifacts_at_path_from(
                    &loader_package.embedded_artifacts,
                    &loader_package.bundled,
                    &identity.path,
                )
                && let Some(artifact) = artifacts.iter().find(|artifact| {
                    artifact.id == *artifact_id
                        && Version::parse(&artifact.version, LoaderKind::Forge) == *selected_version
                })
            {
                selected.loader_nested_jars.insert(if prefix.is_empty() {
                    artifact.path.clone()
                } else {
                    format!("{prefix}!/{}", artifact.path)
                });
            }
            continue;
        }
        let Some(entry) = lockfile.packages.iter().find(|entry| {
            entry.mod_id == identity.owner && locked_source(entry) == identity.source
        }) else {
            continue;
        };
        if !entry.filename.is_empty() {
            selected.top_level_jars.insert(entry.filename.clone());
        }
        if identity.path.is_empty() {
            selected.active_mod_ids.insert(entry.mod_id.clone());
            selected
                .active_mod_ids
                .extend(entry.provides.iter().map(|provided| provided.id.clone()));
        } else if let Some(bundled) = bundled_at_path(&entry.bundled, &identity.path) {
            selected.active_mod_ids.insert(bundled.mod_id.clone());
            selected
                .active_mod_ids
                .extend(bundled.provides.iter().map(|provided| provided.id.clone()));
        }
        if identity.location == CandidateLocation::Nested
            && let Some(path) = nested_archive_path(entry, &identity.path)
        {
            selected
                .nested_jars
                .entry(entry.filename.clone())
                .or_default()
                .insert(path);
        }
        if let SolverPackage::EmbeddedArtifact(artifact_id) = package
            && let Some(selected_version) = version.domain()
            && let Some((prefix, artifacts)) = embedded_artifacts_at_path(entry, &identity.path)
            && let Some(artifact) = artifacts.iter().find(|artifact| {
                artifact.id == *artifact_id
                    && Version::parse(&artifact.version, LoaderKind::Forge) == *selected_version
            })
        {
            let path = if prefix.is_empty() {
                artifact.path.clone()
            } else {
                format!("{prefix}!/{}", artifact.path)
            };
            selected
                .nested_jars
                .entry(entry.filename.clone())
                .or_default()
                .insert(path);
        }
    }
    Ok(selected)
}

trait RuntimeBundledNode: Sized {
    fn origin(&self) -> &crate::jar::JarModOrigin;
    fn embedded_artifacts(&self) -> &[crate::metadata::EmbeddedArtifact];
    fn bundled(&self) -> &[Self];
}

impl RuntimeBundledNode for crate::lockfile::BundledMod {
    fn origin(&self) -> &crate::jar::JarModOrigin {
        &self.origin
    }

    fn embedded_artifacts(&self) -> &[crate::metadata::EmbeddedArtifact] {
        &self.embedded_artifacts
    }

    fn bundled(&self) -> &[Self] {
        &self.bundled
    }
}

impl RuntimeBundledNode for types::BundledCandidate {
    fn origin(&self) -> &crate::jar::JarModOrigin {
        &self.origin
    }

    fn embedded_artifacts(&self) -> &[crate::metadata::EmbeddedArtifact] {
        &self.embedded_artifacts
    }

    fn bundled(&self) -> &[Self] {
        &self.bundled
    }
}

fn bundled_at_path<'a, T: RuntimeBundledNode>(roots: &'a [T], path: &[usize]) -> Option<&'a T> {
    let mut bundled = roots;
    let mut selected = None;
    for index in path {
        let node = bundled.get(*index)?;
        selected = Some(node);
        bundled = node.bundled();
    }
    selected
}

fn nested_archive_path(entry: &PackageEntry, path: &[usize]) -> Option<String> {
    nested_archive_path_from(&entry.bundled, path)
}

fn nested_archive_path_from<T: RuntimeBundledNode>(roots: &[T], path: &[usize]) -> Option<String> {
    let mut bundled = roots;
    let mut archives = Vec::new();
    for index in path {
        let selected = bundled.get(*index)?;
        if let crate::jar::JarModOrigin::Nested { path, .. } = selected.origin() {
            archives.push(path.clone());
        }
        bundled = selected.bundled();
    }
    (!archives.is_empty()).then(|| archives.join("!/"))
}

fn embedded_artifacts_at_path<'a>(
    entry: &'a PackageEntry,
    path: &[usize],
) -> Option<(String, &'a [crate::metadata::EmbeddedArtifact])> {
    embedded_artifacts_at_path_from(&entry.embedded_artifacts, &entry.bundled, path)
}

fn embedded_artifacts_at_path_from<'a, T: RuntimeBundledNode>(
    root_artifacts: &'a [crate::metadata::EmbeddedArtifact],
    roots: &'a [T],
    path: &[usize],
) -> Option<(String, &'a [crate::metadata::EmbeddedArtifact])> {
    if path.is_empty() {
        return Some((String::new(), root_artifacts));
    }
    let mut bundled = roots;
    let mut archives = Vec::new();
    let mut selected = None;
    for index in path {
        let node = bundled.get(*index)?;
        if let crate::jar::JarModOrigin::Nested { path, .. } = node.origin() {
            archives.push(path.clone());
        }
        selected = Some(node);
        bundled = node.bundled();
    }
    selected.map(|node| (archives.join("!/"), node.embedded_artifacts()))
}

pub fn dependents<'a>(package: &str, entries: &'a [PackageEntry]) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|entry| {
            entry
                .dependencies
                .iter()
                .flat_map(|dependency| dependency.relations())
                .any(|dependency| dependency.kind.installs_target() && dependency.id == package)
        })
        .map(|entry| entry.mod_id.as_str())
        .collect()
}

pub fn find_entry<'a>(package: &str, entries: &'a [PackageEntry]) -> Option<&'a PackageEntry> {
    entries.iter().find(|entry| entry.mod_id == package)
}

pub fn check_version_conflict(
    package: &str,
    new_version: &str,
    entries: &[PackageEntry],
) -> Result<(), String> {
    if let Some(entry) = find_entry(package, entries)
        && entry.version != new_version
    {
        return Err(format!(
            "'{}' version conflict: lock has '{}', resolved '{}'",
            entry.mod_id, entry.version, new_version
        ));
    }
    Ok(())
}

/// Complete the candidate graph and enumerate its package-version Pareto front.
pub async fn resolve_candidate_portfolio(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
) -> Result<ResolutionPortfolio, String> {
    resolve_candidate_portfolio_with_progress(manifest, lockfile, catalog, None).await
}

pub async fn resolve_minimal_change_portfolio(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
) -> Result<ResolutionPortfolio, String> {
    resolve_minimal_change_portfolio_with_progress(manifest, lockfile, catalog, None).await
}

pub async fn resolve_candidate_portfolio_with_progress(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
    progress: Option<ProgressReporter>,
) -> Result<ResolutionPortfolio, String> {
    resolve_portfolio_with_progress_detailed(
        manifest,
        lockfile,
        catalog,
        ResolutionObjective::MaximizeVersions,
        progress,
    )
    .await
    .map_err(|error| error.to_string())
}

pub async fn resolve_minimal_change_portfolio_with_progress(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
    progress: Option<ProgressReporter>,
) -> Result<ResolutionPortfolio, String> {
    resolve_portfolio_with_progress_detailed(
        manifest,
        lockfile,
        catalog,
        ResolutionObjective::MinimizeChanges,
        progress,
    )
    .await
    .map_err(|error| error.to_string())
}

#[derive(Debug)]
pub(crate) enum ResolutionFailure {
    NoSolution(String),
    Internal(String),
}

impl std::fmt::Display for ResolutionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSolution(message) | Self::Internal(message) => formatter.write_str(message),
        }
    }
}

pub(crate) async fn resolve_required_package_portfolio_with_progress(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
    progress: Option<ProgressReporter>,
) -> Result<ResolutionPortfolio, ResolutionFailure> {
    resolve_portfolio_with_progress_detailed(
        manifest,
        lockfile,
        catalog,
        ResolutionObjective::RequireManifestPackages,
        progress,
    )
    .await
}

pub(crate) async fn resolve_package_preserving_portfolio_with_progress(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
    progress: Option<ProgressReporter>,
) -> Result<ResolutionPortfolio, ResolutionFailure> {
    resolve_portfolio_with_progress_detailed(
        manifest,
        lockfile,
        catalog,
        ResolutionObjective::PreserveManifestPackages,
        progress,
    )
    .await
}

#[derive(Clone, Copy)]
enum ResolutionObjective {
    MaximizeVersions,
    MinimizeChanges,
    RequireManifestPackages,
    PreserveManifestPackages,
}

async fn resolve_portfolio_with_progress_detailed(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &CandidateCatalog,
    objective: ResolutionObjective,
    progress: Option<ProgressReporter>,
) -> Result<ResolutionPortfolio, ResolutionFailure> {
    let graph = match objective {
        ResolutionObjective::RequireManifestPackages => build_solver_graph_with_package_roots(
            manifest,
            lockfile,
            &catalog.candidates,
            catalog.loader_package.as_ref(),
            Environment::Both,
            ManifestPackageRoots::RequiredTopLevel,
        ),
        ResolutionObjective::PreserveManifestPackages => build_solver_graph_with_package_roots(
            manifest,
            lockfile,
            &catalog.candidates,
            catalog.loader_package.as_ref(),
            Environment::Both,
            ManifestPackageRoots::Preferred,
        ),
        ResolutionObjective::MaximizeVersions | ResolutionObjective::MinimizeChanges => {
            build_solver_graph(
                manifest,
                lockfile,
                &catalog.candidates,
                catalog.loader_package.as_ref(),
            )
        }
    }
    .map_err(ResolutionFailure::Internal)?;
    let mut maximized_mods: Vec<_> = catalog
        .candidates
        .keys()
        .chain(manifest.packages.keys())
        .chain(lockfile.packages.iter().map(|entry| &entry.mod_id))
        .cloned()
        .collect();
    maximized_mods.sort();
    maximized_mods.dedup();
    let maximized_packages = maximized_mods
        .iter()
        .cloned()
        .map(SolverPackage::logical)
        .collect::<Vec<_>>();
    let watched_candidates = highest_candidates(&catalog.candidates, graph.loader);
    let mut trace = diagnostics::ResolutionTrace::with_progress(watched_candidates, progress);
    let solved = match objective {
        ResolutionObjective::MaximizeVersions | ResolutionObjective::RequireManifestPackages => {
            pubgrub::resolve_maximal_solutions_with_observer(
                &graph.provider,
                graph.root_package.clone(),
                graph.root_version.clone(),
                maximized_packages,
                solver_version_ordering(),
                &mut trace,
            )
        }
        ResolutionObjective::MinimizeChanges => {
            let preferences =
                minimal_change_preferences(manifest, lockfile, &graph.provider, &maximized_mods);
            pubgrub::resolve_minimal_change_solutions_with_observer(
                &graph.provider,
                graph.root_package.clone(),
                graph.root_version.clone(),
                preferences,
                maximized_packages,
                solver_version_ordering(),
                &mut trace,
            )
        }
        ResolutionObjective::PreserveManifestPackages => {
            let preferences = manifest_package_preferences(manifest, &graph.provider);
            pubgrub::resolve_minimal_change_solutions_with_observer(
                &graph.provider,
                graph.root_package.clone(),
                graph.root_version.clone(),
                preferences,
                maximized_packages,
                solver_version_ordering(),
                &mut trace,
            )
        }
    };
    let solutions = match solved {
        Ok(solutions) => solutions,
        Err(pubgrub::PubGrubError::NoSolution(derivation_tree)) => {
            return Err(ResolutionFailure::NoSolution(
                diagnostics::describe_no_solution(&derivation_tree),
            ));
        }
        Err(pubgrub::PubGrubError::ErrorChoosingVersion { package, source: _ }) => {
            return Err(ResolutionFailure::Internal(format!(
                "internal error: no version of '{package}' matches constraint"
            )));
        }
        Err(pubgrub::PubGrubError::ErrorRetrievingDependencies {
            package,
            version,
            source,
        }) => {
            return Err(ResolutionFailure::Internal(format!(
                "internal error: deps of '{package}' v{version}: {source}"
            )));
        }
        Err(pubgrub::PubGrubError::ErrorInShouldCancel(error)) => {
            return Err(ResolutionFailure::Internal(error.to_string()));
        }
        Err(pubgrub::PubGrubError::InvalidVersionOrdering {
            package,
            version,
            reason,
        }) => {
            return Err(ResolutionFailure::Internal(format!(
                "internal error: invalid version ordering for '{package}' v{version}: {reason}"
            )));
        }
    };
    let snapshots = trace.into_solutions();
    if snapshots.len() != solutions.len() {
        return Err(ResolutionFailure::Internal(
            "internal error: solver trace count does not match solution count".to_string(),
        ));
    }

    let report_context = ReportContext {
        lockfile,
        candidates: &catalog.candidates,
        loader: graph.loader,
        exclusions: &graph.exclusions,
        target: graph.target,
    };
    let alternatives = solutions
        .into_iter()
        .zip(snapshots)
        .map(|(solution, snapshot)| collect_report(&report_context, &solution, &snapshot))
        .collect();
    Ok(ResolutionPortfolio { alternatives })
}

type SolverVersionOrdering = pubgrub::VersionOrdering<
    fn(&SolverVersion) -> Ranges<SolverVersion>,
    fn(&SolverVersion) -> Ranges<SolverVersion>,
    fn(&SolverVersion) -> Ranges<SolverVersion>,
>;

fn solver_version_ordering() -> SolverVersionOrdering {
    pubgrub::VersionOrdering::new(
        |version: &SolverVersion| Ranges::singleton(version.clone()),
        |version: &SolverVersion| {
            version.domain().map_or_else(
                || Ranges::singleton(version.clone()),
                |semantic| solver_range(semantic.precedence_class()),
            )
        },
        |version: &SolverVersion| {
            version.domain().map_or_else(
                || Ranges::strictly_higher_than(version.clone()),
                |semantic| solver_range(semantic.strictly_higher_precedence()),
            )
        },
    )
}

fn minimal_change_preferences(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    provider: &provider::OrbitDependencyProvider,
    packages: &[String],
) -> Vec<pubgrub::PackagePreference<SolverPackage, Ranges<SolverVersion>>> {
    let mut preferences = Vec::new();
    for mod_id in packages {
        let package = SolverPackage::logical(mod_id.clone());
        if !manifest.packages.contains_key(mod_id) {
            preferences.push(pubgrub::PackagePreference::absent(package));
            continue;
        }
        if !lockfile
            .packages
            .iter()
            .any(|entry| entry.mod_id == *mod_id)
        {
            continue;
        }
        let preferred = provider
            .versions
            .get(&package)
            .into_iter()
            .flatten()
            .filter(|version| {
                version.candidate_identity().is_some_and(|identity| {
                    identity.installed
                        && identity.owner == *mod_id
                        && identity.path.is_empty()
                        && lockfile.packages.iter().any(|entry| {
                            entry.mod_id == *mod_id && locked_source(entry) == identity.source
                        })
                })
            })
            .fold(Ranges::empty(), |range, version| {
                range.union(&Ranges::singleton(version.clone()))
            });
        if preferred != Ranges::empty() {
            preferences.push(pubgrub::PackagePreference::selected(package, preferred));
        }
    }
    preferences
}

fn manifest_package_preferences(
    manifest: &OrbitManifest,
    provider: &provider::OrbitDependencyProvider,
) -> Vec<pubgrub::PackagePreference<SolverPackage, Ranges<SolverVersion>>> {
    let loader = manifest
        .project
        .loader_kind()
        .expect("solver graph already validated the manifest Loader");
    manifest
        .packages
        .iter()
        .filter_map(|(mod_id, spec)| {
            let package = SolverPackage::logical(mod_id.clone());
            let preferred = manifest_package_versions(provider, mod_id, spec, loader, true);
            (preferred != Ranges::empty())
                .then(|| pubgrub::PackagePreference::selected(package, preferred))
        })
        .collect()
}

struct ReportContext<'a> {
    lockfile: &'a OrbitLockfile,
    candidates: &'a HashMap<String, Vec<CandidateVersion>>,
    loader: LoaderKind,
    exclusions: &'a graph::ExclusionMap,
    target: Environment,
}

fn collect_report(
    context: &ReportContext<'_>,
    solution: &pubgrub::SelectedDependencies<SolverPackage, SolverVersion>,
    trace: &diagnostics::ResolutionSnapshot,
) -> ResolutionReport {
    let mut warnings = resolution_warnings(
        context.lockfile,
        context.candidates,
        solution,
        context.loader,
        context.exclusions,
        context.target,
    );

    let mut selected_versions = BTreeMap::new();
    let mut selected_sources = BTreeMap::new();
    let mut selected_candidates = BTreeMap::new();
    let mut selected_identities = BTreeMap::new();
    for (package, version) in solution.iter() {
        let SolverPackage::Mod(mod_id) = package else {
            continue;
        };
        let Some(identity) = version
            .candidate_identity()
            .filter(|identity| identity.path.is_empty() && identity.owner == *mod_id)
        else {
            continue;
        };
        let Some(semantic) = version.domain() else {
            continue;
        };
        selected_versions.insert(mod_id.clone(), semantic.to_string());
        selected_sources.insert(mod_id.clone(), identity.source.clone());
        selected_identities.insert(mod_id.clone(), identity.clone());
        if context.candidates.get(mod_id).is_some_and(|candidates| {
            candidates
                .iter()
                .any(|candidate| candidate.id == identity.source)
        }) {
            selected_candidates.insert(mod_id.clone(), identity.source.clone());
        }
    }
    for (package, version) in &selected_versions {
        let analysis = Version::parse(version, context.loader).numeric_analysis();
        if let Some(reason) = analysis.reason() {
            warnings.push(format!(
                "{package} {version} cannot be numeric-filtered: {reason}; the numeric version constraint was not applied and the string rule used the full raw version text"
            ));
        }
    }
    warnings.sort();
    warnings.dedup();

    let mut changes = Vec::new();
    let mut retained_lock_entries = std::collections::HashSet::new();
    for (package, selected_version) in &selected_versions {
        let identity = &selected_identities[package];
        let installed: Vec<_> = context
            .lockfile
            .packages
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.mod_id == *package)
            .collect();
        if installed.is_empty() {
            changes.push(PackageChange {
                package: package.clone(),
                current_version: None,
                selected_version: Some(selected_version.clone()),
                filename: None,
                selected_filename: selected_candidate_filename(
                    context.candidates,
                    package,
                    &identity.source,
                ),
                selected_description: selected_candidate_description(
                    context.candidates,
                    package,
                    &identity.source,
                ),
                kind: PackageChangeKind::Install,
            });
            continue;
        }

        if identity.installed
            && let Some((selected_index, _)) = installed
                .iter()
                .copied()
                .find(|(_, entry)| locked_source(entry) == identity.source)
        {
            retained_lock_entries.insert(selected_index);
            for (index, entry) in installed {
                if index != selected_index {
                    changes.push(removal_change(entry));
                }
            }
            continue;
        }

        let (active_index, active) = installed
            .iter()
            .copied()
            .max_by(|(_, left), (_, right)| {
                let left = Version::parse(&left.version, context.loader);
                let right = Version::parse(&right.version, context.loader);
                left.cmp_precedence(&right).then_with(|| left.cmp(&right))
            })
            .expect("installed is not empty");
        let current = Version::parse(&active.version, context.loader);
        let selected = Version::parse(selected_version, context.loader);
        let kind = match selected.cmp_precedence(&current) {
            std::cmp::Ordering::Greater => PackageChangeKind::Upgrade,
            std::cmp::Ordering::Less => PackageChangeKind::Downgrade,
            std::cmp::Ordering::Equal => PackageChangeKind::Replace,
        };
        changes.push(PackageChange {
            package: package.clone(),
            current_version: Some(active.version.clone()),
            selected_version: Some(selected_version.clone()),
            filename: (!active.filename.is_empty()).then(|| active.filename.clone()),
            selected_filename: selected_candidate_filename(
                context.candidates,
                package,
                &identity.source,
            ),
            selected_description: selected_candidate_description(
                context.candidates,
                package,
                &identity.source,
            ),
            kind,
        });
        for (index, entry) in installed {
            if index != active_index {
                changes.push(removal_change(entry));
            }
        }
    }
    for (index, entry) in context.lockfile.packages.iter().enumerate() {
        if !selected_versions.contains_key(&entry.mod_id) && !retained_lock_entries.contains(&index)
        {
            changes.push(removal_change(entry));
        }
    }
    changes.sort_by(|left, right| {
        left.package
            .cmp(&right.package)
            .then_with(|| left.current_version.cmp(&right.current_version))
            .then_with(|| left.filename.cmp(&right.filename))
    });

    let mut diagnostics = Vec::new();
    let mut packages: Vec<_> = context.candidates.keys().collect();
    packages.sort();
    for package in packages {
        let Some(selected) = solution.get(&SolverPackage::logical(package)) else {
            continue;
        };
        let selected_semantic = selected.domain();
        let Some(candidate) = highest_candidate(
            context.candidates.get(package).map(Vec::as_slice),
            context.loader,
        ) else {
            continue;
        };
        if selected_semantic.is_some_and(|selected| {
            Version::parse(&candidate.jar_version, context.loader).cmp_precedence(selected)
                == std::cmp::Ordering::Greater
        }) {
            diagnostics.push(trace.diagnose_skipped(package, selected));
        }
    }
    ResolutionReport {
        selected_versions,
        selected_sources,
        selected_candidates,
        changes,
        diagnostics,
        warnings,
    }
}

fn removal_change(entry: &PackageEntry) -> PackageChange {
    PackageChange {
        package: entry.mod_id.clone(),
        current_version: Some(entry.version.clone()),
        selected_version: None,
        filename: (!entry.filename.is_empty()).then(|| entry.filename.clone()),
        selected_filename: None,
        selected_description: None,
        kind: PackageChangeKind::Remove,
    }
}

fn selected_candidate_filename(
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    package: &str,
    source: &str,
) -> Option<String> {
    candidates.get(package).and_then(|versions| {
        versions
            .iter()
            .find(|candidate| candidate.id == source)
            .map(|candidate| candidate.filename.clone())
            .filter(|filename| !filename.is_empty())
    })
}

fn selected_candidate_description(
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    package: &str,
    source: &str,
) -> Option<String> {
    candidates.get(package).and_then(|versions| {
        versions
            .iter()
            .find(|candidate| candidate.id == source)
            .map(CandidateVersion::display_description)
    })
}

fn highest_candidates(
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: LoaderKind,
) -> impl Iterator<Item = (String, SolverVersion)> {
    let mut watched: Vec<_> = candidates
        .iter()
        .filter_map(|(package, versions)| {
            highest_candidate(Some(versions.as_slice()), loader).map(|candidate| {
                (
                    package.clone(),
                    SolverVersion::candidate(
                        Version::parse(&candidate.jar_version, loader),
                        CandidateIdentity {
                            owner: package.clone(),
                            source: candidate.id.clone(),
                            path: Vec::new(),
                            location: CandidateLocation::Root,
                            installed: false,
                        },
                    ),
                )
            })
        })
        .collect();
    watched.sort_by(|(left, _), (right, _)| left.cmp(right));
    watched.into_iter()
}

fn highest_candidate(
    candidates: Option<&[CandidateVersion]>,
    loader: LoaderKind,
) -> Option<&CandidateVersion> {
    candidates?.iter().max_by(|left, right| {
        let left = Version::parse(&left.jar_version, loader);
        let right = Version::parse(&right.jar_version, loader);
        left.cmp_precedence(&right).then_with(|| left.cmp(&right))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jar::JarModOrigin;
    use crate::lockfile::{BundledMod, LockMeta, PackageEntry};
    use crate::metadata::{Environment, ModDependency, ModLoadCondition};
    use crate::progress::{ProgressEvent, ProgressReporter, ResolutionActivity};
    use std::sync::{Arc, Mutex};

    fn manifest() -> OrbitManifest {
        toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "forge"
modloader_version = "47.2.0"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"
[packages]
a = { version = "*", remotes = [{ type = "file", path = "a.jar" }] }
b = { version = "*", remotes = [{ type = "file", path = "b.jar" }] }
"#,
        )
        .unwrap()
    }

    fn locked(package: &str) -> PackageEntry {
        PackageEntry {
            mod_id: package.to_string(),
            version: "1".to_string(),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            filename: format!("{package}.jar"),
            remotes: vec![crate::manifest::PackageRemote::File {
                path: format!("{package}.jar"),
            }],
            artifact_sources: vec![crate::lockfile::ArtifactSource::File {
                path: format!("{package}.jar"),
            }],
            dependencies: Vec::new(),
            environment: Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        }
    }

    fn candidate(version: &str, dependencies: Vec<ModDependency>) -> CandidateVersion {
        CandidateVersion {
            id: format!("candidate-{version}"),
            filename: format!("candidate-{version}.jar"),
            display_sources: vec!["test candidate".to_string()],
            jar_version: version.to_string(),
            dependencies: dependencies.into_iter().map(Into::into).collect(),
            environment: Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        }
    }

    fn lockfile() -> OrbitLockfile {
        OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![locked("a"), locked("b")],
        }
    }

    fn upgrades(report: &ResolutionReport) -> BTreeMap<String, String> {
        report
            .changes
            .iter()
            .filter(|change| change.kind == PackageChangeKind::Upgrade)
            .map(|change| {
                (
                    change.package.clone(),
                    change.selected_version.clone().unwrap(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn selected_opaque_version_warns_that_numeric_filtering_was_bypassed() {
        let mut manifest = manifest();
        manifest.project.modloader = "fabric".to_string();
        manifest.project.modloader_version = "0.16.0".to_string();
        manifest.packages.shift_remove("b");
        manifest.packages["a"].version = "=999".to_string();
        manifest.packages["a"].string = "all".to_string();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16.0".to_string(),
            },
            packages: Vec::new(),
        };
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![candidate("release-vNext", Vec::new())],
        );

        let portfolio = resolve_candidate_portfolio(&manifest, &lockfile, &catalog)
            .await
            .unwrap();
        let report = &portfolio.alternatives[0];
        assert_eq!(report.selected_versions["a"], "release-vNext");
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("cannot be numeric-filtered"))
        );
    }

    #[tokio::test]
    async fn package_preserving_portfolio_drops_only_unavailable_manifest_packages() {
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate("2", Vec::new())]);
        let empty_lock = OrbitLockfile {
            meta: lockfile().meta,
            packages: Vec::new(),
        };

        let strict = resolve_candidate_portfolio(&manifest(), &empty_lock, &catalog).await;
        assert!(strict.is_err());

        let portfolio = resolve_package_preserving_portfolio_with_progress(
            &manifest(),
            &empty_lock,
            &catalog,
            None,
        )
        .await
        .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        assert_eq!(portfolio.alternatives[0].selected_versions["a"], "2");
        assert!(
            !portfolio.alternatives[0]
                .selected_versions
                .contains_key("b")
        );
    }

    #[tokio::test]
    async fn package_preserving_portfolio_returns_incomparable_minimal_removal_sets() {
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![candidate("1", vec![ModDependency::required("c", "[1]")])],
        );
        catalog.candidates.insert(
            "b".to_string(),
            vec![candidate("1", vec![ModDependency::required("c", "[2]")])],
        );
        catalog.candidates.insert(
            "c".to_string(),
            vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
        );
        let empty_lock = OrbitLockfile {
            meta: lockfile().meta,
            packages: Vec::new(),
        };

        let portfolio = resolve_package_preserving_portfolio_with_progress(
            &manifest(),
            &empty_lock,
            &catalog,
            None,
        )
        .await
        .unwrap();
        let retained = portfolio
            .alternatives
            .iter()
            .map(|alternative| {
                ["a", "b"]
                    .into_iter()
                    .filter(|package| alternative.selected_versions.contains_key(*package))
                    .collect::<Vec<_>>()
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            retained,
            std::collections::BTreeSet::from([vec!["a"], vec!["b"]])
        );
    }

    #[tokio::test]
    async fn package_preserving_portfolio_never_relaxes_manifest_version_constraints() {
        let mut constrained = manifest();
        constrained.packages["a"].version = "[3]".to_string();
        constrained.packages.shift_remove("b");
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate("2", Vec::new())]);
        let empty_lock = OrbitLockfile {
            meta: lockfile().meta,
            packages: Vec::new(),
        };

        let portfolio = resolve_package_preserving_portfolio_with_progress(
            &constrained,
            &empty_lock,
            &catalog,
            None,
        )
        .await
        .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        assert!(portfolio.alternatives[0].selected_versions.is_empty());
    }

    #[tokio::test]
    async fn candidate_portfolio_contains_every_upgrade_tradeoff() {
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![
                candidate("1", Vec::new()),
                candidate("2", vec![ModDependency::required("b", "[1]")]),
            ],
        );
        catalog.candidates.insert(
            "b".to_string(),
            vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
        );

        let portfolio = resolve_candidate_portfolio(&manifest(), &lockfile(), &catalog)
            .await
            .unwrap();
        let upgrades: std::collections::BTreeSet<_> =
            portfolio.alternatives.iter().map(upgrades).collect();

        assert_eq!(upgrades.len(), 2);
        assert!(upgrades.contains(&BTreeMap::from([("a".to_string(), "2".to_string())])));
        assert!(upgrades.contains(&BTreeMap::from([("b".to_string(), "2".to_string())])));
        for alternative in &portfolio.alternatives {
            assert_eq!(alternative.diagnostics.len(), 1, "{alternative:?}");
            let diagnostic = &alternative.diagnostics[0];
            assert_ne!(
                diagnostic.kind,
                crate::resolver::types::CandidateDiagnosticKind::Unexplained,
                "{alternative:?}"
            );
            assert!(
                diagnostic
                    .facts
                    .iter()
                    .any(|fact| fact.contains('a') && fact.contains('b')),
                "{alternative:?}"
            );
        }
    }

    #[tokio::test]
    async fn targeted_upgrade_keeps_its_blocking_reason_instead_of_upgrading_another_package() {
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![candidate(
                "2",
                vec![ModDependency::required("unavailable-dependency", "=2")],
            )],
        );
        catalog.candidates.insert(
            "b".to_string(),
            vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
        );

        let portfolio = resolve_candidate_portfolio(&manifest(), &lockfile(), &catalog)
            .await
            .unwrap();
        let all_packages = select_upgrade_resolution(portfolio.clone(), None, None).unwrap();
        assert!(
            all_packages.changes.iter().any(|change| {
                change.package == "b" && change.kind == PackageChangeKind::Upgrade
            })
        );

        let targeted = select_upgrade_resolution(portfolio, Some("a"), None).unwrap();
        assert!(targeted.changes.is_empty(), "{targeted:?}");
        assert_eq!(targeted.diagnostics.len(), 1, "{targeted:?}");
        assert_eq!(targeted.diagnostics[0].package, "a");
        assert!(
            targeted.diagnostics[0]
                .facts
                .iter()
                .any(|fact| fact.contains("unavailable-dependency")),
            "{targeted:?}"
        );
    }

    #[tokio::test]
    async fn portfolio_reports_balanced_dynamic_solver_work() {
        let mut catalog = CandidateCatalog::default();
        for package in ["a", "b"] {
            catalog.candidates.insert(
                package.to_string(),
                vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
            );
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let progress: ProgressReporter = Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });

        let portfolio = resolve_candidate_portfolio_with_progress(
            &manifest(),
            &lockfile(),
            &catalog,
            Some(progress),
        )
        .await
        .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        let events = events.lock().unwrap();
        let started = events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::ResolutionWorkStarted { .. }))
            .count();
        let finished = events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::ResolutionWorkFinished { .. }))
            .count();
        assert!(started > 1, "{events:?}");
        assert_eq!(started, finished);
        assert!(events.iter().any(|event| matches!(
            event,
            ProgressEvent::ResolutionActivity {
                activity: ResolutionActivity::Decision { .. }
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ProgressEvent::ResolutionActivity {
                activity: ResolutionActivity::Solution
            }
        )));
    }

    #[tokio::test]
    async fn independent_upgrades_have_one_solution_and_do_not_prompt() {
        let mut catalog = CandidateCatalog::default();
        for package in ["a", "b"] {
            catalog.candidates.insert(
                package.to_string(),
                vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
            );
        }
        let portfolio = resolve_candidate_portfolio(&manifest(), &lockfile(), &catalog)
            .await
            .unwrap();
        assert_eq!(portfolio.alternatives.len(), 1);

        let selected = select_resolution(
            portfolio,
            Some(Box::new(|_| {
                panic!("unique solution must not invoke selector")
            })),
        )
        .unwrap();
        assert_eq!(
            upgrades(&selected),
            BTreeMap::from([
                ("a".to_string(), "2".to_string()),
                ("b".to_string(), "2".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn minimal_change_keeps_every_feasible_installed_realization() {
        let current = lockfile();
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate("2", Vec::new())]);
        catalog
            .candidates
            .insert("b".to_string(), vec![candidate("2", Vec::new())]);

        let portfolio = resolve_minimal_change_portfolio(&manifest(), &current, &catalog)
            .await
            .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        assert!(portfolio.alternatives[0].changes.is_empty());
        assert_eq!(portfolio.alternatives[0].selected_versions["a"], "1");
        assert_eq!(portfolio.alternatives[0].selected_versions["b"], "1");
    }

    #[tokio::test]
    async fn minimal_change_reports_balanced_preference_probe_progress() {
        let current = lockfile();
        let mut catalog = CandidateCatalog::default();
        for package in ["a", "b"] {
            catalog.candidates.insert(
                package.to_string(),
                vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
            );
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let progress: ProgressReporter = Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });

        resolve_minimal_change_portfolio_with_progress(
            &manifest(),
            &current,
            &catalog,
            Some(progress),
        )
        .await
        .unwrap();

        let events = events.lock().unwrap();
        let started = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    ProgressEvent::ResolutionWorkStarted {
                        work: crate::progress::ResolutionWork::PreferenceProbe { .. }
                    }
                )
            })
            .count();
        let finished = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    ProgressEvent::ResolutionWorkFinished {
                        work: crate::progress::ResolutionWork::PreferenceProbe { .. }
                    }
                )
            })
            .count();
        assert!(started > 0, "{events:?}");
        assert_eq!(started, finished, "{events:?}");
    }

    #[tokio::test]
    async fn minimal_change_returns_incomparable_package_change_sets() {
        let mut current = lockfile();
        current.packages[0].dependencies = vec![ModDependency::required("b", "=2").into()];
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate("2", Vec::new())]);
        catalog
            .candidates
            .insert("b".to_string(), vec![candidate("2", Vec::new())]);

        let portfolio = resolve_minimal_change_portfolio(&manifest(), &current, &catalog)
            .await
            .unwrap();
        let choices = portfolio
            .alternatives
            .iter()
            .map(|alternative| {
                (
                    alternative.selected_versions["a"].clone(),
                    alternative.selected_versions["b"].clone(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            choices,
            std::collections::BTreeSet::from([
                ("1".to_string(), "2".to_string()),
                ("2".to_string(), "1".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn minimal_change_avoids_an_unnecessary_new_dependency_package() {
        let current = lockfile();
        let mut requested_manifest = manifest();
        requested_manifest.packages.insert(
            "requested".to_string(),
            crate::manifest::PackageSpec::new(
                "*",
                vec![crate::manifest::PackageRemote::File {
                    path: "requested.jar".to_string(),
                }],
            ),
        );
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "requested".to_string(),
            vec![
                candidate("1", Vec::new()),
                candidate("2", vec![ModDependency::required("new-dependency", "=1")]),
            ],
        );
        catalog.candidates.insert(
            "new-dependency".to_string(),
            vec![candidate("1", Vec::new())],
        );

        let portfolio = resolve_minimal_change_portfolio(&requested_manifest, &current, &catalog)
            .await
            .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        assert_eq!(
            portfolio.alternatives[0].selected_versions["requested"],
            "1"
        );
        assert!(
            !portfolio.alternatives[0]
                .selected_versions
                .contains_key("new-dependency")
        );
    }

    #[tokio::test]
    async fn equal_precedence_suffixes_remain_distinct_pareto_solutions() {
        let mut manifest = manifest();
        manifest.packages.shift_remove("b");
        manifest.packages["a"].version = "=1.2.3".to_string();
        let mut lockfile = lockfile();
        lockfile.packages.retain(|entry| entry.mod_id == "a");
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![
                candidate("1.2.3-alpha", Vec::new()),
                candidate("1.2.3-beta", Vec::new()),
            ],
        );

        let portfolio = resolve_candidate_portfolio(&manifest, &lockfile, &catalog)
            .await
            .unwrap();
        let selected: std::collections::BTreeSet<_> = portfolio
            .alternatives
            .iter()
            .map(|alternative| alternative.selected_versions["a"].clone())
            .collect();

        assert_eq!(
            selected,
            std::collections::BTreeSet::from(
                ["1.2.3-alpha".to_string(), "1.2.3-beta".to_string(),]
            )
        );
        assert!(portfolio.alternatives.iter().all(|alternative| {
            alternative
                .changes
                .iter()
                .any(|change| change.kind == PackageChangeKind::Upgrade)
        }));
    }

    #[tokio::test]
    async fn explicit_suffix_constraint_selects_one_candidate_without_a_prompt() {
        let mut manifest = manifest();
        manifest.packages.shift_remove("b");
        manifest.packages["a"].version = "=1.2.3-alpha".to_string();
        let mut lockfile = lockfile();
        lockfile.packages.retain(|entry| entry.mod_id == "a");
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![
                candidate("1.2.3-alpha", Vec::new()),
                candidate("1.2.3-beta", Vec::new()),
            ],
        );

        let portfolio = resolve_candidate_portfolio(&manifest, &lockfile, &catalog)
            .await
            .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        assert_eq!(
            portfolio.alternatives[0].selected_versions["a"],
            "1.2.3-alpha"
        );
    }

    #[tokio::test]
    async fn equal_declared_versions_with_distinct_files_remain_distinct_solutions() {
        let mut manifest = manifest();
        manifest.packages.shift_remove("b");
        manifest.packages["a"].version = "=1.2.3-alpha".to_string();
        let mut lockfile = lockfile();
        lockfile.packages.retain(|entry| entry.mod_id == "a");
        let mut first = candidate("1.2.3-alpha", Vec::new());
        first.id = "first-file".to_string();
        first.filename = "first.jar".to_string();
        let mut second = candidate("1.2.3-alpha", Vec::new());
        second.id = "second-file".to_string();
        second.filename = "second.jar".to_string();
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![first, second]);

        let portfolio = resolve_candidate_portfolio(&manifest, &lockfile, &catalog)
            .await
            .unwrap();
        let selected: std::collections::BTreeSet<_> = portfolio
            .alternatives
            .iter()
            .map(|alternative| alternative.selected_candidates["a"].clone())
            .collect();

        assert_eq!(
            selected,
            std::collections::BTreeSet::from(
                ["first-file".to_string(), "second-file".to_string(),]
            )
        );
    }

    #[tokio::test]
    async fn changing_only_the_suffix_is_a_replacement_not_an_upgrade() {
        let mut manifest = manifest();
        manifest.packages.shift_remove("b");
        manifest.packages["a"].version = "=1.2.3".to_string();
        let mut lockfile = lockfile();
        lockfile.packages.retain(|entry| entry.mod_id == "a");
        lockfile.packages[0].version = "1.2.3-alpha".to_string();
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate("1.2.3-beta", Vec::new())]);

        let portfolio = resolve_candidate_portfolio(&manifest, &lockfile, &catalog)
            .await
            .unwrap();
        let replacement = portfolio
            .alternatives
            .iter()
            .flat_map(|alternative| &alternative.changes)
            .find(|change| change.package == "a")
            .unwrap();

        assert_eq!(replacement.kind, PackageChangeKind::Replace);
        assert!(
            portfolio
                .alternatives
                .iter()
                .all(|alternative| !alternative.has_upgrade())
        );
    }

    #[tokio::test]
    async fn coordinated_upgrade_dominates_the_lower_disconnected_solution() {
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![
                candidate("1", vec![ModDependency::required("b", "[1]")]),
                candidate("2", vec![ModDependency::required("b", "[2]")]),
            ],
        );
        catalog.candidates.insert(
            "b".to_string(),
            vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
        );

        let portfolio = resolve_candidate_portfolio(&manifest(), &lockfile(), &catalog)
            .await
            .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        assert_eq!(
            portfolio.alternatives[0].selected_versions["a"],
            "2".to_string()
        );
        assert_eq!(
            portfolio.alternatives[0].selected_versions["b"],
            "2".to_string()
        );
    }

    #[tokio::test]
    async fn an_upgrade_solution_may_include_a_dependency_downgrade() {
        let mut current = lockfile();
        current
            .packages
            .iter_mut()
            .find(|entry| entry.mod_id == "b")
            .unwrap()
            .version = "2".to_string();
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![
                candidate("1", Vec::new()),
                candidate("2", vec![ModDependency::required("b", "[1]")]),
            ],
        );
        catalog.candidates.insert(
            "b".to_string(),
            vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
        );

        let portfolio = resolve_candidate_portfolio(&manifest(), &current, &catalog)
            .await
            .unwrap();
        let alternative = portfolio
            .alternatives
            .iter()
            .find(|alternative| alternative.has_upgrade())
            .unwrap();

        assert!(
            alternative.changes.iter().any(|change| {
                change.package == "a" && change.kind == PackageChangeKind::Upgrade
            })
        );
        assert!(alternative.changes.iter().any(|change| {
            change.package == "b" && change.kind == PackageChangeKind::Downgrade
        }));
    }

    #[tokio::test]
    async fn add_can_select_an_older_existing_package_with_a_compatible_dependency_range() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "26.1.2"
modloader = "fabric"
modloader_version = "0.19.2"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"
[packages]
reeses-sodium-options = { version = "*", remotes = [{ type = "file", path = "reeses.jar" }] }
voxy = { version = "*", remotes = [{ type = "file", path = "voxy.jar" }] }
"#,
        )
        .unwrap();
        let mut installed_reeses = locked("reeses-sodium-options");
        installed_reeses.version = "2".to_string();
        installed_reeses.dependencies = vec![ModDependency::required("sodium", ">=0.9.1").into()];
        let mut installed_sodium = locked("sodium");
        installed_sodium.version = "0.9.1".to_string();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "26.1.2".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.19.2".to_string(),
            },
            packages: vec![installed_reeses, installed_sodium],
        };
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "reeses-sodium-options".to_string(),
            vec![
                candidate("1", vec![ModDependency::required("sodium", ">=0.8.7")]),
                candidate("2", vec![ModDependency::required("sodium", ">=0.9.1")]),
            ],
        );
        catalog.candidates.insert(
            "voxy".to_string(),
            vec![candidate(
                "0.2.16-beta",
                vec![ModDependency::required("sodium", "=0.8.9")],
            )],
        );
        catalog.candidates.insert(
            "sodium".to_string(),
            vec![
                candidate("0.8.9", Vec::new()),
                candidate("0.9.1", Vec::new()),
            ],
        );

        let portfolio = resolve_candidate_portfolio(&manifest, &lockfile, &catalog)
            .await
            .unwrap();

        assert!(portfolio.alternatives.iter().any(|solution| {
            solution.selected_versions["reeses-sodium-options"] == "1"
                && solution.selected_versions["voxy"] == "0.2.16-beta"
                && solution.selected_versions["sodium"] == "0.8.9"
        }));
    }

    #[tokio::test]
    async fn equal_version_candidates_include_the_feasible_installed_realization() {
        let mut current = lockfile();
        current.packages.retain(|entry| entry.mod_id == "a");
        let mut only_a = manifest();
        only_a.packages.shift_remove("b");
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate("1", Vec::new())]);

        let portfolio = resolve_candidate_portfolio(&only_a, &current, &catalog)
            .await
            .unwrap();

        assert!(
            portfolio
                .alternatives
                .iter()
                .all(|alternative| !alternative.has_upgrade())
        );
        assert_eq!(portfolio.alternatives.len(), 2);
        assert!(
            portfolio
                .alternatives
                .iter()
                .any(|alternative| alternative.changes.is_empty())
        );
        assert!(portfolio.alternatives.iter().any(|alternative| {
            alternative
                .changes
                .iter()
                .any(|change| change.package == "a" && change.kind == PackageChangeKind::Replace)
        }));
    }

    #[tokio::test]
    async fn duplicate_installed_package_versions_plan_removal_of_the_unselected_file() {
        let mut older = locked("a");
        older.filename = "a-1.jar".to_string();
        let mut newer = locked("a");
        newer.version = "2".to_string();
        newer.filename = "a-2.jar".to_string();
        let current = OrbitLockfile {
            meta: lockfile().meta,
            packages: vec![older, newer],
        };
        let mut only_a = manifest();
        only_a.packages.shift_remove("b");

        let portfolio =
            resolve_candidate_portfolio(&only_a, &current, &CandidateCatalog::default())
                .await
                .unwrap();

        assert_eq!(portfolio.alternatives.len(), 1);
        let selected = &portfolio.alternatives[0];
        assert_eq!(selected.selected_versions["a"], "2");
        assert!(selected.changes.iter().any(|change| {
            change.kind == PackageChangeKind::Remove
                && change.current_version.as_deref() == Some("1")
                && change.filename.as_deref() == Some("a-1.jar")
        }));
    }

    #[tokio::test]
    async fn jar_dependency_without_a_downloaded_project_is_no_solution() {
        let mut manifest = manifest();
        manifest.packages.shift_remove("b");
        let lockfile = OrbitLockfile {
            meta: lockfile().meta,
            packages: Vec::new(),
        };
        let mut catalog = CandidateCatalog::default();
        catalog.candidates.insert(
            "a".to_string(),
            vec![candidate(
                "1",
                vec![ModDependency::required("jar-only-id", "*")],
            )],
        );

        let error = resolve_candidate_portfolio(&manifest, &lockfile, &catalog)
            .await
            .unwrap_err();

        assert!(error.contains("jar-only-id"), "{error}");
    }

    #[test]
    fn fabric_wildcard_conflict_hides_internal_range_sentinels() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "26.1.2"
modloader = "fabric"
modloader_version = "0.19.2"

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"

[packages]
iris = { version = "*", remotes = [{ type = "file", path = "iris.jar" }] }
"#,
        )
        .unwrap();
        let mut iris = locked("iris");
        iris.version = "1.11.2+mc26.1.2".to_string();
        iris.dependencies = vec![ModDependency::required("sodium", "0.9.x").into()];
        let mut sodium = locked("sodium");
        sodium.version = "0.8.12+mc26.1.2".to_string();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "26.1.2".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.19.2".to_string(),
            },
            packages: vec![iris, sodium],
        };

        let error = check_lockfile_graph(&manifest, &lockfile).unwrap_err();

        assert!(error.contains("sodium 0.9.x"), "{error}");
        assert!(!error.contains("x-upper"), "{error}");
        assert!(!error.contains("<1.11.2"), "{error}");
    }

    #[test]
    fn multiple_solutions_use_the_selected_alternative() {
        let first = ResolutionReport {
            selected_versions: BTreeMap::from([("a".to_string(), "2".to_string())]),
            ..ResolutionReport::default()
        };
        let second = ResolutionReport {
            selected_versions: BTreeMap::from([("b".to_string(), "2".to_string())]),
            ..ResolutionReport::default()
        };
        let portfolio = ResolutionPortfolio {
            alternatives: vec![first, second.clone()],
        };

        let selected = select_resolution(
            portfolio,
            Some(Box::new(|alternatives| {
                assert_eq!(alternatives.len(), 2);
                Ok(1)
            })),
        )
        .unwrap();

        assert_eq!(selected.selected_versions, second.selected_versions);
    }

    #[test]
    fn invalid_solution_selection_is_rejected() {
        let portfolio = ResolutionPortfolio {
            alternatives: vec![ResolutionReport::default(), ResolutionReport::default()],
        };

        let error = select_resolution(portfolio, Some(Box::new(|_| Ok(2)))).unwrap_err();

        assert_eq!(
            error,
            "dependency solution selector returned invalid choice 3 for 2 alternatives"
        );
    }

    #[test]
    fn cancelled_solution_selection_is_propagated_without_defaulting() {
        let portfolio = ResolutionPortfolio {
            alternatives: vec![ResolutionReport::default(), ResolutionReport::default()],
        };

        let error = select_resolution(
            portfolio,
            Some(Box::new(|_| {
                Err("interaction cancelled by user".to_string())
            })),
        )
        .unwrap_err();

        assert_eq!(error, "interaction cancelled by user");
    }

    #[test]
    fn nested_identity_path_becomes_a_physical_archive_chain() {
        let mut entry = locked("wrapper");
        entry.bundled = vec![nested_mod(
            "outer.jar",
            vec![nested_mod("inner.jar", Vec::new())],
        )];

        assert_eq!(
            nested_archive_path(&entry, &[0, 0]).as_deref(),
            Some("outer.jar!/inner.jar")
        );
        assert!(nested_archive_path(&entry, &[1]).is_none());
    }

    #[test]
    fn runtime_load_selection_reuses_the_solver_selected_top_level_files() {
        let mut older = locked("a");
        older.filename = "a-1.jar".to_string();
        let mut newer = locked("a");
        newer.version = "2".to_string();
        newer.filename = "a-2.jar".to_string();
        let current = OrbitLockfile {
            meta: lockfile().meta,
            packages: vec![older, newer, locked("b")],
        };

        let selected =
            selected_runtime_load(&manifest(), &current, None, Environment::Both).unwrap();

        assert_eq!(
            selected.top_level_jars,
            std::collections::BTreeSet::from(["a-2.jar".to_string(), "b.jar".to_string()])
        );
        assert_eq!(
            selected.active_mod_ids,
            std::collections::BTreeSet::from(["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn runtime_load_selection_also_filters_loader_owned_nested_modules() {
        let loader = types::PlatformCandidate {
            mod_id: "forge".to_string(),
            version: "47.2.0".to_string(),
            dependencies: Vec::new(),
            environment: Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: vec![types::BundledCandidate {
                mod_id: "loader_child".to_string(),
                version: "1".to_string(),
                load_condition: ModLoadCondition::Always,
                origin: JarModOrigin::Nested {
                    path: "META-INF/jars/loader-child.jar".to_string(),
                    artifact: None,
                },
                environment: Environment::Both,
                dependencies: Vec::new(),
                provides: Vec::new(),
                language_loader: None,
                embedded_artifacts: Vec::new(),
                bundled: Vec::new(),
            }],
        };

        let selected =
            selected_runtime_load(&manifest(), &lockfile(), Some(&loader), Environment::Both)
                .unwrap();

        assert!(selected.active_mod_ids.contains("loader_child"));
        assert_eq!(
            selected.loader_nested_jars,
            std::collections::BTreeSet::from(["META-INF/jars/loader-child.jar".to_string()])
        );
    }

    fn nested_mod(path: &str, bundled: Vec<BundledMod>) -> BundledMod {
        BundledMod {
            mod_id: path.to_string(),
            version: "1".to_string(),
            load_condition: ModLoadCondition::IfPossible,
            origin: JarModOrigin::Nested {
                path: path.to_string(),
                artifact: None,
            },
            environment: Environment::Both,
            dependencies: Vec::new(),
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled,
        }
    }
}
